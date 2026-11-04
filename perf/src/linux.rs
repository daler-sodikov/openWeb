//! Linux/X11 measurements. Everything is observed from outside the app:
//! window and pixels through X11, input through XTest, CPU and memory
//! through `/proc` for the whole process tree (app + WebKit processes).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, ImageFormat, MapState, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::Args;

/// Kernel clock ticks per second for `/proc/<pid>/stat` (USER_HZ, 100 on Linux).
const TICKS_PER_SEC: f64 = 100.0;
/// "Ready" = process-tree CPU below this share of one core for SETTLE_FOR.
const SETTLED_CPU_PERCENT: f64 = 5.0;
const SETTLE_FOR: Duration = Duration::from_secs(1);
const IDLE_SAMPLE: Duration = Duration::from_secs(10);
/// Pixels that must differ from every baseline frame to count as "typed".
const CHANGED_PIXELS: usize = 12;

// Window-relative layout of the Ferrule window (toolbar 37 px, tabs 30 px,
// sidebar 260 px). Used only to pick screen regions to watch.
const SIDEBAR: Rect = Rect {
    x: 10,
    y: 70,
    w: 230,
    h: 150,
};
const EDITOR_FIRST_LINE: Rect = Rect {
    x: 300,
    y: 68,
    w: 900,
    h: 24,
};
const CLICK_LINE_END: (i16, i16) = (1150, 80);

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i16,
    y: i16,
    w: u16,
    h: u16,
}

struct X11 {
    conn: RustConnection,
    root: Window,
    net_wm_pid: u32,
    wm_protocols: u32,
    wm_delete: u32,
    key_x: u8,
}

impl X11 {
    fn connect() -> anyhow::Result<Self> {
        let (conn, screen) =
            x11rb::connect(None).context("cannot connect to the X server (is DISPLAY set?)")?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &[u8]| -> anyhow::Result<u32> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        let net_wm_pid = atom(b"_NET_WM_PID")?;
        let wm_protocols = atom(b"WM_PROTOCOLS")?;
        let wm_delete = atom(b"WM_DELETE_WINDOW")?;
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let mapping = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per = usize::from(mapping.keysyms_per_keycode);
        let index = mapping
            .keysyms
            .chunks(per.max(1))
            .position(|syms| syms.first() == Some(&0x78)) // keysym "x"
            .context("no keycode for 'x'")?;
        let key_x = min + u8::try_from(index)?;
        Ok(Self {
            conn,
            root,
            net_wm_pid,
            wm_protocols,
            wm_delete,
            key_x,
        })
    }

    /// The viewable top-level window of `pid` (by `_NET_WM_PID`).
    fn find_window(&self, pid: u32) -> Option<Window> {
        let mut stack = vec![(self.root, 0)];
        while let Some((window, depth)) = stack.pop() {
            let tree = self.conn.query_tree(window).ok()?.reply().ok()?;
            for child in tree.children {
                let owner = self
                    .conn
                    .get_property(false, child, self.net_wm_pid, AtomEnum::CARDINAL, 0, 1)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .and_then(|r| r.value32().and_then(|mut v| v.next()));
                if owner == Some(pid) {
                    let viewable = self
                        .conn
                        .get_window_attributes(child)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .is_some_and(|a| a.map_state == MapState::VIEWABLE);
                    let wide = self
                        .conn
                        .get_geometry(child)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .is_some_and(|g| g.width > 400);
                    if viewable && wide {
                        return Some(child);
                    }
                }
                if depth < 3 {
                    stack.push((child, depth + 1));
                }
            }
        }
        None
    }

    fn image(&self, window: Window, r: Rect) -> Option<Vec<u8>> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, window, r.x, r.y, r.w, r.h, !0)
            .ok()?
            .reply()
            .ok()?;
        Some(reply.data)
    }

    /// Whether a region shows more than one flat color (text was drawn).
    fn has_content(&self, window: Window, r: Rect) -> bool {
        let Some(data) = self.image(window, r) else {
            return false;
        };
        let mut counts: HashMap<&[u8], usize> = HashMap::new();
        for px in data.as_chunks::<4>().0 {
            *counts.entry(&px[..3]).or_default() += 1;
        }
        let total = data.len() / 4;
        let majority = counts.values().copied().max().unwrap_or(0);
        total - majority > 50
    }

    fn sync(&self) -> anyhow::Result<()> {
        self.conn.flush()?;
        // A round trip makes sure the server processed the requests.
        self.conn.get_input_focus()?.reply()?;
        Ok(())
    }

    fn click(&self, window: Window, (x, y): (i16, i16)) -> anyhow::Result<()> {
        let at = self
            .conn
            .translate_coordinates(window, self.root, x, y)?
            .reply()?;
        let (rx, ry) = (at.dst_x, at.dst_y);
        self.conn.xtest_fake_input(6, 0, 0, self.root, rx, ry, 0)?; // motion
        self.conn.xtest_fake_input(4, 1, 0, self.root, rx, ry, 0)?; // press
        self.conn.xtest_fake_input(5, 1, 0, self.root, rx, ry, 0)?; // release
        self.sync()
    }

    fn type_x(&self) -> anyhow::Result<()> {
        self.conn
            .xtest_fake_input(2, self.key_x, 0, self.root, 0, 0, 0)?;
        self.conn
            .xtest_fake_input(3, self.key_x, 0, self.root, 0, 0, 0)?;
        self.sync()
    }

    fn close(&self, window: Window) -> anyhow::Result<()> {
        let event =
            ClientMessageEvent::new(32, window, self.wm_protocols, [self.wm_delete, 0, 0, 0, 0]);
        self.conn
            .send_event(false, window, EventMask::NO_EVENT, event)?;
        self.sync()
    }
}

/// The process and all its descendants.
fn process_tree(root: u32) -> Vec<u32> {
    let mut parents: Vec<(u32, u32)> = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if let Some(ppid) = stat_fields(pid).and_then(|f| f.get(1).and_then(|p| p.parse().ok()))
            {
                parents.push((pid, ppid));
            }
        }
    }
    let mut tree = vec![root];
    let mut seen: HashSet<u32> = HashSet::from([root]);
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        for (pid, ppid) in &parents {
            if *ppid == parent && seen.insert(*pid) {
                tree.push(*pid);
            }
        }
        i += 1;
    }
    tree
}

/// Fields of `/proc/<pid>/stat` after the command name (field 3 onwards).
fn stat_fields(pid: u32) -> Option<Vec<String>> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rsplit_once(')')?.1;
    Some(after.split_whitespace().map(str::to_owned).collect())
}

/// Total CPU ticks (user + system) of the process tree.
fn cpu_ticks(root: u32) -> u64 {
    process_tree(root)
        .into_iter()
        .filter_map(stat_fields)
        .map(|f| {
            let get = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            get(11) + get(12) // utime, stime (fields 14 and 15)
        })
        .sum()
}

/// CPU use of the tree over `window`, in percent of one core.
fn cpu_percent(root: u32, window: Duration) -> f64 {
    let before = cpu_ticks(root);
    std::thread::sleep(window);
    let after = cpu_ticks(root);
    (after.saturating_sub(before) as f64 / TICKS_PER_SEC) / window.as_secs_f64() * 100.0
}

/// (PSS, RSS) of the process tree in MiB.
fn memory_mib(root: u32) -> (f64, f64) {
    let mut pss = 0u64;
    let mut rss = 0u64;
    for pid in process_tree(root) {
        let Ok(text) = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) else {
            continue;
        };
        for line in text.lines() {
            let value = |l: &str| -> u64 {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            };
            if line.starts_with("Pss:") {
                pss += value(line);
            } else if line.starts_with("Rss:") {
                rss += value(line);
            }
        }
    }
    (pss as f64 / 1024.0, rss as f64 / 1024.0)
}

fn wait_for<T>(timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> anyhow::Result<T> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(value) = probe() {
            return Ok(value);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    bail!("timed out after {timeout:?}")
}

#[derive(Debug, Default)]
struct Run {
    window_ms: f64,
    ready_ms: f64,
    pss_mib: f64,
    rss_mib: f64,
    idle_cpu: f64,
    typing_ms: Vec<f64>,
    quit_ms: f64,
}

struct AppProcess(Child);

impl Drop for AppProcess {
    fn drop(&mut self) {
        // Never leave a benchmark app running if a measurement fails.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch(args: &Args, project: &Path, data: &Path, x: &X11, full: bool) -> anyhow::Result<Run> {
    let mut run = Run::default();
    let start = Instant::now();
    let mut app = AppProcess(
        Command::new(&args.app)
            .arg(project)
            .arg("main.ts")
            .env("XDG_DATA_HOME", data)
            .env("GDK_BACKEND", "x11")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("cannot start {}", args.app.display()))?,
    );
    let pid = app.0.id();
    let window = wait_for(Duration::from_secs(30), || x.find_window(pid)).context("no window")?;
    wait_for(Duration::from_secs(30), || {
        x.has_content(window, SIDEBAR).then_some(())
    })
    .context("the project tree never appeared")?;
    run.window_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Ready: CPU settled for SETTLE_FOR (sampled in 100 ms steps).
    let step = Duration::from_millis(100);
    let needed = (SETTLE_FOR.as_millis() / step.as_millis()) as usize;
    let mut calm = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while calm < needed && Instant::now() < deadline {
        if cpu_percent(pid, step) < SETTLED_CPU_PERCENT {
            calm += 1;
        } else {
            calm = 0;
        }
    }
    // The settled second itself is not part of the startup work.
    run.ready_ms = (start.elapsed() - SETTLE_FOR).as_secs_f64() * 1000.0;

    if full {
        (run.pss_mib, run.rss_mib) = memory_mib(pid);
        run.idle_cpu = cpu_percent(pid, IDLE_SAMPLE);
        run.typing_ms = typing_latency(x, window, args.keys)?;
    }

    let quit = Instant::now();
    x.close(window)?;
    wait_for(Duration::from_secs(10), || app.0.try_wait().ok().flatten())
        .context("the app did not quit")?;
    run.quit_ms = quit.elapsed().as_secs_f64() * 1000.0;
    Ok(run)
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p[..3] != q[..3])
        .count()
}

/// Key press (XTest) until the glyph is on screen, per keystroke. Baseline
/// frames cover both caret-blink states, so blinking is not mistaken for
/// the glyph.
fn typing_latency(x: &X11, window: Window, keys: usize) -> anyhow::Result<Vec<f64>> {
    wait_for(Duration::from_secs(20), || {
        x.has_content(window, EDITOR_FIRST_LINE).then_some(())
    })
    .context("the editor never showed main.ts")?;
    x.click(window, CLICK_LINE_END)?;
    std::thread::sleep(Duration::from_millis(500));
    let mut samples = Vec::with_capacity(keys);
    for _ in 0..keys {
        let mut baselines: Vec<Vec<u8>> = Vec::new();
        let until = Instant::now() + Duration::from_millis(1300);
        while Instant::now() < until {
            if let Some(frame) = x.image(window, EDITOR_FIRST_LINE)
                && !baselines.iter().any(|b| differing(b, &frame) == 0)
            {
                baselines.push(frame);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        let sent = Instant::now();
        x.type_x()?;
        let latency = wait_for(Duration::from_secs(1), || {
            let frame = x.image(window, EDITOR_FIRST_LINE)?;
            baselines
                .iter()
                .all(|b| differing(b, &frame) > CHANGED_PIXELS)
                .then(|| sent.elapsed())
        })
        .context("a keystroke never appeared on screen")?;
        samples.push(latency.as_secs_f64() * 1000.0);
    }
    Ok(samples)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn machine() -> String {
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        })
        .unwrap_or_default();
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    let mem = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines().next().map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .unwrap_or("0")
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    / 1024.0
                    / 1024.0
            })
        })
        .unwrap_or(0.0);
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let display = std::env::var("DISPLAY").unwrap_or_default();
    format!(
        "{cpu}, {cores} threads, {mem:.1} GiB RAM, Linux {}, DISPLAY={display}",
        kernel.trim()
    )
}

/// Runs the benchmark and prints a Markdown report to stdout.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let x = X11::connect()?;
    let dir = tempfile::tempdir()?;
    let project = dir.path().join("bench-project");
    let files = crate::generate::generate(&project, args.files)?;
    let data = dir.path().join("data");

    let cold = launch(args, &project, &data, &x, false)?;
    let warm = launch(args, &project, &data, &x, true)?;

    let mut typing = warm.typing_ms.clone();
    typing.sort_by(f64::total_cmp);
    let mut out = std::io::stdout().lock();
    writeln!(out, "Machine: {}", machine())?;
    writeln!(
        out,
        "Project: {files} generated files; app: {}",
        args.app.display()
    )?;
    writeln!(out)?;
    writeln!(out, "| Metric | Measured | Budget |")?;
    writeln!(out, "| --- | --- | --- |")?;
    writeln!(
        out,
        "| Window with project, cold (no index cache) | {:.0} ms | < 1 s |",
        cold.window_ms
    )?;
    writeln!(
        out,
        "| Window with project, warm | {:.0} ms | < 1 s |",
        warm.window_ms
    )?;
    writeln!(
        out,
        "| Ready (CPU settled), cold | {:.0} ms | < 2 s |",
        cold.ready_ms
    )?;
    writeln!(
        out,
        "| Ready (CPU settled), warm | {:.0} ms | < 2 s |",
        warm.ready_ms
    )?;
    writeln!(
        out,
        "| Memory, PSS of all processes | {:.0} MiB (RSS {:.0} MiB) | < 250 MB |",
        warm.pss_mib, warm.rss_mib
    )?;
    writeln!(
        out,
        "| Idle CPU (10 s average) | {:.1} % of a core | < 5 % |",
        warm.idle_cpu
    )?;
    writeln!(
        out,
        "| Typing latency ({} keys) | median {:.1} ms, p95 {:.1} ms, max {:.1} ms | < 8 ms |",
        typing.len(),
        percentile(&typing, 0.5),
        percentile(&typing, 0.95),
        typing.last().copied().unwrap_or(f64::NAN)
    )?;
    writeln!(out, "| Quit | {:.0} ms | < 0.5 s |", warm.quit_ms)?;
    Ok(())
}
