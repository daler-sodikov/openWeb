//! The document: rope, version and transactional edits.

use ropey::Rope;

use crate::history::{Entry, History, UndoOutcome};
use crate::selection::Selection;

/// Replace the UTF-16 range `from..to` with `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Start offset (UTF-16 code units).
    pub from: usize,
    /// End offset (UTF-16 code units), `>= from`.
    pub to: usize,
    /// Inserted text (`\n` line endings).
    pub insert: String,
}

impl Edit {
    /// Creates an edit.
    pub fn new(from: usize, to: usize, insert: impl Into<String>) -> Self {
        Self {
            from,
            to,
            insert: insert.into(),
        }
    }
}

/// What kind of user action produced a transaction; used for undo grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Typing text.
    Typing,
    /// Deleting with Backspace/Delete.
    Deleting,
    /// Anything else (paste, cut, drop, programmatic); never grouped.
    Other,
}

/// One atomic change: non-overlapping edits sorted by position, all relative
/// to the text before the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// The edits.
    pub edits: Vec<Edit>,
    /// Kind of user action.
    pub kind: EditKind,
    /// Selection before the change (restored by undo).
    pub selection_before: Selection,
    /// Selection after the change (restored by redo).
    pub selection_after: Selection,
}

/// A row/column position as used by tree-sitter (column in bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    /// Zero-based line.
    pub row: usize,
    /// Zero-based byte column.
    pub column: usize,
}

/// An applied edit in byte coordinates, in the order it was applied. Feed
/// these to an incremental parser in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteEdit {
    /// Start byte.
    pub start_byte: usize,
    /// End byte before the edit.
    pub old_end_byte: usize,
    /// End byte after the edit.
    pub new_end_byte: usize,
    /// Start position.
    pub start_point: Point,
    /// End position before the edit.
    pub old_end_point: Point,
    /// End position after the edit.
    pub new_end_point: Point,
}

/// Errors from document operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BufferError {
    /// The caller's version is not the document's current version.
    #[error("version mismatch: document is at {actual}, request was based on {expected}")]
    VersionMismatch {
        /// Version the caller based its request on.
        expected: u32,
        /// Current document version.
        actual: u32,
    },
    /// An edit was out of range, unsorted, overlapping or split a surrogate pair.
    #[error("invalid edit: {0}")]
    InvalidEdit(String),
}

/// Result of applying transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Version after the transactions.
    pub version: u32,
    /// Byte edits for incremental parsing, in application order.
    pub byte_edits: Vec<ByteEdit>,
}

/// A text document with version tracking and undo history.
#[derive(Debug, Clone)]
pub struct Document {
    rope: Rope,
    version: u32,
    history: History,
}

impl Document {
    /// Creates a document at version 0. `text` must use `\n` line endings.
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
            history: History::default(),
        }
    }

    /// The underlying rope.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Current version; increases by one per applied transaction (including
    /// each step of an undo or redo).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The full text.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Length in UTF-16 code units.
    pub fn len_utf16(&self) -> usize {
        self.rope.len_utf16_cu()
    }

    /// Converts a UTF-16 offset to a char index, rejecting offsets past the
    /// end or inside a surrogate pair.
    pub fn utf16_to_char(&self, offset: usize) -> Result<usize, BufferError> {
        if offset > self.len_utf16() {
            return Err(BufferError::InvalidEdit(format!(
                "offset {offset} is past the end ({})",
                self.len_utf16()
            )));
        }
        let char_idx = self.rope.utf16_cu_to_char(offset);
        if self.rope.char_to_utf16_cu(char_idx) != offset {
            return Err(BufferError::InvalidEdit(format!(
                "offset {offset} splits a surrogate pair"
            )));
        }
        Ok(char_idx)
    }

    /// Converts a char index to a UTF-16 offset.
    pub fn char_to_utf16(&self, char_idx: usize) -> usize {
        self.rope
            .char_to_utf16_cu(char_idx.min(self.rope.len_chars()))
    }

    /// Converts a UTF-16 offset (clamped to the document) to a byte offset.
    pub fn utf16_to_byte_clamped(&self, offset: usize) -> usize {
        let char_idx = self.rope.utf16_cu_to_char(offset.min(self.len_utf16()));
        self.rope.char_to_byte(char_idx)
    }

    /// Converts a byte offset to a UTF-16 offset.
    pub fn byte_to_utf16(&self, byte: usize) -> usize {
        let char_idx = self.rope.byte_to_char(byte.min(self.rope.len_bytes()));
        self.rope.char_to_utf16_cu(char_idx)
    }

    /// Applies transactions based on `base_version`, recording them in the
    /// undo history. Each transaction is validated before it is applied; on
    /// error, transactions before the failing one stay applied and the
    /// caller must resynchronize from [`Document::text`].
    pub fn apply(
        &mut self,
        base_version: u32,
        transactions: &[Transaction],
        now_ms: u64,
    ) -> Result<ApplyOutcome, BufferError> {
        self.check_version(base_version)?;
        let mut byte_edits = Vec::new();
        for tx in transactions {
            let (inverse, mut edits) = self.apply_edits(&tx.edits)?;
            byte_edits.append(&mut edits);
            self.version += 1;
            self.history.record(Entry {
                forward: tx.edits.clone(),
                inverse,
                selection_before: tx.selection_before.clone(),
                selection_after: tx.selection_after.clone(),
                kind: tx.kind,
                time_ms: now_ms,
            });
        }
        Ok(ApplyOutcome {
            version: self.version,
            byte_edits,
        })
    }

    /// Undoes the last undo group. Returns `None` if there is nothing to undo.
    pub fn undo(&mut self, base_version: u32) -> Result<Option<UndoOutcome>, BufferError> {
        self.check_version(base_version)?;
        let Some(group) = self.history.pop_undo() else {
            return Ok(None);
        };
        let mut updates = Vec::with_capacity(group.len());
        let mut byte_edits = Vec::new();
        for entry in group.iter().rev() {
            let (_, mut edits) = self.apply_edits(&entry.inverse)?;
            byte_edits.append(&mut edits);
            updates.push(entry.inverse.clone());
            self.version += 1;
        }
        let selection = group
            .first()
            .map(|e| e.selection_before.clone())
            .unwrap_or_else(|| Selection::cursor(0));
        self.history.push_redo(group);
        Ok(Some(UndoOutcome {
            version: self.version,
            updates,
            selection,
            byte_edits,
        }))
    }

    /// Redoes the last undone group. Returns `None` if there is nothing to redo.
    pub fn redo(&mut self, base_version: u32) -> Result<Option<UndoOutcome>, BufferError> {
        self.check_version(base_version)?;
        let Some(group) = self.history.pop_redo() else {
            return Ok(None);
        };
        let mut updates = Vec::with_capacity(group.len());
        let mut byte_edits = Vec::new();
        for entry in &group {
            let (_, mut edits) = self.apply_edits(&entry.forward)?;
            byte_edits.append(&mut edits);
            updates.push(entry.forward.clone());
            self.version += 1;
        }
        let selection = group
            .last()
            .map(|e| e.selection_after.clone())
            .unwrap_or_else(|| Selection::cursor(0));
        self.history.push_undo(group);
        Ok(Some(UndoOutcome {
            version: self.version,
            updates,
            selection,
            byte_edits,
        }))
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    fn check_version(&self, base_version: u32) -> Result<(), BufferError> {
        if base_version == self.version {
            Ok(())
        } else {
            Err(BufferError::VersionMismatch {
                expected: base_version,
                actual: self.version,
            })
        }
    }

    /// Validates and applies one set of edits. Returns the inverse edits
    /// (relative to the text after the change) and the byte edits.
    fn apply_edits(&mut self, edits: &[Edit]) -> Result<(Vec<Edit>, Vec<ByteEdit>), BufferError> {
        // Validate and convert everything before mutating.
        let mut char_ranges = Vec::with_capacity(edits.len());
        let mut prev_to = 0;
        for (i, edit) in edits.iter().enumerate() {
            if edit.from > edit.to {
                return Err(BufferError::InvalidEdit(format!(
                    "edit {i}: from {} > to {}",
                    edit.from, edit.to
                )));
            }
            if i > 0 && edit.from < prev_to {
                return Err(BufferError::InvalidEdit(format!(
                    "edit {i} overlaps or is not sorted"
                )));
            }
            prev_to = edit.to;
            char_ranges.push((self.utf16_to_char(edit.from)?, self.utf16_to_char(edit.to)?));
        }

        // Inverse edits, positioned in the resulting text.
        let mut inverse = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for (edit, &(cf, ct)) in edits.iter().zip(&char_ranges) {
            let removed = self.rope.slice(cf..ct).to_string();
            let inserted_len = utf16_len(&edit.insert);
            let new_from = offset_add(edit.from, delta);
            inverse.push(Edit::new(new_from, new_from + inserted_len, removed));
            delta += inserted_len as isize - (edit.to - edit.from) as isize;
        }

        // Apply from the end so earlier offsets stay valid.
        let mut byte_edits = Vec::with_capacity(edits.len());
        for (edit, &(cf, ct)) in edits.iter().zip(&char_ranges).rev() {
            let start_byte = self.rope.char_to_byte(cf);
            let old_end_byte = self.rope.char_to_byte(ct);
            let start_point = self.point_of_byte(start_byte);
            let old_end_point = self.point_of_byte(old_end_byte);
            self.rope.remove(cf..ct);
            self.rope.insert(cf, &edit.insert);
            let new_end_byte = start_byte + edit.insert.len();
            byte_edits.push(ByteEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_point,
                old_end_point,
                new_end_point: self.point_of_byte(new_end_byte),
            });
        }
        Ok((inverse, byte_edits))
    }

    fn point_of_byte(&self, byte: usize) -> Point {
        let row = self.rope.byte_to_line(byte);
        Point {
            row,
            column: byte - self.rope.line_to_byte(row),
        }
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn offset_add(offset: usize, delta: isize) -> usize {
    // Offsets of valid, sorted edits never go negative; saturate defensively.
    offset.saturating_add_signed(delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SelRange;

    fn tx(edits: Vec<Edit>, kind: EditKind, before: usize, after: usize) -> Transaction {
        Transaction {
            edits,
            kind,
            selection_before: Selection::cursor(before),
            selection_after: Selection::cursor(after),
        }
    }

    fn typed(doc: &mut Document, at: usize, s: &str, now: u64) {
        let v = doc.version();
        let len = utf16_len(s);
        doc.apply(
            v,
            &[tx(
                vec![Edit::new(at, at, s)],
                EditKind::Typing,
                at,
                at + len,
            )],
            now,
        )
        .unwrap();
    }

    #[test]
    fn applies_multi_edit_transaction_relative_to_original_text() {
        let mut doc = Document::new("one two three");
        let out = doc
            .apply(
                0,
                &[tx(
                    vec![
                        Edit::new(0, 3, "1"),
                        Edit::new(4, 7, "2"),
                        Edit::new(13, 13, "!"),
                    ],
                    EditKind::Other,
                    0,
                    0,
                )],
                0,
            )
            .unwrap();
        assert_eq!(doc.text(), "1 2 three!");
        assert_eq!(out.version, 1);
        assert_eq!(out.byte_edits.len(), 3);
        // Applied from the end.
        assert_eq!(out.byte_edits[0].start_byte, 13);
        assert_eq!(out.byte_edits[2].start_byte, 0);
    }

    #[test]
    fn rejects_stale_version_and_bad_edits() {
        let mut doc = Document::new("abc");
        assert!(matches!(
            doc.apply(5, &[], 0),
            Err(BufferError::VersionMismatch {
                expected: 5,
                actual: 0
            })
        ));
        for edits in [
            vec![Edit::new(2, 1, "")],
            vec![Edit::new(0, 9, "")],
            vec![Edit::new(1, 2, ""), Edit::new(0, 1, "")],
            vec![Edit::new(0, 2, ""), Edit::new(1, 3, "")],
        ] {
            assert!(matches!(
                doc.apply(0, &[tx(edits, EditKind::Other, 0, 0)], 0),
                Err(BufferError::InvalidEdit(_))
            ));
        }
        assert_eq!(doc.text(), "abc");
        assert_eq!(doc.version(), 0);
    }

    #[test]
    fn utf16_offsets_with_astral_chars() {
        // "a😀b": 😀 is 2 UTF-16 code units and 4 bytes.
        let mut doc = Document::new("a😀b");
        assert_eq!(doc.len_utf16(), 4);
        assert!(doc.utf16_to_char(2).is_err(), "inside surrogate pair");
        assert_eq!(doc.utf16_to_byte_clamped(3), 5);
        assert_eq!(doc.byte_to_utf16(5), 3);
        doc.apply(
            0,
            &[tx(vec![Edit::new(3, 4, "c")], EditKind::Other, 0, 0)],
            0,
        )
        .unwrap();
        assert_eq!(doc.text(), "a😀c");
    }

    #[test]
    fn byte_edits_carry_points() {
        let mut doc = Document::new("ab\ncd\n");
        let out = doc
            .apply(
                0,
                &[tx(vec![Edit::new(4, 5, "XY\nZ")], EditKind::Other, 0, 0)],
                0,
            )
            .unwrap();
        assert_eq!(doc.text(), "ab\ncXY\nZ\n");
        let e = out.byte_edits[0];
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (4, 5, 8));
        assert_eq!(e.start_point, Point { row: 1, column: 1 });
        assert_eq!(e.old_end_point, Point { row: 1, column: 2 });
        assert_eq!(e.new_end_point, Point { row: 2, column: 1 });
    }

    #[test]
    fn undo_redo_round_trip_with_selection() {
        let mut doc = Document::new("hello");
        let sel_after = Selection {
            ranges: vec![SelRange::cursor(1), SelRange::cursor(7)],
            main: 0,
        };
        doc.apply(
            0,
            &[Transaction {
                edits: vec![Edit::new(0, 0, "X"), Edit::new(5, 5, "Y")],
                kind: EditKind::Other,
                selection_before: Selection::cursor(0),
                selection_after: sel_after.clone(),
            }],
            0,
        )
        .unwrap();
        assert_eq!(doc.text(), "XhelloY");

        let undo = doc.undo(1).unwrap().unwrap();
        assert_eq!(doc.text(), "hello");
        assert_eq!(undo.version, 2);
        assert_eq!(
            undo.updates,
            vec![vec![Edit::new(0, 1, ""), Edit::new(6, 7, "")]]
        );
        assert_eq!(undo.selection, Selection::cursor(0));

        let redo = doc.redo(2).unwrap().unwrap();
        assert_eq!(doc.text(), "XhelloY");
        assert_eq!(redo.version, 3);
        assert_eq!(redo.selection, sel_after);
        assert!(doc.redo(3).unwrap().is_none());
    }

    #[test]
    fn typing_is_grouped_until_pause_or_kind_change() {
        let mut doc = Document::new("");
        typed(&mut doc, 0, "a", 0);
        typed(&mut doc, 1, "b", 100);
        typed(&mut doc, 2, "c", 200);
        // Pause > 500 ms starts a new group.
        typed(&mut doc, 3, "d", 900);
        assert_eq!(doc.text(), "abcd");

        let v = doc.version();
        let undo = doc.undo(v).unwrap().unwrap();
        assert_eq!(doc.text(), "abc");
        assert_eq!(undo.updates.len(), 1);

        let undo = doc.undo(doc.version()).unwrap().unwrap();
        assert_eq!(doc.text(), "");
        assert_eq!(
            undo.updates.len(),
            3,
            "three typed chars undone as one group"
        );
        assert_eq!(undo.selection, Selection::cursor(0));
        assert!(doc.undo(doc.version()).unwrap().is_none());

        doc.redo(doc.version()).unwrap();
        assert_eq!(doc.text(), "abc");
    }

    #[test]
    fn caret_jump_breaks_group_and_new_edit_clears_redo() {
        let mut doc = Document::new("xy");
        typed(&mut doc, 0, "a", 0);
        // Caret moved elsewhere before typing again: selection_before != previous selection_after.
        typed(&mut doc, 3, "b", 10);
        doc.undo(doc.version()).unwrap();
        assert_eq!(doc.text(), "axy");
        assert!(doc.can_redo());
        typed(&mut doc, 1, "c", 20);
        assert!(!doc.can_redo());
    }

    #[test]
    fn many_random_edits_undo_to_original() {
        let original = "fn main() {\n    let x = 1;\n}\n";
        let mut doc = Document::new(original);
        let mut seed: u64 = 42;
        let mut next = |m: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % m.max(1)
        };
        for i in 0..200 {
            // Pick char boundaries, then convert to UTF-16 offsets.
            let chars = doc.rope().len_chars();
            let ca = next(chars + 1);
            let cb = (ca + next(4)).min(chars);
            let (a, b) = (doc.char_to_utf16(ca), doc.char_to_utf16(cb));
            let insert = ["", "é", "😀", "\n", "abc"][next(5)];
            doc.apply(
                doc.version(),
                &[tx(vec![Edit::new(a, b, insert)], EditKind::Other, 0, 0)],
                i * 1000,
            )
            .unwrap();
        }
        while doc.can_undo() {
            doc.undo(doc.version()).unwrap();
        }
        assert_eq!(doc.text(), original);
        while doc.can_redo() {
            doc.redo(doc.version()).unwrap();
        }
        assert_ne!(doc.text(), original);
    }
}
