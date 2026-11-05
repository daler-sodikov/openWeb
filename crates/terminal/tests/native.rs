//! Real shells on real PTYs (CI on Linux, macOS and Windows): start, echo,
//! resize, exit code, cleanup of child processes and a flood of output.
//! Built only with the `native` feature.

#![cfg(feature = "native")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_terminal::native::NativeBackend;
use ferrule_terminal::{
    EventSink, SpawnSpec, TermSize, Terminal, TerminalEvent, TerminalOptions, terminal_env,
};

const SIZE: TermSize = TermSize { cols: 80, rows: 24 };

/// `/bin/sh` on Unix, `cmd.exe` on Windows: present on every CI runner.
fn shell(cwd: PathBuf) -> SpawnSpec {
    let program = if cfg!(windows) {
        PathBuf::from("cmd.exe")
    } else {
        PathBuf::from("/bin/sh")
    };
    SpawnSpec {
        program,
        args: Vec::new(),
        cwd,
        env: terminal_env(),
    }
}

/// Acknowledges every frame (like the view) and records exits.
struct Recorder {
    frames: AtomicUsize,
    exited: Mutex<Option<Option<i32>>>,
    terminal: Mutex<Option<Arc<Terminal>>>,
}

fn start(cwd: PathBuf) -> (Arc<Terminal>, Arc<Recorder>) {
    let recorder = Arc::new(Recorder {
        frames: AtomicUsize::new(0),
        exited: Mutex::new(None),
        terminal: Mutex::new(None),
    });
    let r = recorder.clone();
    let sink: EventSink = Arc::new(move |event| match event {
        TerminalEvent::Frame(frame) => {
            r.frames.fetch_add(1, Ordering::Relaxed);
            if let Some(t) = r.terminal.lock().unwrap().as_ref() {
                t.ack(frame.seq);
            }
        }
        TerminalEvent::Exited { code, .. } => *r.exited.lock().unwrap() = Some(code),
    });
    let terminal = Arc::new(
        Terminal::start(
            1,
            &NativeBackend,
            &shell(cwd),
            SIZE,
            &TerminalOptions::default(),
            sink,
        )
        .unwrap(),
    );
    *recorder.terminal.lock().unwrap() = Some(terminal.clone());
    (terminal, recorder)
}

fn screen(t: &Terminal, rows: u16) -> String {
    (0..rows)
        .map(|row| t.row_text(row))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Waits until a line on screen satisfies `pred`; returns that line.
fn wait_line(t: &Terminal, rows: u16, what: &str, pred: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(line) = (0..rows)
            .map(|r| t.row_text(r))
            .find(|l| pred(l.trim_end()))
        {
            return line.trim_end().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "no line {what} on screen:\n{}",
            screen(t, rows)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run(t: &Terminal, command: &str) {
    t.input(&format!("{command}\r")).unwrap();
}

#[test]
fn a_shell_starts_in_its_directory_and_echoes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ferrule-marker-file"), "").unwrap();
    let (t, recorder) = start(dir.path().to_path_buf());
    run(&t, "echo hello-native");
    wait_line(&t, SIZE.rows, "with the echo", |l| l == "hello-native");
    // The working directory is the given one: its file is listed.
    run(&t, if cfg!(windows) { "dir /b" } else { "ls" });
    wait_line(&t, SIZE.rows, "with the marker file", |l| {
        l.ends_with("ferrule-marker-file")
    });
    assert!(recorder.frames.load(Ordering::Relaxed) > 0);
    t.close();
}

#[test]
fn resize_reaches_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let (t, _) = start(dir.path().to_path_buf());
    t.resize(TermSize::new(100, 30)).unwrap();
    if cfg!(windows) {
        run(&t, "mode con");
        wait_line(&t, 30, "with 100 columns", |l| {
            l.contains("Columns") && l.contains("100")
        });
    } else {
        run(&t, "stty size");
        wait_line(&t, 30, "with the size", |l| l == "30 100");
    }
    t.close();
}

#[test]
fn the_exit_code_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (t, recorder) = start(dir.path().to_path_buf());
    run(&t, "exit 3");
    let deadline = Instant::now() + Duration::from_secs(20);
    while recorder.exited.lock().unwrap().is_none() {
        assert!(Instant::now() < deadline, "no exit event");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(*recorder.exited.lock().unwrap(), Some(Some(3)));
    assert_eq!(t.exited(), Some(Some(3)));
}

/// Whether process `pid` still runs.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let pid = libc::pid_t::try_from(pid).unwrap();
    // SAFETY: kill with signal 0 only checks that the process exists; it
    // sends nothing and touches no memory.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[test]
fn closing_ends_the_running_program() {
    let dir = tempfile::tempdir().unwrap();
    let (t, _) = start(dir.path().to_path_buf());
    // A long-running program in the foreground (like a dev server) that
    // prints its pid first.
    if cfg!(windows) {
        run(
            &t,
            "powershell -NoProfile -Command \"$p = Start-Process ping -ArgumentList '-n','1000','127.0.0.1' -PassThru -NoNewWindow; 'pid=' + $p.Id; Wait-Process -Id $p.Id\"",
        );
    } else {
        run(&t, "sh -c 'echo pid=$$; exec sleep 1000'");
    }
    let line = wait_line(&t, SIZE.rows, "with the pid", |l| {
        l.strip_prefix("pid=")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    });
    let pid: u32 = line.trim_start_matches("pid=").parse().unwrap();
    assert!(alive(pid));
    t.close();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} survived closing the terminal"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_flood_is_batched_into_few_frames() {
    let dir = tempfile::tempdir().unwrap();
    let (t, recorder) = start(dir.path().to_path_buf());
    // cmd.exe loops are slow; fewer lines still flood the PTY.
    let lines = if cfg!(windows) { 20_000 } else { 200_000 };
    let started = Instant::now();
    if cfg!(windows) {
        run(&t, &format!("for /L %i in (1,1,{lines}) do @echo line %i"));
    } else {
        run(
            &t,
            &format!("i=0; while [ $i -lt {lines} ]; do i=$((i+1)); echo line $i; done"),
        );
    }
    wait_line(&t, SIZE.rows, "with the last line", |l| {
        l == format!("line {lines}")
    });
    let frames = recorder.frames.load(Ordering::Relaxed);
    let elapsed = started.elapsed();
    // At most about one frame per 16 ms interval (plus a few for the
    // prompt), far fewer than the lines printed.
    let bound = usize::try_from(elapsed.as_millis() / 16).unwrap() + 20;
    assert!(
        frames <= bound,
        "{frames} frames in {elapsed:?} for {lines} lines"
    );
    // Still responsive.
    run(&t, "echo after-flood");
    wait_line(&t, SIZE.rows, "after the flood", |l| l == "after-flood");
    t.close();
}
