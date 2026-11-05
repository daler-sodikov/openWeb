//! The session's language server integration against the scripted
//! `fake-lsp-server` (built by `cargo test --workspace`), configured through
//! the `typescript.serverPath` user setting: document sync from the Rust
//! buffer (emoji and CJK edits, undo, Enter), diagnostics positions, hover
//! rendering and cancellation, navigation, and a missing server.

#![allow(clippy::unwrap_used, clippy::expect_used)]
// Skip reasons are printed so a skipped test is visible in the output.
#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    ChangeTransaction, DocId, EditKind, EditorSelection, IpcErrorKind, NavKind, SelectionRange,
    ServerStateKind, Severity, TextEdit,
};

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn server_states(&self) -> Vec<ServerStateKind> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::ServerStatus(s) => Some(s.state),
                _ => None,
            })
            .collect()
    }
}

/// The fake server next to this test binary (`target/<profile>/`).
fn fake_server() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?;
    let path = dir.join(format!("fake-lsp-server{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        return Some(path);
    }
    let message = format!(
        "fake-lsp-server not built at {}; run `cargo test --workspace`",
        path.display()
    );
    assert!(std::env::var_os("CI").is_none(), "{message}");
    eprintln!("SKIPPED: {message}");
    None
}

struct Fixture {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    root: PathBuf,
    session: Arc<Session>,
    events: Arc<Events>,
}

async fn fixture(server_path: &Path, files: &[(&str, &str)]) -> Fixture {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        std::fs::write(project.path().join(rel), text).unwrap();
    }
    let settings = data.path().join("settings.json");
    std::fs::write(
        &settings,
        serde_json::json!({ "typescript": { "serverPath": server_path } }).to_string(),
    )
    .unwrap();
    let events = Arc::new(Events::default());
    let mut config = SessionConfig::new(data.path());
    config.settings_file = settings;
    let session = Session::with_config(config, events.clone()).unwrap();
    session
        .open_project(project.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    Fixture {
        root: project.path().to_path_buf(),
        _dirs: (project, data),
        session,
        events,
    }
}

async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..1000 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn ready(f: &Fixture) {
    eventually("the server to be ready", || async {
        f.events.server_states().contains(&ServerStateKind::Ready)
    })
    .await;
}

fn cursor(at: u32) -> EditorSelection {
    EditorSelection {
        ranges: vec![SelectionRange {
            anchor: at,
            head: at,
        }],
        main: 0,
    }
}

fn typed(from: u32, to: u32, insert: &str) -> ChangeTransaction {
    ChangeTransaction {
        edits: vec![TextEdit {
            from,
            to,
            insert: insert.into(),
        }],
        kind: EditKind::Other,
        selection_before: cursor(from),
        selection_after: cursor(from),
    }
}

fn utf16(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap()
}

/// Waits until the diagnostics point exactly at every `error` in the text.
async fn diagnostics_match(f: &Fixture, doc: DocId, text: &str) {
    let expected: Vec<u32> = text
        .match_indices("error")
        .map(|(byte, _)| utf16(&text[..byte]))
        .collect();
    eventually("diagnostics at every `error`", || async {
        let diagnostics = f.session.doc_diagnostics(doc).await.unwrap();
        let starts: Vec<u32> = diagnostics
            .items
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.from)
            .collect();
        starts == expected
            && diagnostics
                .items
                .iter()
                .all(|d| d.to - d.from == utf16("error"))
    })
    .await;
}

#[tokio::test]
async fn sync_follows_every_edit_path_with_emoji_and_cjk() {
    let Some(server) = fake_server() else { return };
    let f = fixture(&server, &[("a.ts", "let 名前 = \"😀\"; // error\n")]).await;
    let doc = f.session.open_document("a.ts".into()).await.unwrap();
    ready(&f).await;
    let mut text = "let 名前 = \"😀\"; // error\n".to_owned();
    diagnostics_match(&f, doc.doc_id, &text).await;

    // Two transactions in one call, each with emoji/CJK around the edits.
    let txs = vec![
        typed(0, 0, "🎉 error 漢字 "),
        typed(
            utf16("🎉 error 漢字 let "),
            utf16("🎉 error 漢字 let 名前"),
            "値😀",
        ),
    ];
    let status = f
        .session
        .apply_changes(doc.doc_id, doc.version, txs)
        .await
        .unwrap();
    text = "🎉 error 漢字 let 値😀 = \"😀\"; // error\n".to_owned();
    diagnostics_match(&f, doc.doc_id, &text).await;

    // Enter computed in Rust, then undo of everything.
    let end = utf16(&text);
    let update = f
        .session
        .new_line(doc.doc_id, status.version, cursor(end))
        .await
        .unwrap();
    let typed_error = f
        .session
        .apply_changes(
            doc.doc_id,
            update.status.version,
            vec![typed(end + 1, end + 1, "error")],
        )
        .await
        .unwrap();
    text.push_str("\nerror");
    diagnostics_match(&f, doc.doc_id, &text).await;
    let mut version = typed_error.version;
    while let Some(update) = f.session.undo(doc.doc_id, version).await.unwrap() {
        version = update.status.version;
    }
    diagnostics_match(&f, doc.doc_id, "let 名前 = \"😀\"; // error\n").await;

    // Problems carry display positions (line, UTF-16 column).
    let problems = f.session.problems().await.unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].path, "a.ts");
    assert_eq!(problems[0].problems[0].line, 0);
    assert_eq!(
        problems[0].problems[0].column,
        utf16("let 名前 = \"😀\"; // ")
    );
}

#[tokio::test]
async fn hover_renders_markdown_and_newer_hovers_cancel_older_ones() {
    let Some(server) = fake_server() else { return };
    let f = fixture(&server, &[("h.ts", "slow 😀 fast\n")]).await;
    let doc = f.session.open_document("h.ts".into()).await.unwrap();
    ready(&f).await;
    let fast = f
        .session
        .hover(doc.doc_id, doc.version, utf16("slow 😀 fa"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fast.html, "<p><strong>fast</strong></p>");

    // The fake server never answers `slow` until it is cancelled.
    let session = f.session.clone();
    let slow = tokio::spawn(async move { session.hover(doc.doc_id, doc.version, 1).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let again = f
        .session
        .hover(doc.doc_id, doc.version, utf16("slow 😀 f"))
        .await
        .unwrap();
    assert!(again.is_some());
    let err = slow.await.unwrap().unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Cancelled);

    // A stale version is refused.
    let stale = f.session.hover(doc.doc_id, doc.version + 1, 0).await;
    assert_eq!(stale.unwrap_err().kind, IpcErrorKind::Conflict);
}

#[tokio::test]
async fn definitions_from_several_files_come_with_previews() {
    let Some(server) = fake_server() else { return };
    let f = fixture(
        &server,
        &[
            ("a.ts", "const x = shared;\n"),
            ("b.ts", "// one\n  export const shared = 1;\n"),
        ],
    )
    .await;
    let a = f.session.open_document("a.ts".into()).await.unwrap();
    f.session.open_document("b.ts".into()).await.unwrap();
    ready(&f).await;
    // Wait until both documents are open on the server.
    eventually("both documents on the server", || async {
        f.session.problems().await.unwrap().is_empty()
            && f.session
                .goto_definition(a.doc_id, a.version, 12, NavKind::Definition)
                .await
                .is_ok_and(|t| t.len() == 2)
    })
    .await;
    let targets = f
        .session
        .goto_definition(a.doc_id, a.version, 12, NavKind::Definition)
        .await
        .unwrap();
    assert_eq!(targets[0].path, "a.ts");
    assert_eq!((targets[0].line, targets[0].column), (0, 10));
    assert_eq!(targets[1].path, "b.ts");
    assert_eq!((targets[1].line, targets[1].column), (1, 15));
    assert_eq!(targets[1].preview, "export const shared = 1;");
    assert!(targets.iter().all(|t| t.in_project));
    let _ = &f.root;
}

#[tokio::test]
async fn a_missing_server_is_reported_and_editing_still_works() {
    let missing = std::env::temp_dir().join("ferrule-no-such-server");
    let f = fixture(&missing, &[("m.ts", "let a = 1;\n")]).await;
    let doc = f.session.open_document("m.ts".into()).await.unwrap();
    eventually("the not-found status", || async {
        f.events
            .server_states()
            .contains(&ServerStateKind::NotFound)
    })
    .await;
    let status = f.session.language_servers().await.unwrap();
    assert_eq!(status[0].state, ServerStateKind::NotFound);
    assert!(
        status[0].detail.contains("typescript.serverPath"),
        "{}",
        status[0].detail
    );
    // Editing, saving and code insight requests keep working (no results).
    let changed = f
        .session
        .apply_changes(doc.doc_id, doc.version, vec![typed(0, 0, "// x\n")])
        .await
        .unwrap();
    f.session.save_document(doc.doc_id).await.unwrap();
    assert!(
        f.session
            .hover(doc.doc_id, changed.version, 0)
            .await
            .unwrap()
            .is_none()
    );
    let nav = f
        .session
        .goto_definition(doc.doc_id, changed.version, 0, NavKind::Definition)
        .await
        .unwrap_err();
    assert_eq!(nav.kind, IpcErrorKind::Conflict);
    assert!(
        f.session
            .doc_diagnostics(doc.doc_id)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn unused_imports_show_without_any_server() {
    let missing = std::env::temp_dir().join("ferrule-no-such-server");
    let text = "import { a, b } from \"./lib\";\nimport * as unused from \"./other\";\nexport const x = a;\n";
    let f = fixture(&missing, &[("u.ts", text)]).await;
    let doc = f.session.open_document("u.ts".into()).await.unwrap();
    let items = f.session.doc_diagnostics(doc.doc_id).await.unwrap().items;
    let oxc: Vec<(u32, u32, &str)> = items
        .iter()
        .filter(|d| d.source.as_deref() == Some("oxc"))
        .map(|d| (d.from, d.to, d.message.as_str()))
        .collect();
    let second = utf16("import { a, b } from \"./lib\";\n");
    assert_eq!(
        oxc,
        [
            (12, 13, "Unused import 'b'"),
            (
                second,
                second + utf16("import * as unused from \"./other\";"),
                "Unused import 'unused'"
            ),
        ]
    );
    assert!(
        items
            .iter()
            .all(|d| d.unnecessary && d.severity == Severity::Hint)
    );
    // Using `b` removes its hint at once (no server involved).
    let end = utf16(text);
    let changed = f
        .session
        .apply_changes(
            doc.doc_id,
            doc.version,
            vec![typed(end, end, "export const y = b;\n")],
        )
        .await
        .unwrap();
    let items = f.session.doc_diagnostics(doc.doc_id).await.unwrap();
    assert_eq!(items.version, changed.version);
    assert_eq!(items.items.len(), 1);
    assert_eq!(items.items[0].message, "Unused import 'unused'");
}

#[tokio::test]
async fn a_newer_completion_cancels_the_older_one() {
    let Some(server) = fake_server() else { return };
    let f = fixture(&server, &[("c.ts", "slow 😀 fast\nfa")]).await;
    let doc = f.session.open_document("c.ts".into()).await.unwrap();
    ready(&f).await;
    // The fake server never answers completion on `slow` until cancelled.
    let session = f.session.clone();
    let older =
        tokio::spawn(async move { session.completion(doc.doc_id, doc.version, 2, None).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let end = utf16("slow 😀 fast\nfa");
    let newer = f
        .session
        .completion(doc.doc_id, doc.version, end, None)
        .await
        .unwrap()
        .unwrap();
    // Ranked in Rust for `fa`: `fast` matches, `slow` does not. (The fake
    // server also lists the typed `fa` itself, an exact match ranked first.)
    let labels: Vec<&str> = newer.items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"fast") && !labels.contains(&"slow"),
        "{labels:?}"
    );
    assert_eq!(newer.from, end - 2);
    let err = older.await.unwrap().unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Cancelled);
    // A typed character the server does not list as a trigger asks nothing.
    assert!(
        f.session
            .completion(doc.doc_id, doc.version, end, Some(";".into()))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn rename_edits_every_open_document_at_utf16_positions() {
    let Some(server) = fake_server() else { return };
    let a_text = "const 名前 = \"😀\"; // 名前\n";
    let b_text = "🎉 use(名前);\n";
    let f = fixture(&server, &[("a.ts", a_text), ("b.ts", b_text)]).await;
    let a = f.session.open_document("a.ts".into()).await.unwrap();
    f.session.open_document("b.ts".into()).await.unwrap();
    ready(&f).await;
    let at = utf16("const 名");
    let preview = eventually_preview(&f, a.doc_id, a.version, at).await;
    let counts: Vec<(&str, u32)> = preview
        .files
        .iter()
        .map(|p| (p.path.as_str(), p.count))
        .collect();
    assert_eq!(counts, [("a.ts", 2), ("b.ts", 1)]);
    let result = f.session.apply_refactoring(preview.id).await.unwrap();
    assert!(
        result.skipped.is_empty() && result.files_written.is_empty(),
        "{result:?}"
    );
    assert_eq!(result.open_documents.len(), 2);
    for replace in &result.open_documents {
        f.session
            .apply_changes(
                replace.doc_id,
                replace.version,
                vec![ChangeTransaction {
                    edits: replace.edits.clone(),
                    kind: EditKind::Other,
                    selection_before: cursor(0),
                    selection_after: cursor(0),
                }],
            )
            .await
            .unwrap();
    }
    let a_now = f.session.open_document("a.ts".into()).await.unwrap().text;
    let b_now = f.session.open_document("b.ts".into()).await.unwrap().text;
    assert_eq!(a_now, "const 値😀 = \"😀\"; // 値😀\n");
    assert_eq!(b_now, "🎉 use(値😀);\n");
}

/// Rename preview once both documents are open on the fake server.
async fn eventually_preview(
    f: &Fixture,
    doc: DocId,
    version: u32,
    at: u32,
) -> ferrule_ipc::RefactoringPreview {
    for _ in 0..500 {
        if let Ok(preview) = f
            .session
            .rename_preview(doc, version, at, "値😀".into())
            .await
            && preview.files.len() == 2
        {
            return preview;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for a rename preview over both files");
}
