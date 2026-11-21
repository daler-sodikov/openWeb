//! Debug sessions (M5b-5) against the scripted debugger: trust, the event
//! sequence with `seq`, refused inspector flags, and the cleanup of Stop
//! and `close_project`, also while paused. The runs use the in-memory run
//! backend; the real-Node test comes with the CDP backend.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_core::fake_debug::FakeConnector;
use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    DebugEvent, DebugEventKind, DebugPauseReason, IpcErrorKind, RunConfig, RunKind, StartDebug,
    TerminalSize,
};
use ferrule_run_debug::fake::FakeRunBackend;
use ferrule_run_debug::fake_debug::{FakeDebug, ScriptItem};
use ferrule_run_debug::{Frame, Location, PauseInfo, PauseReason};

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
                        _ => "other",
                    },
                    frames[0].location.line
                ),
            })
            .collect()
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

async fn open(dir: &Path, debugger: bool) -> (Arc<Session>, Arc<Events>) {
    let root = dir.join("p");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.js"), "console.log('hi');\n").unwrap();
    let events = Arc::new(Events::default());
    let mut config = SessionConfig::new(dir.join("data"));
    config.run_backend = Some(Arc::new(FakeRunBackend));
    if debugger {
        config.debug_connector = Some(Arc::new(FakeConnector::new(program)));
    }
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

/// Waits until the session's events are exactly `kinds`.
async fn wait_for(events: &Events, session: u32, kinds: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if events.kinds(session) == kinds {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {kinds:?}, got {:?}",
            events.kinds(session)
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
            stopped: false
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
