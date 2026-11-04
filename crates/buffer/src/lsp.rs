//! Positions and changes in the shape the Language Server Protocol uses
//! with the UTF-16 position encoding (`docs/ARCHITECTURE.md`, 4.7).
//!
//! Document text only contains `\n` line breaks (and the rope counts only
//! those), so lines here are exactly LSP lines.

use ropey::Rope;

use crate::document::Document;

/// A zero-based line and UTF-16 code-unit column, as LSP counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct LinePos {
    /// Zero-based line.
    pub line: usize,
    /// Zero-based column in UTF-16 code units.
    pub character: usize,
}

impl LinePos {
    /// Creates a position.
    pub fn new(line: usize, character: usize) -> Self {
        Self { line, character }
    }
}

/// One applied edit as an LSP incremental content change: `start..end` is
/// relative to the text right before this edit. A list of these, in the
/// order they were produced, is a valid `didChange` content-change list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspChange {
    /// Start of the replaced range.
    pub start: LinePos,
    /// End of the replaced range.
    pub end: LinePos,
    /// Inserted text.
    pub text: String,
}

/// LSP position of a char index in `rope`.
pub(crate) fn line_pos_of_char(rope: &Rope, char_idx: usize) -> LinePos {
    let char_idx = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(char_idx);
    let line_start = rope.char_to_utf16_cu(rope.line_to_char(line));
    LinePos {
        line,
        character: rope.char_to_utf16_cu(char_idx) - line_start,
    }
}

impl Document {
    /// LSP position of a UTF-16 offset (clamped to the document).
    pub fn line_pos(&self, offset: usize) -> LinePos {
        let rope = self.rope();
        line_pos_of_char(rope, rope.utf16_cu_to_char(offset.min(self.len_utf16())))
    }

    /// UTF-16 offset of an LSP position. A line past the end maps to the end
    /// of the document, a column past the end of its line to the end of that
    /// line (before the line break), and a column inside a surrogate pair to
    /// the start of the pair.
    pub fn offset_of(&self, pos: LinePos) -> usize {
        offset_of(self.rope(), pos)
    }
}

/// [`Document::offset_of`] for any rope (e.g. a file that is not open).
pub fn offset_of(rope: &Rope, pos: LinePos) -> usize {
    if pos.line >= rope.len_lines() {
        return rope.len_utf16_cu();
    }
    let line = rope.line(pos.line);
    let mut line_len = line.len_utf16_cu();
    if line.len_chars() > 0 && line.char(line.len_chars() - 1) == '\n' {
        line_len -= 1;
    }
    let line_start = rope.char_to_utf16_cu(rope.line_to_char(pos.line));
    let offset = line_start + pos.character.min(line_len);
    // Round down out of a surrogate pair.
    rope.char_to_utf16_cu(rope.utf16_cu_to_char(offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Edit, EditKind, Transaction};
    use crate::selection::Selection;

    fn apply(doc: &mut Document, edits: Vec<Edit>) -> Vec<LspChange> {
        let v = doc.version();
        doc.apply(
            v,
            &[Transaction {
                edits,
                kind: EditKind::Other,
                selection_before: Selection::cursor(0),
                selection_after: Selection::cursor(0),
            }],
            0,
        )
        .unwrap()
        .lsp_changes
    }

    /// Applies LSP changes to a plain string the way a server would.
    fn apply_lsp(text: &str, changes: &[LspChange]) -> String {
        let mut text = text.to_owned();
        for change in changes {
            let start = string_offset(&text, change.start);
            let end = string_offset(&text, change.end);
            text.replace_range(start..end, &change.text);
        }
        text
    }

    /// Byte offset of an LSP position in a string, computed independently of
    /// the rope (lines split on `\n`, columns counted in UTF-16).
    fn string_offset(text: &str, pos: LinePos) -> usize {
        let mut byte = 0;
        for (i, line) in text.split('\n').enumerate() {
            if i == pos.line {
                let mut units = 0;
                for (b, ch) in line.char_indices() {
                    if units >= pos.character {
                        return byte + b;
                    }
                    units += ch.len_utf16();
                }
                return byte + line.len();
            }
            byte += line.len() + 1;
        }
        text.len()
    }

    fn utf16_len(text: &str) -> usize {
        text.encode_utf16().count()
    }

    #[test]
    fn converts_positions_with_emoji_and_cjk() {
        // "😀" is two UTF-16 units, "漢字" two units in total.
        let doc = Document::new("a😀b\n漢字x\n");
        assert_eq!(doc.line_pos(0), LinePos::new(0, 0));
        assert_eq!(doc.line_pos(3), LinePos::new(0, 3)); // after the emoji
        assert_eq!(doc.line_pos(5), LinePos::new(1, 0)); // start of line 2
        assert_eq!(doc.line_pos(7), LinePos::new(1, 2)); // after 漢字
        assert_eq!(doc.line_pos(9), LinePos::new(2, 0)); // empty last line
        assert_eq!(doc.line_pos(99), LinePos::new(2, 0)); // clamped
        for offset in [0, 1, 3, 4, 5, 6, 7, 8, 9] {
            assert_eq!(
                doc.offset_of(doc.line_pos(offset)),
                offset,
                "offset {offset}"
            );
        }
    }

    #[test]
    fn clamps_out_of_range_positions() {
        let doc = Document::new("a😀b\nxy");
        assert_eq!(
            doc.offset_of(LinePos::new(0, 2)),
            1,
            "inside the surrogate pair"
        );
        assert_eq!(doc.offset_of(LinePos::new(0, 50)), 4, "past the line end");
        assert_eq!(
            doc.offset_of(LinePos::new(1, 50)),
            7,
            "past the last line end"
        );
        assert_eq!(doc.offset_of(LinePos::new(7, 0)), 7, "past the last line");
    }

    #[test]
    fn changes_rebuild_the_text_like_a_server_would() {
        let initial = "const 名前 = \"😀\";\nlet x = 1;\n";
        let mut doc = Document::new(initial);
        let mut server = initial.to_owned();
        let end = utf16_len(initial);
        let steps = [
            // Several edits in one transaction, relative to the same text.
            vec![
                Edit::new(6, 8, "値🎉"),
                Edit::new(11, 15, "\"漢字😀\""),
                Edit::new(16, 16, "\n// 💥 end"),
            ],
            // Delete across a line break and an emoji.
            vec![Edit::new(10, 22, "")],
            // Insert at the start and replace the tail.
            vec![Edit::new(0, 0, "🙂"), Edit::new(20, end - 2, "y")],
        ];
        for edits in steps {
            let changes = apply(&mut doc, edits);
            server = apply_lsp(&server, &changes);
            assert_eq!(server, doc.text());
        }
    }

    #[test]
    fn undo_and_redo_report_changes_too() {
        let mut doc = Document::new("x😀y");
        apply(&mut doc, vec![Edit::new(1, 3, "漢")]);
        let mut server = doc.text();
        let undo = doc.undo(doc.version()).unwrap().unwrap();
        server = apply_lsp(&server, &undo.lsp_changes);
        assert_eq!(server, "x😀y");
        let redo = doc.redo(doc.version()).unwrap().unwrap();
        server = apply_lsp(&server, &redo.lsp_changes);
        assert_eq!(server, "x漢y");
    }
}
