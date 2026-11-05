//! Session integration tests: real files in a temp dir, real recovery store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ferrule_core::{NoEvents, Session};
use ferrule_ipc::{
    CaretDirection, ChangeTransaction, DocId, EditKind, EditorSelection, EntryKind, IpcErrorKind,
    SelectionRange, TextEdit, TokenClass,
};

const AUTOSAVE: Duration = Duration::from_millis(20);

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

fn project_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/app.ts"), "const a = 1;\n").unwrap();
    fs::write(root.join("src/win.ts"), "let x = 1;\r\nlet y = 2;\r\n").unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    dir
}

async fn session(dir: &Path) -> Arc<Session> {
    let session =
        Session::with_autosave_delay(dir.join("data"), Arc::new(NoEvents), AUTOSAVE).unwrap();
    session
        .open_project(dir.join("demo").to_string_lossy().into_owned())
        .await
        .unwrap();
    session
}

#[tokio::test]
async fn open_project_and_list_tree() {
    let dir = project_dir();
    let session = Session::new(dir.path().join("data"), Arc::new(NoEvents)).unwrap();
    let info = session
        .open_project(dir.path().join("demo").to_string_lossy().into_owned())
        .await
        .unwrap();
    assert_eq!(info.name, "demo");
    let root = session.list_dir(String::new()).await.unwrap();
    let names: Vec<_> = root.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert_eq!(
        names,
        vec![
            ("src", EntryKind::Directory),
            ("README.md", EntryKind::File)
        ]
    );
    let src = session.list_dir("src".into()).await.unwrap();
    assert_eq!(src[0].path, "src/app.ts");

    let err = session.list_dir("../..".into()).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
    let err = session
        .open_project(dir.path().join("missing").to_string_lossy().into_owned())
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::NotFound);
}

#[tokio::test]
async fn edit_highlight_undo_redo_save() {
    let dir = project_dir();
    let session = session(dir.path()).await;
    let doc = session.open_document("src/app.ts".into()).await.unwrap();
    assert_eq!(doc.text, "const a = 1;\n");
    assert_eq!(doc.language.as_deref(), Some("TypeScript"));
    assert!(!doc.dirty);

    let hl = session.highlights(doc.doc_id, 0, 100).await.unwrap();
    assert_eq!(hl.spans[0].class, TokenClass::Keyword);
    assert_eq!((hl.spans[0].from, hl.spans[0].to), (0, 5));

    // Type "export " at the start: highlighting follows the edit.
    let status = session
        .apply_changes(doc.doc_id, 0, vec![typing(0, "export ")])
        .await
        .unwrap();
    assert_eq!(status.version, 1);
    assert!(status.dirty);
    let hl = session.highlights(doc.doc_id, 0, 100).await.unwrap();
    assert_eq!(hl.version, 1);
    let keywords: Vec<_> = hl
        .spans
        .iter()
        .filter(|s| s.class == TokenClass::Keyword)
        .map(|s| (s.from, s.to))
        .collect();
    assert_eq!(keywords, vec![(0, 6), (7, 12)]);

    // Stale base version is a conflict.
    let err = session
        .apply_changes(doc.doc_id, 0, vec![typing(0, "x")])
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);

    let undo = session.undo(doc.doc_id, 1).await.unwrap().unwrap();
    assert_eq!(undo.status.version, 2);
    assert_eq!(
        undo.updates,
        vec![vec![TextEdit {
            from: 0,
            to: 7,
            insert: String::new()
        }]]
    );
    assert_eq!(undo.selection, caret(0));
    let redo = session.redo(doc.doc_id, 2).await.unwrap().unwrap();
    assert_eq!(redo.selection, caret(7));
    assert!(session.redo(doc.doc_id, 3).await.unwrap().is_none());

    let saved = session.save_document(doc.doc_id).await.unwrap();
    assert!(!saved.status.dirty);
    let on_disk = fs::read_to_string(dir.path().join("demo/src/app.ts")).unwrap();
    assert_eq!(on_disk, "export const a = 1;\n");
}

#[tokio::test]
async fn save_keeps_crlf_line_endings() {
    let dir = project_dir();
    let session = session(dir.path()).await;
    let doc = session.open_document("src/win.ts".into()).await.unwrap();
    assert_eq!(
        doc.text, "let x = 1;\nlet y = 2;\n",
        "normalized for the view"
    );
    session
        .apply_changes(doc.doc_id, 0, vec![typing(22, "// end\n")])
        .await
        .unwrap();
    session.save_document(doc.doc_id).await.unwrap();
    let bytes = fs::read(dir.path().join("demo/src/win.ts")).unwrap();
    assert_eq!(bytes, b"let x = 1;\r\nlet y = 2;\r\n// end\r\n");
}

#[tokio::test]
async fn multi_cursor_commands() {
    let dir = project_dir();
    let session = session(dir.path()).await;
    let doc = session.open_document("src/win.ts".into()).await.unwrap();
    let below = session
        .add_caret(doc.doc_id, 0, caret(4), CaretDirection::Down)
        .await
        .unwrap();
    assert_eq!(below.ranges.len(), 2);
    assert_eq!(below.ranges[1].head, 15);

    let word = session
        .select_next_occurrence(doc.doc_id, 0, caret(0))
        .await
        .unwrap();
    assert_eq!(word.ranges, vec![SelectionRange { anchor: 0, head: 3 }]);
    let all = session
        .select_all_occurrences(doc.doc_id, 0, word)
        .await
        .unwrap();
    assert_eq!(all.ranges.len(), 2, "both `let`s");

    let err = session
        .select_next_occurrence(doc.doc_id, 7, caret(0))
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
}

#[tokio::test]
async fn unsaved_text_survives_a_crash() {
    let dir = project_dir();
    {
        let session = session(dir.path()).await;
        let doc = session.open_document("src/app.ts".into()).await.unwrap();
        session
            .apply_changes(doc.doc_id, 0, vec![typing(0, "// draft\n")])
            .await
            .unwrap();
        // Wait for the autosave delay, then "crash": drop without saving or closing.
        tokio::time::sleep(AUTOSAVE * 10).await;
    }
    let disk = fs::read_to_string(dir.path().join("demo/src/app.ts")).unwrap();
    assert_eq!(disk, "const a = 1;\n", "the file itself was never saved");

    let session = session(dir.path()).await;
    let doc = session.open_document("src/app.ts".into()).await.unwrap();
    assert!(doc.restored);
    assert!(doc.dirty);
    assert_eq!(doc.text, "// draft\nconst a = 1;\n");

    // Undo goes back to the text on disk.
    let undo = session
        .undo(doc.doc_id, doc.version)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(undo.status.version, doc.version + 1);
    session.save_document(doc.doc_id).await.unwrap();
    session.close_document(doc.doc_id).await.unwrap();
    let again = session.open_document("src/app.ts".into()).await.unwrap();
    assert!(!again.restored, "saving removed the recovery file");
    assert_eq!(again.text, "const a = 1;\n");
}

#[tokio::test]
async fn closing_a_modified_tab_keeps_its_text() {
    let dir = project_dir();
    let session = session(dir.path()).await;
    let doc = session.open_document("README.md".into()).await.unwrap();
    assert_eq!(doc.language.as_deref(), Some("Markdown"));
    let hl = session.highlights(doc.doc_id, 0, 10).await.unwrap();
    assert!(!hl.spans.is_empty(), "the heading is highlighted");
    session
        .apply_changes(doc.doc_id, 0, vec![typing(7, "More.\n")])
        .await
        .unwrap();
    // Close immediately, before the autosave delay: close flushes recovery.
    session.close_document(doc.doc_id).await.unwrap();
    assert_eq!(
        session.highlights(doc.doc_id, 0, 1).await.unwrap_err().kind,
        IpcErrorKind::NotFound
    );
    let reopened = session.open_document("README.md".into()).await.unwrap();
    assert!(reopened.restored);
    assert_eq!(reopened.text, "# Demo\nMore.\n");
    assert_ne!(reopened.doc_id, DocId(0));
}

#[tokio::test]
async fn opening_twice_returns_the_same_document() {
    let dir = project_dir();
    let session = session(dir.path()).await;
    let a = session.open_document("src/app.ts".into()).await.unwrap();
    session
        .apply_changes(a.doc_id, 0, vec![typing(0, "x")])
        .await
        .unwrap();
    let b = session.open_document("src/app.ts".into()).await.unwrap();
    assert_eq!(a.doc_id, b.doc_id);
    assert_eq!(b.version, 1);
    assert_eq!(b.text, "xconst a = 1;\n");
}

#[test]
fn keymap_has_every_action() {
    // Every action except Open Folder, Split Right, Restart TypeScript Server,
    // Branches, Show Diff and Annotate with Git Blame
    // (no default keys in the IntelliJ keymap).
    assert_eq!(ferrule_core::keymap().len(), 38);
}
