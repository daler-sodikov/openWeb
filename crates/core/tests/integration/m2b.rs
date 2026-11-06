//! M2b session tests: EditorConfig, caret info, folds, ignored entries and
//! embedded languages. Real files in a temp dir.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ferrule_core::{NoEvents, Session};
use ferrule_ipc::{
    ChangeTransaction, EditKind, EditorSelection, SelectionRange, TextEdit, TokenClass,
};

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
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

async fn session(dir: &Path) -> Arc<Session> {
    let s = Session::with_autosave_delay(
        dir.join("data"),
        Arc::new(NoEvents),
        Duration::from_millis(20),
    )
    .unwrap();
    s.open_project(dir.join("p").to_string_lossy().into_owned())
        .await
        .unwrap();
    s
}

#[tokio::test]
async fn editorconfig_sets_indentation_and_applies_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    write(
        &root,
        ".editorconfig",
        "root = true\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = crlf\n\
         trim_trailing_whitespace = true\ninsert_final_newline = true\n[*.md]\ntrim_trailing_whitespace = false\n",
    );
    write(&root, "a.ts", "if (x) {\n  y();   \n}");
    write(&root, "n.md", "keep  \nthis");
    let s = session(dir.path()).await;

    let doc = s.open_document("a.ts".into()).await.unwrap();
    assert_eq!(
        doc.indent_unit, "    ",
        "EditorConfig beats the detected 2 spaces"
    );
    assert_eq!(doc.tab_width, 4);
    assert!(doc.auto_close.contains(&"``".to_owned()));

    // Make it dirty, then save: trailing spaces trimmed, final newline added,
    // CRLF written, and the change is one undoable step.
    s.apply_changes(
        doc.doc_id,
        0,
        vec![ChangeTransaction {
            edits: vec![TextEdit {
                from: 0,
                to: 0,
                insert: "// x\n".into(),
            }],
            kind: EditKind::Typing,
            selection_before: caret(0),
            selection_after: caret(5),
        }],
    )
    .await
    .unwrap();
    let saved = s.save_document(doc.doc_id).await.unwrap();
    assert_eq!(saved.updates.len(), 1);
    assert!(!saved.status.dirty);
    assert_eq!(
        fs::read(root.join("a.ts")).unwrap(),
        b"// x\r\nif (x) {\r\n  y();\r\n}\r\n"
    );
    let undo = s
        .undo(doc.doc_id, saved.status.version)
        .await
        .unwrap()
        .unwrap();
    assert!(
        undo.status.dirty,
        "undoing the save edits differs from the saved text"
    );

    let md = s.open_document("n.md".into()).await.unwrap();
    s.apply_changes(
        md.doc_id,
        0,
        vec![ChangeTransaction {
            edits: vec![TextEdit {
                from: 0,
                to: 0,
                insert: "x".into(),
            }],
            kind: EditKind::Typing,
            selection_before: caret(0),
            selection_after: caret(1),
        }],
    )
    .await
    .unwrap();
    s.save_document(md.doc_id).await.unwrap();
    assert_eq!(
        fs::read(root.join("n.md")).unwrap(),
        b"xkeep  \r\nthis\r\n",
        "section override keeps spaces"
    );
}

#[tokio::test]
async fn caret_info_and_folds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    let text = "export function App() {\n  return (\n    <main>\n      <b>😀 {\"x\"}</b>\n    </main>\n  );\n}\n";
    write(&root, "App.tsx", text);
    let s = session(dir.path()).await;
    let doc = s.open_document("App.tsx".into()).await.unwrap();

    let b_tag = text.find("<b>").unwrap() as u32 + 3; // inside <b>, ASCII prefix
    let info = s.caret_info(doc.doc_id, b_tag).await.unwrap();
    let labels: Vec<_> = info.breadcrumbs.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["App", "main", "b"]);
    assert_eq!(
        info.breadcrumbs[0].offset, 7,
        "the function, after `export `"
    );

    // Offsets are UTF-16: after the emoji they differ from byte offsets.
    let brace_byte = text.find("{\"x\"}").unwrap();
    let brace16 = text[..brace_byte].encode_utf16().count() as u32;
    let info = s.caret_info(doc.doc_id, brace16).await.unwrap();
    let pair = info.brackets.unwrap();
    assert_eq!((pair.open_from, pair.open_to), (brace16, brace16 + 1));
    assert_eq!(pair.close_from, brace16 + 4);

    let folds = s.folds(doc.doc_id).await.unwrap();
    assert_eq!(
        folds.ranges.len(),
        3,
        "function body, return parens, <main>"
    );
}

#[tokio::test]
async fn ignored_entries_and_embedded_languages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root, ".gitignore", "dist\n");
    write(&root, "dist/out.js", "");
    write(&root, "node_modules/x/index.js", "");
    write(
        &root,
        "C.vue",
        "<template><p>hi</p></template>\n<script>\nconst a = 1;\n</script>\n",
    );
    write(&root, "R.md", "# Title\n\n```ts\nlet x = 1;\n```\n");
    let s = session(dir.path()).await;

    let entries = s.list_dir(String::new()).await.unwrap();
    let ignored = |name: &str| entries.iter().find(|e| e.name == name).unwrap().ignored;
    assert!(ignored("dist"));
    assert!(ignored("node_modules"));
    assert!(!ignored("C.vue"));

    for (file, word, class) in [
        ("C.vue", "const", TokenClass::Keyword),
        ("R.md", "let", TokenClass::Keyword),
        ("R.md", "Title", TokenClass::Heading),
    ] {
        let doc = s.open_document(file.into()).await.unwrap();
        assert!(doc.language.is_some(), "{file}");
        let text = fs::read_to_string(root.join(file)).unwrap();
        let at = text.find(word).unwrap() as u32;
        let hl = s.highlights(doc.doc_id, 0, 1000).await.unwrap();
        assert!(
            hl.spans
                .iter()
                .any(|sp| sp.from <= at && at < sp.to && sp.class == class),
            "{file}: {word} should be {class:?}: {:?}",
            hl.spans
        );
    }
}
