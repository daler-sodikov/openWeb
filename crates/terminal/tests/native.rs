//! Real programs on real PTYs (CI on Linux, macOS and Windows): working
//! directory and typed input, resize, exit code, cleanup of child processes
//! and a flood of output. Built only with the `native` feature.
//!
//! On Unix each test starts its program directly as `/bin/sh -c <script>`:
//! no prompt, no job control and nothing from the user's shell or its
//! startup files, so the output does not depend on who runs the tests (on
//! Ubuntu `/bin/sh` is dash, elsewhere bash). Input still goes through the
//! PTY: the scripts `read` typed lines. The PTY echoes typed text when it is
//! written, so a program's output always starts on a line of its own. (An
//! interactive shell without a line editor, like dash, prints its first
//! prompt after the echo of text typed ahead, and the output then lands on
//! the prompt's line: that made these tests depend on timing.)
//!
//! On Windows the tests type into an interactive `cmd.exe`, which reads
//! typed-ahead input itself and echoes it after its prompt.

#![cfg(feature = "native")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_terminal::native::NativeBackend;
use ferrule_terminal::{
    EventSink, SpawnSpec, TermSize, Terminal, TerminalEvent, TerminalOptions, Terminals,
    terminal_env,
};

const SIZE: TermSize = TermSize { cols: 80, rows: 24 };

/// `/bin/sh -c unix_script` on Unix, an interactive `cmd.exe` on Windows
/// (the test then types its commands).
fn program(cwd: &Path, unix_script: &str) -> SpawnSpec {
    let (program, args) = if cfg!(windows) {
        (PathBuf::from("cmd.exe"), Vec::new())
    } else {
        (
            PathBuf::from("/bin/sh"),
            vec!["-c".to_owned(), unix_script.to_owned()],
        )
    };
    SpawnSpec {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: terminal_env(),
    }
}

/// Acknowledges every frame (like the view) and records exits.
struct Recorder {
    frames: AtomicUsize,
    exited: Mutex<Option<Option<i32>>>,
    terminal: Mutex<Option<Arc<Terminal>>>,
}

fn start(spec: &SpawnSpec) -> (Arc<Terminal>, Arc<Recorder>) {
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
            spec,
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

/// Types a line (Enter is CR, as a keyboard sends it).
fn type_line(t: &Terminal, text: &str) {
    t.input(&format!("{text}\r")).unwrap();
}

fn wait_exit(recorder: &Recorder) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(code) = *recorder.exited.lock().unwrap() {
            return code;
        }
        assert!(Instant::now() < deadline, "no exit event");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_program_starts_in_its_directory_and_reads_typed_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ferrule-marker-file"), "").unwrap();
    let (t, recorder) = start(&program(
        dir.path(),
        r#"IFS= read -r line; echo "got:$line"; ls"#,
    ));
    if cfg!(windows) {
        type_line(&t, "echo got:hello-native");
        type_line(&t, "dir /b");
    } else {
        type_line(&t, "hello-native");
    }
    wait_line(&t, SIZE.rows, "with the typed text", |l| {
        l == "got:hello-native"
    });
    // The working directory is the given one: its file is listed.
    wait_line(&t, SIZE.rows, "with the marker file", |l| {
        l.ends_with("ferrule-marker-file")
    });
    assert!(recorder.frames.load(Ordering::Relaxed) > 0);
    t.close();
}

#[test]
fn resize_reaches_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let (t, _) = start(&program(dir.path(), "IFS= read -r _; stty size"));
    t.resize(TermSize::new(100, 30)).unwrap();
    if cfg!(windows) {
        type_line(&t, "mode con");
        wait_line(&t, 30, "with 100 columns", |l| {
            l.contains("Columns") && l.contains("100")
        });
    } else {
        // The program asks for its size only after the resize.
        type_line(&t, "");
        wait_line(&t, 30, "with the size", |l| l == "30 100");
    }
    t.close();
}

#[test]
fn the_exit_code_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (t, recorder) = start(&program(dir.path(), r#"IFS= read -r code; exit "$code""#));
    if cfg!(windows) {
        type_line(&t, "exit 3");
    } else {
        type_line(&t, "3");
    }
    assert_eq!(wait_exit(&recorder), Some(3));
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
    // A long-running child of the program (like a dev server started by a
    // script), which prints its pid. On Unix it shares the program's
    // process group (no job control without a terminal shell).
    let (t, _) = start(&program(dir.path(), r#"sleep 1000 & echo "pid=$!"; wait"#));
    if cfg!(windows) {
        type_line(
            &t,
            "powershell -NoProfile -Command \"$p = Start-Process ping -ArgumentList '-n','1000','127.0.0.1' -PassThru -NoNewWindow; 'pid=' + $p.Id; Wait-Process -Id $p.Id\"",
        );
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
    // cmd.exe loops are slow; fewer lines still flood the PTY.
    let lines = if cfg!(windows) { 20_000 } else { 200_000 };
    let started = Instant::now();
    let (t, recorder) = start(&program(
        dir.path(),
        &format!(
            r#"i=0; while [ "$i" -lt {lines} ]; do i=$((i+1)); echo "line $i"; done; IFS= read -r line; echo "got:$line""#
        ),
    ));
    if cfg!(windows) {
        type_line(&t, &format!("for /L %i in (1,1,{lines}) do @echo line %i"));
    }
    wait_line(&t, SIZE.rows, "with the last line", |l| {
        l == format!("line {lines}")
    });
    let frames = recorder.frames.load(Ordering::Relaxed);
    let elapsed = started.elapsed();
    // At most about one frame per 16 ms interval (plus a few for the
    // start), far fewer than the lines printed.
    let bound = usize::try_from(elapsed.as_millis() / 16).unwrap() + 20;
    assert!(
        frames <= bound,
        "{frames} frames in {elapsed:?} for {lines} lines"
    );
    // Still responsive: typed input reaches the program.
    if cfg!(windows) {
        type_line(&t, "echo got:after-flood");
    } else {
        type_line(&t, "after-flood");
    }
    wait_line(&t, SIZE.rows, "after the flood", |l| l == "got:after-flood");
    t.close();
}

#[test]
fn close_all_and_wait_returns_after_the_programs_ended() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let terminals = Terminals::new(
        Some(Arc::new(NativeBackend)),
        TerminalOptions::default(),
        Arc::new(|_| {}),
    );
    let script = format!(r#"sleep 1000 & echo "$!" > '{}'; wait"#, pidfile.display());
    if cfg!(windows) {
        // The Windows path is covered by closing_ends_the_running_program.
        return;
    }
    terminals.open(&program(dir.path(), &script), SIZE).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "no pid written");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(alive(pid));
    let left = terminals.close_all_and_wait(Duration::from_secs(10));
    assert!(left.is_empty(), "terminals still running: {left:?}");
    assert!(!alive(pid), "the background child survived");
}
