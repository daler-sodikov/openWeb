//! M2 session tests: indentation, undo grouping, modified state, reload,
//! search, find/replace and file watching. Real files in a temp dir.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_core::{EventSink, Session, SessionEvent};
use ferrule_ipc::{
    ChangeTransaction, EditKind, EditorSelection, FsChangeKind, IpcErrorKind, SearchItem,
    SearchTab, SelectionRange, TextEdit, TextSearch,
};

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn caret(pos: u32) -> EditorSelection {
    EditorSelection {
        ranges: vec![SelectionRange {
            anchor: pos,
            head: pos,
        }],
        main: 0,
    }
}

fn typing(at: u32, text: &str) -> ChangeTransaction {
    let len = text.encode_utf16().count() as u32;
    ChangeTransaction {
        edits: vec![TextEdit {
            from: at,
            to: at,
            insert: text.into(),
        }],
        kind: EditKind::Typing,
        selection_before: caret(at),
        selection_after: caret(at + len),
    }
}

fn search(pattern: &str) -> TextSearch {
    TextSearch {
        pattern: pattern.into(),
        regex: false,
        case_sensitive: false,
        whole_word: false,
        replacement: None,
    }
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    session: Arc<Session>,
    events: Arc<Events>,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    write(
        &root,
        "src/components/Counter.tsx",
        "export function Counter() {\n  return <div>count</div>;\n}\n",
    );
    write(
        &root,
        "src/utils/format.ts",
        "export function formatPrice(v: number) {\n  return `${v} EUR`;\n}\n",
    );
    write(
        &root,
        "src/win.ts",
        "const count = 1;\r\nconst counter = 2;\r\n",
    );
    write(&root, "dist/bundle.js", "function count() {}");
    write(&root, ".gitignore", "dist\n");
    let events = Arc::new(Events::default());
    let session = Session::with_autosave_delay(
        dir.path().join("data"),
        events.clone(),
        Duration::from_millis(20),
    )
    .unwrap();
    let root = fs::canonicalize(&root).unwrap();
    session
        .open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    wait_for_index(&session).await;
    Fixture {
        _dir: dir,
        root,
        session,
        events,
    }
}

async fn wait_for_index(session: &Session) {
    for _ in 0..200 {
        let r = session
            .search_everywhere(String::new(), SearchTab::Files)
            .await
            .unwrap();
        if !r.indexing {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("index not ready");
}

#[tokio::test]
async fn enter_indents_in_rust_and_groups_with_typing() {
    let f = fixture().await;
    let s = &f.session;
    let doc = s.open_document("src/utils/format.ts".into()).await.unwrap();
    assert_eq!(doc.indent_unit, "  ");
    let end_of_line_1 = 40; // after "{" of `export function formatPrice(v: number) {`
    s.apply_changes(doc.doc_id, 0, vec![typing(end_of_line_1, " ")])
        .await
        .unwrap();
    let enter = s
        .new_line(doc.doc_id, 1, caret(end_of_line_1 + 1))
        .await
        .unwrap();
    // Trailing space removed, indented one level after "{".
    assert_eq!(
        enter.updates,
        vec![vec![TextEdit {
            from: 40,
            to: 41,
            insert: "\n  ".into()
        }]]
    );
    assert_eq!(enter.selection, caret(43));
    s.apply_changes(doc.doc_id, 2, vec![typing(43, "x")])
        .await
        .unwrap();

    // One undo removes the space, the newline and the "x".
    let undo = s.undo(doc.doc_id, 3).await.unwrap().unwrap();
    assert_eq!(undo.updates.len(), 3);
    assert!(!undo.status.dirty, "back to the saved text");
}

#[tokio::test]
async fn modified_state_follows_the_text_not_the_version() {
    let f = fixture().await;
    let s = &f.session;
    let doc = s.open_document("src/win.ts".into()).await.unwrap();
    let st = s
        .apply_changes(doc.doc_id, 0, vec![typing(0, "a")])
        .await
        .unwrap();
    assert!(st.dirty);
    let undo = s.undo(doc.doc_id, 1).await.unwrap().unwrap();
    assert!(
        !undo.status.dirty,
        "undo back to the saved text clears the marker"
    );
    let redo = s.redo(doc.doc_id, 2).await.unwrap().unwrap();
    assert!(redo.status.dirty);
    s.save_document(doc.doc_id).await.unwrap();
    let undo = s.undo(doc.doc_id, 3).await.unwrap().unwrap();
    assert!(
        undo.status.dirty,
        "after saving, the saved text is the new baseline"
    );
}

#[tokio::test]
async fn reload_document_from_disk() {
    let f = fixture().await;
    let s = &f.session;
    let doc = s.open_document("src/win.ts".into()).await.unwrap();
    assert!(!s.disk_changed(doc.doc_id).await.unwrap());
    let same = s.reload_document(doc.doc_id, 0).await.unwrap();
    assert!(same.updates.is_empty());

    fs::write(
        f.root.join("src/win.ts"),
        "const count = 10;\nconst counter = 2;\n",
    )
    .unwrap();
    assert!(s.disk_changed(doc.doc_id).await.unwrap());
    let reload = s.reload_document(doc.doc_id, 0).await.unwrap();
    assert_eq!(
        reload.updates,
        vec![vec![TextEdit {
            from: 15,
            to: 15,
            insert: "0".into()
        }]],
        "only the differing part is replaced"
    );
    assert!(!reload.status.dirty);
    let err = s.reload_document(doc.doc_id, 0).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
    // Reload is undoable and then the buffer differs from disk.
    let undo = s
        .undo(doc.doc_id, reload.status.version)
        .await
        .unwrap()
        .unwrap();
    assert!(undo.status.dirty);
}

#[tokio::test]
async fn search_everywhere_files_symbols_actions_and_recent() {
    let f = fixture().await;
    let s = &f.session;
    let all = s
        .search_everywhere("cnt".into(), SearchTab::All)
        .await
        .unwrap();
    let mut saw_file = false;
    let mut saw_symbol = false;
    for item in &all.items {
        match item {
            SearchItem::File { path, .. } if path == "src/components/Counter.tsx" => {
                saw_file = true
            }
            SearchItem::Symbol {
                name, path, line, ..
            } if name == "Counter" => {
                assert_eq!((path.as_str(), *line), ("src/components/Counter.tsx", 0));
                saw_symbol = true;
            }
            SearchItem::File { path, .. } => {
                assert!(!path.starts_with("dist/"), "ignored file listed")
            }
            _ => {}
        }
    }
    assert!(saw_file && saw_symbol, "{all:?}");

    let actions = s
        .search_everywhere("fif".into(), SearchTab::Actions)
        .await
        .unwrap();
    assert!(
        matches!(&actions.items[0], SearchItem::Action { name, .. } if name == "Find in Files")
    );

    let symbols = s
        .search_everywhere("fP".into(), SearchTab::Symbols)
        .await
        .unwrap();
    assert!(matches!(&symbols.items[0], SearchItem::Symbol { name, .. } if name == "formatPrice"));

    s.open_document("src/win.ts".into()).await.unwrap();
    s.open_document("src/utils/format.ts".into()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        s.recent_files().await.unwrap(),
        vec!["src/utils/format.ts", "src/win.ts"]
    );
    let empty = s
        .search_everywhere(String::new(), SearchTab::Files)
        .await
        .unwrap();
    assert!(
        matches!(&empty.items[0], SearchItem::File { path, .. } if path == "src/utils/format.ts")
    );
}

#[tokio::test]
async fn find_in_files_searches_unsaved_text_and_respects_options() {
    let f = fixture().await;
    let s = &f.session;
    let found = s.find_in_files(search("count")).await.unwrap();
    let paths: Vec<_> = found
        .matches
        .iter()
        .map(|m| (m.path.as_str(), m.line))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("src/components/Counter.tsx", 0),
            ("src/components/Counter.tsx", 1),
            ("src/win.ts", 0),
            ("src/win.ts", 1),
        ],
        "dist/ is ignored"
    );
    let whole = s
        .find_in_files(TextSearch {
            whole_word: true,
            case_sensitive: true,
            ..search("count")
        })
        .await
        .unwrap();
    assert_eq!(whole.matches.len(), 2);
    assert_eq!(whole.matches[1].line_text, "const count = 1;");

    // Unsaved text is searched instead of the file on disk.
    let doc = s.open_document("src/win.ts".into()).await.unwrap();
    s.apply_changes(doc.doc_id, 0, vec![typing(0, "// needle\n")])
        .await
        .unwrap();
    let needle = s.find_in_files(search("needle")).await.unwrap();
    assert_eq!(needle.matches.len(), 1);
    assert_eq!(needle.matches[0].path, "src/win.ts");

    let regex = s
        .find_in_files(TextSearch {
            regex: true,
            replacement: Some("[$1]".into()),
            ..search(r"format(\w+)")
        })
        .await
        .unwrap();
    assert_eq!(regex.matches[0].replacement.as_deref(), Some("[Price]"));

    let err = s
        .find_in_files(TextSearch {
            regex: true,
            ..search("(")
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
}

#[tokio::test]
async fn replace_in_files_writes_closed_files_and_returns_open_edits() {
    let f = fixture().await;
    let s = &f.session;
    let doc = s
        .open_document("src/components/Counter.tsx".into())
        .await
        .unwrap();
    let result = s
        .replace_in_files(
            TextSearch {
                whole_word: true,
                ..search("count")
            },
            "total".into(),
        )
        .await
        .unwrap();
    assert_eq!(result.files_written, 1);
    assert_eq!(result.replacements, 2);
    assert_eq!(
        fs::read(f.root.join("src/win.ts")).unwrap(),
        b"const total = 1;\r\nconst counter = 2;\r\n",
        "CRLF kept, `counter` untouched"
    );
    assert_eq!(result.open_documents.len(), 1, "Counter.tsx: `count</div>`");
    assert_eq!(result.open_documents[0].doc_id, doc.doc_id);
    assert_eq!(result.open_documents[0].edits[0].insert, "total");
    // The open document itself is not changed by the backend.
    let reopened = s
        .open_document("src/components/Counter.tsx".into())
        .await
        .unwrap();
    assert!(reopened.text.contains("count</div>"));
}

#[tokio::test]
async fn watcher_updates_index_and_emits_events() {
    let f = fixture().await;
    let s = &f.session;
    write(
        &f.root,
        "src/fresh/NewThing.ts",
        "export class NewThing {}\n",
    );
    fs::remove_file(f.root.join("src/win.ts")).unwrap();

    let mut found = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let r = s
            .search_everywhere("NewThing".into(), SearchTab::Symbols)
            .await
            .unwrap();
        if !r.items.is_empty() {
            found = true;
            break;
        }
    }
    assert!(found, "new file indexed through the watcher");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = f.events.0.lock().unwrap().clone();
    let changes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::FsChanged(batch) => Some(batch.changes.clone()),
            SessionEvent::IndexStatus(_) => None,
        })
        .flatten()
        .collect();
    assert!(
        changes
            .iter()
            .any(|c| c.path == "src/win.ts" && c.kind == FsChangeKind::Removed),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|c| c.path.starts_with("src/fresh") && c.kind == FsChangeKind::Changed),
        "{changes:?}"
    );
    let files = s
        .search_everywhere("win".into(), SearchTab::Files)
        .await
        .unwrap();
    assert!(
        !files
            .items
            .iter()
            .any(|i| matches!(i, SearchItem::File { path, .. } if path == "src/win.ts")),
        "removed file dropped from the index: {files:?}"
    );
}
