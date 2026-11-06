//! `Supervisor` against the scripted `fake-lsp-server` process: document
//! sync (with emoji and CJK text), debouncing, cancellation, progress,
//! crash and restart, giving up, encoding negotiation and shutdown.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_lsp_client::lsp_types::notification::Notification;
use ferrule_lsp_client::lsp_types::request::{HoverRequest, Request};
use ferrule_lsp_client::lsp_types::{
    HoverParams, Position, Range, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkDoneProgressParams,
};
use ferrule_lsp_client::{
    ClientHooks, LspError, RestartPolicy, ServerCommand, ServerConfig, ServerState, Supervisor,
    path_to_uri,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Hooks {
    states: Mutex<Vec<ServerState>>,
    started: Mutex<Vec<u64>>,
    notifications: Mutex<Vec<(String, Value)>>,
}

impl ClientHooks for Hooks {
    fn state_changed(&self, state: &ServerState) {
        self.states.lock().unwrap().push(state.clone());
    }

    fn started(&self, generation: u64) {
        self.started.lock().unwrap().push(generation);
    }

    fn notification(&self, method: &str, params: &Value) {
        self.notifications
            .lock()
            .unwrap()
            .push((method.to_owned(), params.clone()));
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    hooks: Arc<Hooks>,
    server: Supervisor,
}

fn config(root: PathBuf, env: Vec<(&str, &str)>) -> ServerConfig {
    let command = ServerCommand {
        program: PathBuf::from(env!("CARGO_BIN_EXE_fake-lsp-server")),
        args: Vec::new(),
        cwd: root.clone(),
        env: env
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    };
    let mut config = ServerConfig::new("Fake", command, root);
    config.restart = RestartPolicy {
        base_delay: Duration::from_millis(10),
        max_attempts: 2,
        stable_after: Duration::from_secs(60),
    };
    config.debounce = Duration::from_millis(300);
    config
}

fn start(env: Vec<(&str, &str)>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let hooks = Arc::new(Hooks::default());
    let server = Supervisor::start(config(root.clone(), env), hooks.clone());
    Fixture {
        _dir: dir,
        root,
        hooks,
        server,
    }
}

/// Polls `check` for up to five seconds.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn running(fixture: &Fixture, generation: u64) {
    eventually(&format!("generation {generation}"), || {
        fixture.server.generation() == Some(generation)
    })
    .await;
}

fn uri(fixture: &Fixture, name: &str) -> Uri {
    path_to_uri(&fixture.root.join(name)).unwrap()
}

fn change(start: (u32, u32), end: (u32, u32), text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range::new(
            Position::new(start.0, start.1),
            Position::new(end.0, end.1),
        )),
        range_length: None,
        text: text.into(),
    }
}

/// Custom requests understood by the fake server.
enum FakeText {}
impl Request for FakeText {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "fake/text";
}
enum FakeCrash {}
impl Notification for FakeCrash {
    type Params = ();
    const METHOD: &'static str = "fake/crash";
}
enum FakeLog {}
impl Request for FakeLog {
    type Params = Value;
    type Result = Vec<String>;
    const METHOD: &'static str = "fake/log";
}

async fn server_text(fixture: &Fixture, uri: &Uri) -> Value {
    fixture
        .server
        .request::<FakeText>(json!({ "uri": uri.as_str() }), &CancellationToken::new())
        .await
        .unwrap()
}

async fn server_log(fixture: &Fixture) -> Vec<String> {
    fixture
        .server
        .request::<FakeLog>(Value::Null, &CancellationToken::new())
        .await
        .unwrap()
}

fn hover_params(uri: &Uri, line: u32, character: u32) -> HoverParams {
    HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position: Position::new(line, character),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    }
}

#[tokio::test]
async fn incremental_sync_matches_with_emoji_and_cjk() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    let uri = uri(&fixture, "a.ts");
    assert!(fixture.server.did_open(
        1,
        uri.clone(),
        "typescript",
        1,
        "let 名前 = \"😀\";\n".into()
    ));
    // "😀" is two UTF-16 units.
    let changes = vec![
        // Each change is relative to the text after the previous one.
        change((0, 4), (0, 6), "値🎉"),   // `let 値🎉 = "😀";`
        change((0, 11), (0, 13), "漢字"), // `let 値🎉 = "漢字";`
        change((0, 15), (0, 15), "\n// 👍"),
    ];
    assert!(
        fixture
            .server
            .did_change(1, &uri, 2, changes, || unreachable!("incremental"))
    );
    fixture.server.flush(&uri);
    let synced = server_text(&fixture, &uri).await;
    assert_eq!(synced["text"], "let 値🎉 = \"漢字\";\n// 👍\n");
    assert_eq!(synced["version"], 2);
    assert!(fixture.server.did_save(1, &uri));
    assert!(fixture.server.did_close(1, &uri));
    let log = server_log(&fixture).await;
    let sync: Vec<&str> = log
        .iter()
        .map(String::as_str)
        .filter(|m| m.starts_with("textDocument/"))
        .collect();
    assert_eq!(
        sync,
        [
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/didSave",
            "textDocument/didClose"
        ]
    );
    // A stale generation is refused.
    assert!(
        !fixture
            .server
            .did_open(7, uri, "typescript", 1, String::new())
    );
}

#[tokio::test]
async fn changes_are_debounced_into_one_notification() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    let uri = uri(&fixture, "b.ts");
    fixture
        .server
        .did_open(1, uri.clone(), "typescript", 1, String::new());
    for (version, text) in [(2, "a"), (3, "b"), (4, "c")] {
        let at = u32::try_from(version - 2).unwrap();
        fixture.server.did_change(
            1,
            &uri,
            version,
            vec![change((0, at), (0, at), text)],
            || unreachable!(),
        );
    }
    // Not sent yet (debounced) …
    let early = server_log(&fixture).await;
    assert!(!early.iter().any(|m| m == "textDocument/didChange"));
    // … then one notification with all three changes and the last version.
    eventually("the debounced didChange", || {
        fixture
            .hooks
            .notifications
            .lock()
            .unwrap()
            .iter()
            .any(|(m, p)| m == "textDocument/publishDiagnostics" && p["version"] == 4)
    })
    .await;
    let log = server_log(&fixture).await;
    assert_eq!(
        log.iter()
            .filter(|m| *m == "textDocument/didChange")
            .count(),
        1
    );
    assert_eq!(server_text(&fixture, &uri).await["text"], "abc");
}

#[tokio::test]
async fn publishes_diagnostics_at_utf16_positions() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    let uri = uri(&fixture, "c.ts");
    fixture
        .server
        .did_open(1, uri.clone(), "typescript", 1, "😀漢 error\n".into());
    eventually("diagnostics", || {
        fixture
            .hooks
            .notifications
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "textDocument/publishDiagnostics")
    })
    .await;
    let notifications = fixture.hooks.notifications.lock().unwrap();
    let (_, params) = notifications
        .iter()
        .find(|(m, _)| m == "textDocument/publishDiagnostics")
        .unwrap();
    // 😀 = 2 units, 漢 = 1, space = 1.
    assert_eq!(
        params["diagnostics"][0]["range"]["start"],
        json!({ "line": 0, "character": 4 })
    );
}

#[tokio::test]
async fn cancelling_a_request_sends_cancel_request() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    let uri = uri(&fixture, "d.ts");
    fixture
        .server
        .did_open(1, uri.clone(), "typescript", 1, "slow fast".into());
    let cancel = CancellationToken::new();
    let slow = {
        let server = fixture.server.clone();
        let cancel = cancel.clone();
        let params = hover_params(&uri, 0, 1);
        tokio::spawn(async move { server.request::<HoverRequest>(params, &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    assert!(matches!(slow.await.unwrap(), Err(LspError::Cancelled)));
    // The connection still works, and the server saw the cancellation.
    let fast = fixture
        .server
        .request::<HoverRequest>(hover_params(&uri, 0, 6), &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(fast.contents).unwrap()["value"],
        "**fast**"
    );
    assert!(
        server_log(&fixture)
            .await
            .contains(&"$/cancelRequest".to_owned())
    );
}

#[tokio::test]
async fn reports_progress_then_ready() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    eventually("ready after progress", || {
        let states = fixture.hooks.states.lock().unwrap();
        states.contains(&ServerState::Busy("Indexing fake project".into()))
            && states.last() == Some(&ServerState::Ready)
    })
    .await;
    assert_eq!(
        fixture.server.server_info().as_deref(),
        Some("fake-lsp-server 1.0")
    );
}

#[tokio::test]
async fn restarts_after_a_crash_and_reopens_documents() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    fixture.server.notify::<FakeCrash>(()).unwrap();
    running(&fixture, 2).await;
    assert_eq!(*fixture.hooks.started.lock().unwrap(), [1, 2]);
    let states = fixture.hooks.states.lock().unwrap().clone();
    assert!(
        states
            .iter()
            .any(|s| matches!(s, ServerState::Restarting { attempt: 1, .. })),
        "{states:?}"
    );
    // The old generation is gone: its documents must be opened again.
    let uri = uri(&fixture, "e.ts");
    assert!(
        !fixture
            .server
            .did_open(1, uri.clone(), "typescript", 1, "x".into())
    );
    assert!(fixture.server.did_open(2, uri, "typescript", 1, "x".into()));
}

#[tokio::test]
async fn gives_up_after_repeated_failures() {
    let fixture = start(vec![("FAKE_LSP_FAIL_START", "1")]);
    eventually("the failed state", || {
        matches!(fixture.server.state(), ServerState::Failed(_))
    })
    .await;
    let ServerState::Failed(message) = fixture.server.state() else {
        unreachable!()
    };
    assert!(message.contains("3 failed starts"), "{message}");
    let states = fixture.hooks.states.lock().unwrap().clone();
    let delays: Vec<Duration> = states
        .iter()
        .filter_map(|s| match s {
            ServerState::Restarting { delay, .. } => Some(*delay),
            _ => None,
        })
        .collect();
    assert_eq!(
        delays,
        [Duration::from_millis(10), Duration::from_millis(20)]
    );
}

#[tokio::test]
async fn refuses_servers_that_do_not_use_utf16() {
    let fixture = start(vec![("FAKE_LSP_ENCODING", "utf-8")]);
    eventually("the failed state", || {
        matches!(fixture.server.state(), ServerState::Failed(_))
    })
    .await;
    let ServerState::Failed(message) = fixture.server.state() else {
        unreachable!()
    };
    assert!(message.contains("utf-16"), "{message}");
    assert_eq!(fixture.server.generation(), None);
}

#[tokio::test]
async fn stop_shuts_the_server_down() {
    let fixture = start(Vec::new());
    running(&fixture, 1).await;
    tokio::time::timeout(Duration::from_secs(5), fixture.server.stop())
        .await
        .unwrap();
    assert_eq!(fixture.server.state(), ServerState::Stopped);
    assert_eq!(fixture.server.generation(), None);
    assert!(matches!(
        fixture
            .server
            .request::<FakeLog>(Value::Null, &CancellationToken::new())
            .await,
        Err(LspError::NotRunning)
    ));
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

#[tokio::test]
async fn stopping_ends_the_processes_the_server_started() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    let fixture = start(vec![("FAKE_LSP_CHILD_PIDFILE", pidfile.to_str().unwrap())]);
    running(&fixture, 1).await;
    eventually("the child's pid", || pidfile.exists()).await;
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid));
    tokio::time::timeout(Duration::from_secs(10), fixture.server.stop())
        .await
        .unwrap();
    // The server exits cleanly but leaves its child; stopping ends it too.
    eventually("the server's child to end", || !alive(pid)).await;
}
