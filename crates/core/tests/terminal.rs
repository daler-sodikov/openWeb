//! Terminal commands over the in-memory backend: shell choice, working
//! directory, frames and acknowledgements, links, rename, and that closing
//! or switching projects ends the processes. Real PTYs are tested in
//! `ferrule-terminal` (native feature).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{IpcErrorKind, TerminalCell, TerminalFrame, TerminalSize};
use ferrule_terminal::fake::FakeBackend;

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn frames(&self) -> Vec<TerminalFrame> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::TerminalFrame(f) => Some(f.clone()),
                _ => None,
            })
            .collect()
    }
}

fn row_text(frame: &TerminalFrame, row: u32) -> Option<String> {
    frame
        .rows
        .iter()
        .find(|r| r.row == row)
        .map(|r| r.runs.iter().map(|run| run.text.as_str()).collect())
}

/// Waits for a frame whose row 0 contains `needle`, acknowledging frames
/// as the view does.
async fn wait_for_row(s: &Session, events: &Events, id: u32, needle: &str) -> TerminalFrame {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for frame in events.frames() {
            s.terminal_ack(id, frame.seq).await.unwrap();
            if row_text(&frame, 0).is_some_and(|t| t.contains(needle)) {
                return frame;
            }
        }
        assert!(Instant::now() < deadline, "no frame showed {needle:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

async fn session(dir: &Path, backend: Option<Arc<FakeBackend>>) -> (Arc<Session>, Arc<Events>) {
    let root = dir.join("p");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/a.ts"),
        "export const a = 1;\nexport const b = 2;\n",
    )
    .unwrap();
    let events = Arc::new(Events::default());
    let mut config = SessionConfig::new(dir.join("data"));
    config.terminal_backend = backend.map(|b| b as Arc<dyn ferrule_terminal::Backend>);
    let s = Session::with_config(config, events.clone()).unwrap();
    s.open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    (s, events)
}

#[tokio::test]
async fn without_a_backend_terminals_say_why() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = session(dir.path(), None).await;
    let list = s.terminals().await.unwrap();
    assert!(list.unavailable.unwrap().contains("native"));
    let err = s.open_terminal(SIZE).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
}

#[tokio::test]
async fn a_terminal_starts_in_the_project_root_and_echoes() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let (s, events) = session(dir.path(), Some(backend.clone())).await;
    let info = s.open_terminal(SIZE).await.unwrap();
    {
        let spawned = backend.log.spawned.lock().unwrap();
        let spec = &spawned[0];
        assert_eq!(
            fs::canonicalize(&spec.cwd).unwrap(),
            fs::canonicalize(dir.path().join("p")).unwrap()
        );
        assert!(
            spec.env
                .iter()
                .any(|(k, v)| k == "TERM" && v == "xterm-256color")
        );
    }
    s.terminal_input(info.id, "echo hi".into()).await.unwrap();
    let frame = wait_for_row(&s, &events, info.id, "echo hi").await;
    assert_eq!(frame.id, info.id);
    assert_eq!(frame.size, SIZE);

    s.terminal_resize(
        info.id,
        TerminalSize {
            cols: 100,
            rows: 30,
        },
    )
    .await
    .unwrap();
    assert_eq!(backend.log.resized.lock().unwrap().len(), 1);
    assert_eq!(s.terminals().await.unwrap().terminals.len(), 1);
}

#[tokio::test]
async fn the_shell_setting_wins() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let (s, _) = session(dir.path(), Some(backend.clone())).await;
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::write(
        dir.path().join("data/settings.json"),
        r#"{ "terminal": { "shellPath": "/opt/my-shell", "shellArgs": ["--fancy"] } }"#,
    )
    .unwrap();
    let info = s.open_terminal(SIZE).await.unwrap();
    assert_eq!(Path::new(&info.shell), Path::new("/opt/my-shell"));
    assert_eq!(info.title, "my-shell");
    let spawned = backend.log.spawned.lock().unwrap();
    assert_eq!(spawned[0].args, ["--fancy"]);
}

#[tokio::test]
async fn links_resolve_to_project_files() {
    let dir = tempfile::tempdir().unwrap();
    let (s, events) = session(dir.path(), Some(Arc::new(FakeBackend::default()))).await;
    let id = s.open_terminal(SIZE).await.unwrap().id;
    s.terminal_input(id, "at src/a.ts:2:8 and nope.ts:1".into())
        .await
        .unwrap();
    wait_for_row(&s, &events, id, "nope.ts").await;
    let cell = |col| TerminalCell { row: 0, col };
    let target = s.terminal_link_at(id, cell(5)).await.unwrap().unwrap();
    assert_eq!(target.path, "src/a.ts");
    assert!(target.in_project);
    assert_eq!((target.line, target.column), (1, 7));
    assert_eq!(s.terminal_link_at(id, cell(1)).await.unwrap(), None);
    assert_eq!(
        s.terminal_link_at(id, cell(22)).await.unwrap(),
        None,
        "missing files are not links"
    );
}

#[tokio::test]
async fn rename_close_and_project_switch_end_terminals() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let (s, _) = session(dir.path(), Some(backend.clone())).await;
    let first = s.open_terminal(SIZE).await.unwrap().id;
    let second = s.open_terminal(SIZE).await.unwrap().id;
    assert_ne!(first, second);

    let renamed = s.rename_terminal(first, "  build ".into()).await.unwrap();
    assert_eq!(renamed.title, "build");

    s.close_terminal(first).await.unwrap();
    assert_eq!(*backend.log.terminated.lock().unwrap(), 1);
    let err = s.terminal_input(first, "x".into()).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::NotFound);

    let other = dir.path().join("q");
    fs::create_dir_all(&other).unwrap();
    s.open_project(other.to_string_lossy().into_owned())
        .await
        .unwrap();
    assert_eq!(*backend.log.terminated.lock().unwrap(), 2);
    assert!(s.terminals().await.unwrap().terminals.is_empty());
}
