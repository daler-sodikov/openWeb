//! Workspace edits from the language server (`docs/ARCHITECTURE.md`, 4.8):
//! per file, either UTF-16 edits for an open document (the view applies them
//! as one undo step) or an atomic rewrite of a closed file. Files that
//! changed since a preview, and library files, are skipped and reported.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrule_buffer::{Document, LinePos, Rope, offset_of};
use ferrule_ipc as ipc;
use ferrule_lsp_client::lsp_types::{
    self as lsp, DocumentChangeOperation, DocumentChanges, OneOf, TextDocumentEdit, WorkspaceEdit,
};
use ferrule_lsp_client::uri_to_path;

use crate::convert::to_u32;
use crate::session::{DocHandle, Session};

/// The text edits of one file.
pub(crate) struct FileEdits {
    pub path: PathBuf,
    pub edits: Vec<lsp::TextEdit>,
}

/// What a file looked like when a preview was computed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Expect {
    /// An open document at this version.
    Version(u32),
    /// A closed file with this content hash.
    Content(u64),
}

/// Text edits per file; resource operations (create/rename/delete) are not
/// supported and reported as skipped.
pub(crate) fn collect(edit: &WorkspaceEdit) -> (Vec<FileEdits>, Vec<ipc::SkippedFile>) {
    let mut files: Vec<FileEdits> = Vec::new();
    let mut skipped = Vec::new();
    let mut add = |uri: &lsp::Uri, edits: Vec<lsp::TextEdit>| {
        let Some(path) = uri_to_path(uri) else {
            return;
        };
        match files.iter_mut().find(|f| f.path == path) {
            Some(file) => file.edits.extend(edits),
            None => files.push(FileEdits { path, edits }),
        }
    };
    let text_edits = |doc: &TextDocumentEdit| -> Vec<lsp::TextEdit> {
        doc.edits
            .iter()
            .map(|e| match e {
                OneOf::Left(edit) => edit.clone(),
                OneOf::Right(annotated) => annotated.text_edit.clone(),
            })
            .collect()
    };
    match &edit.document_changes {
        Some(DocumentChanges::Edits(docs)) => {
            for doc in docs {
                add(&doc.text_document.uri, text_edits(doc));
            }
        }
        Some(DocumentChanges::Operations(ops)) => {
            for op in ops {
                match op {
                    DocumentChangeOperation::Edit(doc) => {
                        add(&doc.text_document.uri, text_edits(doc))
                    }
                    DocumentChangeOperation::Op(op) => skipped.push(ipc::SkippedFile {
                        path: format!("{op:?}"),
                        reason: "creating, renaming or deleting files is not supported yet".into(),
                    }),
                }
            }
        }
        None => {
            if let Some(changes) = &edit.changes {
                for (uri, edits) in changes {
                    add(uri, edits.clone());
                }
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, skipped)
}

fn content_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Whether a file is library code the editor never changes.
pub(crate) fn is_library(session: &Session, path: &Path) -> bool {
    match session.relative_path(path) {
        Some(rel) => rel.split('/').any(|part| part == "node_modules"),
        None => true,
    }
}

/// Records what every file looks like now (for applying after a preview).
pub(crate) fn snapshot(
    session: &Session,
    files: &[FileEdits],
) -> Result<HashMap<PathBuf, Expect>, ipc::IpcError> {
    let mut expected = HashMap::new();
    for file in files {
        let expect = match session.open_doc_at(&file.path)? {
            Some(doc) => Some(Expect::Version(doc.lock()?.doc.version())),
            None => ferrule_vfs::read_text(&file.path)
                .ok()
                .map(|f| Expect::Content(content_hash(&f.text))),
        };
        if let Some(expect) = expect {
            expected.insert(file.path.clone(), expect);
        }
    }
    Ok(expected)
}

/// LSP edits as sorted UTF-16 edits on `doc` (newlines normalized to `\n`).
pub(crate) fn doc_edits(
    doc: &Document,
    edits: &[lsp::TextEdit],
) -> Result<Vec<ipc::TextEdit>, String> {
    to_offsets(doc.rope(), edits).map(|edits| {
        edits
            .into_iter()
            .map(|(from, to, insert)| ipc::TextEdit {
                from: to_u32(from),
                to: to_u32(to),
                insert,
            })
            .collect()
    })
}

fn position(rope: &Rope, p: lsp::Position) -> usize {
    offset_of(rope, LinePos::new(p.line as usize, p.character as usize))
}

/// Sorted, checked `(from, to, text)` UTF-16 edits.
fn to_offsets(rope: &Rope, edits: &[lsp::TextEdit]) -> Result<Vec<(usize, usize, String)>, String> {
    let mut out: Vec<(usize, usize, String)> = edits
        .iter()
        .map(|e| {
            let from = position(rope, e.range.start);
            let to = position(rope, e.range.end).max(from);
            let text = e.new_text.replace("\r\n", "\n").replace('\r', "\n");
            (from, to, text)
        })
        .collect();
    // Stable: edits inserting at the same offset keep the server's order.
    out.sort_by_key(|(from, to, _)| (*from, *to));
    for pair in out.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err("the server sent overlapping edits".into());
        }
    }
    // Merge inserts at the same position so the view gets disjoint edits.
    let mut merged: Vec<(usize, usize, String)> = Vec::with_capacity(out.len());
    for (from, to, text) in out {
        match merged.last_mut() {
            Some(last) if last.1 == from && last.0 == last.1 && from == to => {
                last.2.push_str(&text)
            }
            _ => merged.push((from, to, text)),
        }
    }
    Ok(merged)
}

/// Applies LSP edits to a whole text.
pub(crate) fn apply_to_text(text: &str, edits: &[lsp::TextEdit]) -> Result<String, String> {
    let rope = Rope::from_str(text);
    let offsets = to_offsets(&rope, edits)?;
    let mut result = text.to_owned();
    for (from, to, insert) in offsets.into_iter().rev() {
        let start = rope.char_to_byte(rope.utf16_cu_to_char(from));
        let end = rope.char_to_byte(rope.utf16_cu_to_char(to));
        result.replace_range(start..end, &insert);
    }
    Ok(result)
}

/// Applies a workspace edit: open documents get edits for the view, closed
/// files are rewritten. Library files and files that differ from
/// `expected` (when given) are skipped.
pub(crate) fn apply(
    session: &Session,
    files: Vec<FileEdits>,
    expected: Option<&HashMap<PathBuf, Expect>>,
    mut skipped: Vec<ipc::SkippedFile>,
) -> ipc::WorkspaceEditResult {
    let mut open_documents = Vec::new();
    let mut files_written = Vec::new();
    for file in files {
        let display = session
            .relative_path(&file.path)
            .unwrap_or_else(|| file.path.to_string_lossy().into_owned());
        let skip = |reason: &str| ipc::SkippedFile {
            path: display.clone(),
            reason: reason.to_owned(),
        };
        if is_library(session, &file.path) {
            skipped.push(skip(
                "library code (outside the project or in node_modules)",
            ));
            continue;
        }
        let expect = expected.and_then(|e| e.get(&file.path)).copied();
        let outcome = match session.open_doc_at(&file.path) {
            Ok(Some(doc)) => {
                open_edits(&doc, &file, expect).map(|edits| open_documents.push(edits))
            }
            Ok(None) => write_closed(&file, expect).map(|()| files_written.push(display.clone())),
            Err(error) => Err(error.message),
        };
        if let Err(reason) = outcome {
            skipped.push(skip(&reason));
        }
    }
    ipc::WorkspaceEditResult {
        open_documents,
        files_written,
        skipped,
    }
}

fn open_edits(
    doc: &Arc<DocHandle>,
    file: &FileEdits,
    expect: Option<Expect>,
) -> Result<ipc::DocumentReplace, String> {
    let state = doc.lock().map_err(|e| e.message)?;
    let version = state.doc.version();
    if let Some(expected) = expect
        && expected != Expect::Version(version)
    {
        return Err("it was edited since the preview".into());
    }
    Ok(ipc::DocumentReplace {
        doc_id: doc.id,
        version,
        edits: doc_edits(&state.doc, &file.edits)?,
    })
}

fn write_closed(file: &FileEdits, expect: Option<Expect>) -> Result<(), String> {
    let current = ferrule_vfs::read_text(&file.path).map_err(|e| e.to_string())?;
    if let Some(expected) = expect
        && expected != Expect::Content(content_hash(&current.text))
    {
        return Err("it changed on disk since the preview".into());
    }
    let text = apply_to_text(&current.text, &file.edits)?;
    let bytes = ferrule_vfs::encode(&text, current.line_ending, current.bom);
    ferrule_vfs::write_atomic(&file.path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> lsp::TextEdit {
        lsp::TextEdit {
            range: lsp::Range::new(lsp::Position::new(sl, sc), lsp::Position::new(el, ec)),
            new_text: text.into(),
        }
    }

    #[test]
    fn applies_edits_with_emoji_and_cjk_in_any_order() {
        let text = "const 名前 = \"😀\";\nuse(名前);\n";
        let edits = [
            edit(1, 4, 1, 6, "値"),
            edit(0, 6, 0, 8, "値"),
            edit(1, 0, 1, 0, "// 🎉\r\n"),
        ];
        assert_eq!(
            apply_to_text(text, &edits).unwrap(),
            "const 値 = \"😀\";\n// 🎉\nuse(値);\n"
        );
    }

    #[test]
    fn rejects_overlaps_and_merges_inserts_at_one_position() {
        let rope = Rope::from_str("abcdef");
        assert!(to_offsets(&rope, &[edit(0, 1, 0, 4, "x"), edit(0, 2, 0, 5, "y")]).is_err());
        let merged = to_offsets(&rope, &[edit(0, 3, 0, 3, "a"), edit(0, 3, 0, 3, "b")]).unwrap();
        assert_eq!(merged, [(3, 3, "ab".to_owned())]);
    }
}
