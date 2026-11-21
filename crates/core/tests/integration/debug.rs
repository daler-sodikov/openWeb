//! Debug sessions (M5b-5). Most tests use the scripted debugger and the
//! in-memory run backend: trust, the event sequence with `seq`, refused
//! inspector flags, a failed attach, and the cleanup of Stop and
//! `close_project`, also while paused. The `real_node_*` tests start Node
//! on a real PTY, attach over CDP, and check the process list; they skip
//! with a message when `node` is not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_core::fake_debug::FakeConnector;
use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    DebugEvent, DebugEventKind, DebugPauseReason, IpcErrorKind, RunConfig, RunKind, StartDebug,
    TerminalFrame, TerminalSize,
};
use ferrule_run_debug::fake::FakeRunBackend;
use ferrule_run_debug::fake_debug::{FakeDebug, ScriptItem};
use ferrule_run_debug::{DebugError, Frame, Location, PauseInfo, PauseReason};

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn debug(&self, session: u32) -> Vec<DebugEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Debug(d) if d.session == session => Some(d.clone()),
                _ => None,
            })
            .collect()
    }

    fn kinds(&self, session: u32) -> Vec<String> {
        self.debug(session)
            .iter()
            .map(|d| match &d.event {
                DebugEventKind::Started => "started".to_owned(),
                DebugEventKind::Resumed => "resumed".to_owned(),
                DebugEventKind::Ended { stopped, .. } => format!("ended(stopped={stopped})"),
                DebugEventKind::Paused { reason, frames, .. } => format!(
                    "paused({},{})",
                    match reason {
                        DebugPauseReason::Entry => "entry",
                        DebugPauseReason::Step => "step",
                        DebugPauseReason::Pause => "pause",
                        DebugPauseReason::Breakpoint => "breakpoint",
                        DebugPauseReason::Exception { .. } => "exception",
                    },
                    frames[0].location.line
                ),
            })
            .collect()
    }

    /// Whether the run's process has ended.
    fn run_finished(&self, run: u32) -> bool {
        self.0
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::RunFinished(f) if f.id == run))
    }

    /// Every row text the run's frames showed.
    fn output(&self, run: u32) -> String {
        let mut out = String::new();
        for event in self.0.lock().unwrap().iter() {
            if let SessionEvent::TerminalFrame(TerminalFrame { id, rows, .. }) = event
                && *id == run
            {
                for row in rows {
                    out.extend(row.runs.iter().map(|r| r.text.as_str()));
                    out.push('\n');
                }
            }
        }
        out
    }
}

const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 20,
};

fn stop(reason: PauseReason, line: u32) -> ScriptItem {
    ScriptItem::Stop(PauseInfo::new(
        reason,
        vec![Frame {
            id: format!("f{line}"),
            name: "main".to_owned(),
            location: Location {
                path_or_script: "main.js".to_owned(),
                line,
                column: 0,
                generated: None,
            },
            is_library: false,
            scopes: Vec::new(),
        }],
    ))
}

fn program() -> FakeDebug {
    FakeDebug::new(vec![
        stop(PauseReason::Entry, 0),
        ScriptItem::Output("hello".to_owned()),
        stop(PauseReason::Step, 3),
        stop(PauseReason::Step, 4),
        ScriptItem::Exit(Some(0)),
    ])
}

fn config(args: &[&str], env: &[(&str, &str)]) -> RunConfig {
    RunConfig {
        id: "adhoc:main".into(),
        name: "main".into(),
        kind: RunKind::Node {
            file: "main.js".into(),
        },
        cwd: String::new(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        package_manager: None,
    }
}

/// Node is only needed to build the command line; the fake backends never
/// start it.
fn node_available() -> bool {
    let found = ferrule_lsp_client::find_on_path("node").is_some();
    if !found {
        eprintln!("skipped: node is not on PATH");
    }
    found
}

/// The scripted debugger (or none) on in-memory runs.
async fn open(dir: &Path, debugger: bool) -> (Arc<Session>, Arc<Events>) {
    let connector = debugger
        .then(|| Arc::new(FakeConnector::new(program)) as Arc<dyn ferrule_core::DebugConnector>);
    open_with(
        dir,
        Arc::new(FakeRunBackend),
        connector,
        "console.log('hi');\n",
    )
    .await
}

/// A project holding `main.js` with `script`, opened with the given run
/// backend and debugger.
async fn open_with(
    dir: &Path,
    run_backend: Arc<dyn ferrule_terminal::Backend>,
    connector: Option<Arc<dyn ferrule_core::DebugConnector>>,
    script: &str,
) -> (Arc<Session>, Arc<Events>) {
    let root = dir.join("p");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.js"), script).unwrap();
    let events = Arc::new(Events::default());
    let mut config = SessionConfig::new(dir.join("data"));
    config.run_backend = Some(run_backend);
    config.debug_connector = connector;
    let s = Session::with_config(config, events.clone()).unwrap();
    s.open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    (s, events)
}

async fn started(s: &Session, config: RunConfig) -> u32 {
    match s.start_debug(config, SIZE).await.unwrap() {
        StartDebug::Started { session, .. } => session,
        other => panic!("expected a started session, got {other:?}"),
    }
}

/// Waits until the session's event kinds satisfy `ok`.
async fn wait_until(events: &Events, session: u32, what: &str, ok: impl Fn(&[String]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if ok(&events.kinds(session)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {what}; the events are {:?}",
            events.kinds(session)
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the session's events are exactly `kinds`.
async fn wait_for<S: AsRef<str>>(events: &Events, session: u32, kinds: &[S]) {
    let want: Vec<&str> = kinds.iter().map(AsRef::as_ref).collect();
    wait_until(events, session, &format!("{want:?}"), |k| {
        k.iter().map(String::as_str).eq(want.iter().copied())
    })
    .await;
}

#[tokio::test]
async fn a_session_needs_trust_then_plays_start_pause_step_end() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open(dir.path(), true).await;

    match s.start_debug(config(&[], &[]), SIZE).await.unwrap() {
        StartDebug::NeedsTrust { command, .. } => {
            assert!(command.contains("--inspect-brk=127.0.0.1:"), "{command}");
            assert!(!command.contains("0.0.0.0"), "{command}");
        }
        other => panic!("debugged without trust: {other:?}"),
    }
    assert!(s.runs().await.unwrap().is_empty(), "nothing may start");
    assert_eq!(s.debug_session_count().unwrap(), 0);

    s.trust_project().await.unwrap();
    let session = started(&s, config(&[], &[])).await;
    let mut seen = vec!["started", "paused(entry,0)"];
    wait_for(&events, session, &seen).await;

    assert_eq!(
        s.debug_pause(session).await.unwrap_err().kind,
        IpcErrorKind::Conflict,
        "pause is for a running program"
    );

    s.debug_resume(session).await.unwrap();
    seen.extend(["resumed", "paused(step,3)"]);
    wait_for(&events, session, &seen).await;

    s.debug_step_over(session).await.unwrap();
    seen.extend(["resumed", "paused(step,4)"]);
    wait_for(&events, session, &seen).await;

    s.debug_step_into(session).await.unwrap();
    seen.extend(["resumed", "ended(stopped=false)"]);
    wait_for(&events, session, &seen).await;

    let all = events.debug(session);
    let seqs: Vec<u32> = all.iter().map(|d| d.seq).collect();
    let expected: Vec<u32> = (1..=u32::try_from(all.len()).unwrap()).collect();
    assert_eq!(seqs, expected, "seq starts at 1 and grows by one");
    assert_eq!(
        all.last().unwrap().event,
        DebugEventKind::Ended {
            exit_code: Some(0),
            stopped: false,
            message: None
        }
    );

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
}

#[tokio::test]
async fn inspector_flags_in_a_configuration_are_refused_before_anything_starts() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, _events) = open(dir.path(), true).await;
    s.trust_project().await.unwrap();
    for bad in [
        config(&["--inspect"], &[]),
        config(&["--inspect-brk=0.0.0.0:9229"], &[]),
        config(&[], &[("NODE_OPTIONS", "--inspect-brk")]),
    ] {
        let err = s.start_debug(bad.clone(), SIZE).await.unwrap_err();
        assert_eq!(err.kind, IpcErrorKind::InvalidArgument, "{bad:?}: {err:?}");
    }
    assert!(s.runs().await.unwrap().is_empty(), "no run may start");
    assert_eq!(s.debug_session_count().unwrap(), 0);
    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
}

#[tokio::test]
async fn stop_while_paused_ends_the_session_and_leaves_nothing() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open(dir.path(), true).await;
    s.trust_project().await.unwrap();
    let session = started(&s, config(&[], &[])).await;
    wait_for(&events, session, &["started", "paused(entry,0)"]).await;

    s.stop_debug(session).await.unwrap();
    assert_eq!(
        events.kinds(session),
        ["started", "paused(entry,0)", "ended(stopped=true)"],
        "a paused program is stopped, never resumed first"
    );
    assert_eq!(s.debug_session_count().unwrap(), 0, "no task is left");
    assert_eq!(
        s.stop_debug(session).await.unwrap_err().kind,
        IpcErrorKind::NotFound
    );
    assert_eq!(
        s.debug_resume(session).await.unwrap_err().kind,
        IpcErrorKind::NotFound
    );
    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
}

#[tokio::test]
async fn closing_the_project_ends_a_paused_session_first() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open(dir.path(), true).await;
    s.trust_project().await.unwrap();
    let session = started(&s, config(&[], &[])).await;
    wait_for(&events, session, &["started", "paused(entry,0)"]).await;

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    assert_eq!(
        events.kinds(session),
        ["started", "paused(entry,0)", "ended(stopped=true)"],
        "Ended goes out before the runtime is dropped"
    );
}

/// A connector that cannot attach.
struct Failing;

impl ferrule_core::DebugConnector for Failing {
    fn connect(
        &self,
        _port: u16,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> ferrule_core::BoxFuture<'_, Result<ferrule_core::Connection, DebugError>> {
        Box::pin(async { Err(DebugError::Backend("nothing is listening".into())) })
    }
}

#[tokio::test]
async fn a_failed_attach_ends_the_session_with_the_reason() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open_with(
        dir.path(),
        Arc::new(FakeRunBackend),
        Some(Arc::new(Failing)),
        "console.log('hi');\n",
    )
    .await;
    s.trust_project().await.unwrap();
    let session = started(&s, config(&[], &[])).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = loop {
        if let Some(e) = events.debug(session).last().cloned() {
            break e;
        }
        assert!(Instant::now() < deadline, "no Ended after a failed attach");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let DebugEventKind::Ended {
        stopped, message, ..
    } = ended.event
    else {
        panic!("expected Ended, got {ended:?}");
    };
    assert!(!stopped);
    let message = message.expect("a failed attach must say why");
    assert!(message.contains("nothing is listening"), "{message}");
    assert_eq!(events.kinds(session), ["ended(stopped=false)"]);
    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
}

async fn wait_run_finished(events: &Events, run: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !events.run_finished(run) {
        assert!(
            Instant::now() < deadline,
            "run {run} never ended; output:\n{}",
            events.output(run)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_output(events: &Events, run: u32, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !events.output(run).contains(needle) {
        assert!(
            Instant::now() < deadline,
            "run {run} never printed {needle:?}; output:\n{}",
            events.output(run)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Spawns a child, prints both pids, then idles: a tree to kill.
const TREE: &str = "const { spawn } = require(\"node:child_process\");\n\
const child = spawn(process.execPath, [\"-e\", \"setInterval(() => {}, 1000)\"], { stdio: \"ignore\" });\n\
console.log(`pids ${process.pid} ${child.pid}`);\n\
setInterval(() => {}, 1000);\n";

async fn open_real(dir: &Path, script: &str) -> (Arc<Session>, Arc<Events>) {
    open_with(
        dir,
        Arc::new(ferrule_terminal::native::NativeBackend),
        Some(Arc::new(ferrule_core::CdpConnector)),
        script,
    )
    .await
}

/// Whether a process still exists (a zombie does not count).
fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'),
            Err(_) => false,
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
    }
}

async fn wait_gone(pids: &[u32], what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left: Vec<u32> = pids.iter().copied().filter(|p| alive(*p)).collect();
        if left.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: processes {left:?} are still running"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The two pids the program printed.
async fn wait_pids(events: &Events, run: u32) -> [u32; 2] {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = events.output(run);
        let found = text.lines().find_map(|line| {
            let mut words = line.trim().strip_prefix("pids ")?.split_whitespace();
            Some([words.next()?.parse().ok()?, words.next()?.parse().ok()?])
        });
        if let Some(pids) = found {
            return pids;
        }
        assert!(
            Instant::now() < deadline,
            "the program never printed its pids; output:\n{text}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Starts [`TREE`] under the debugger on real Node and steps past the line
/// that prints the pids: paused on line 3, both processes alive.
async fn real_tree_paused(s: &Session, events: &Events) -> (u32, u32, [u32; 2]) {
    s.trust_project().await.unwrap();
    let (session, run) = match s.start_debug(config(&[], &[]), SIZE).await.unwrap() {
        StartDebug::Started { session, run } => (session, run.id),
        other => panic!("expected a started session, got {other:?}"),
    };
    let mut seen: Vec<String> = vec!["started".into(), "paused(entry,0)".into()];
    wait_for(events, session, &seen).await;
    for line in 1..=3 {
        s.debug_step_over(session).await.unwrap();
        seen.push("resumed".into());
        seen.push(format!("paused(step,{line})"));
        wait_for(events, session, &seen).await;
    }
    let pids = wait_pids(events, run).await;
    assert!(pids.iter().all(|p| alive(*p)), "{pids:?} should be running");
    (session, run, pids)
}

#[tokio::test]
async fn real_node_steps_resumes_pauses_and_stop_kills_the_whole_tree() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open_real(dir.path(), TREE).await;
    let (session, run, pids) = real_tree_paused(&s, &events).await;
    let mut seen = events.kinds(session);

    s.debug_resume(session).await.unwrap();
    seen.push("resumed".into());
    wait_for(&events, session, &seen).await;
    assert_eq!(
        s.debug_resume(session).await.unwrap_err().kind,
        IpcErrorKind::Conflict,
        "a running program cannot be resumed"
    );

    s.debug_pause(session).await.unwrap();
    wait_until(&events, session, "a user pause", |k| {
        k.len() == seen.len() + 1 && k.last().is_some_and(|l| l.starts_with("paused(pause,"))
    })
    .await;

    s.stop_debug(session).await.unwrap();
    assert_eq!(
        events.kinds(session).last().map(String::as_str),
        Some("ended(stopped=true)"),
        "{:?}",
        events.kinds(session)
    );
    wait_gone(&pids, "after Stop while paused").await;
    wait_run_finished(&events, run).await;
    assert_eq!(s.debug_session_count().unwrap(), 0, "no task is left");

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    crate::cleanup::remove_project_dir(&dir.path().join("p"), &report.still_running);
}

#[tokio::test]
async fn real_node_close_project_while_paused_leaves_no_process() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open_real(dir.path(), TREE).await;
    let (session, _run, pids) = real_tree_paused(&s, &events).await;

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    assert_eq!(
        events.kinds(session).last().map(String::as_str),
        Some("ended(stopped=true)")
    );
    // The process list is the evidence; closing the project drops the
    // runtime, so no `RunFinished` is promised here.
    wait_gone(&pids, "after close_project while paused").await;
    crate::cleanup::remove_project_dir(&dir.path().join("p"), &report.still_running);
}

#[tokio::test]
async fn real_node_a_program_that_ends_by_itself_ends_the_session() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = open_real(dir.path(), "console.log('done');\n").await;
    s.trust_project().await.unwrap();
    let (session, run) = match s.start_debug(config(&[], &[]), SIZE).await.unwrap() {
        StartDebug::Started { session, run } => (session, run.id),
        other => panic!("expected a started session, got {other:?}"),
    };
    wait_for(&events, session, &["started", "paused(entry,0)"]).await;
    s.debug_resume(session).await.unwrap();
    wait_for(
        &events,
        session,
        &[
            "started",
            "paused(entry,0)",
            "resumed",
            "ended(stopped=false)",
        ],
    )
    .await;
    // Node waits for the debugger to disconnect; closing the connection
    // lets it exit.
    wait_run_finished(&events, run).await;
    wait_output(&events, run, "done").await;
    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    crate::cleanup::remove_project_dir(&dir.path().join("p"), &report.still_running);
}

#[tokio::test]
async fn a_build_without_a_debugger_says_so() {
    if !node_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, _events) = open(dir.path(), false).await;
    s.trust_project().await.unwrap();
    let err = s.start_debug(config(&[], &[]), SIZE).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Internal);
    assert!(s.runs().await.unwrap().is_empty(), "no run may start");
}
