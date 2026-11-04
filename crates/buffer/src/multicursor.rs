//! Multi-cursor selection commands (IntelliJ-style "clone caret",
//! "select next occurrence", "select all occurrences").

use crate::document::Document;
use crate::selection::{SelRange, Selection};

/// Direction for cloning a caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretDirection {
    /// One line up.
    Up,
    /// One line down.
    Down,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl Document {
    /// Adds a caret one line above the topmost caret (or below the bottommost
    /// one), at the same UTF-16 column clamped to the line length. The new
    /// caret becomes the main range. Unchanged at the first/last line.
    pub fn add_caret(&self, selection: &Selection, direction: CaretDirection) -> Selection {
        let len = self.len_utf16();
        let selection = selection.clone().normalized(len);
        let rope = self.rope();
        let line_of = |pos: usize| rope.char_to_line(rope.utf16_cu_to_char(pos));
        let edge = match direction {
            CaretDirection::Up => selection.ranges.iter().min_by_key(|r| line_of(r.head)),
            CaretDirection::Down => selection.ranges.iter().max_by_key(|r| line_of(r.head)),
        };
        let Some(edge) = edge.copied() else {
            return selection;
        };
        let line = line_of(edge.head);
        let target = match direction {
            CaretDirection::Up if line > 0 => line - 1,
            CaretDirection::Down if line + 1 < rope.len_lines() => line + 1,
            _ => return selection,
        };
        let column = edge.head - self.line_start_utf16(line);
        let pos = self.line_start_utf16(target) + column.min(self.line_len_utf16(target));
        let mut ranges = selection.ranges;
        ranges.push(SelRange::cursor(pos));
        let main = ranges.len() - 1;
        Selection { ranges, main }.normalized(len)
    }

    /// With an empty main range: selects the word at the caret. Otherwise adds
    /// the next occurrence (wrapping around) of the main range's text that is
    /// not already selected, and makes it the main range.
    pub fn select_next_occurrence(&self, selection: &Selection) -> Selection {
        let len = self.len_utf16();
        let selection = selection.clone().normalized(len);
        let main = selection.main_range();
        if main.is_empty() {
            return match self.word_at(main.head) {
                Some(word) => {
                    let mut ranges = selection.ranges.clone();
                    ranges[selection.main] = word;
                    Selection {
                        ranges,
                        main: selection.main,
                    }
                    .normalized(len)
                }
                None => selection,
            };
        }
        let text = self.text();
        let needle = self.slice_utf16(main.from(), main.to());
        if needle.is_empty() {
            return selection;
        }
        let start_byte = self.utf16_to_byte_clamped(main.to());
        let candidates = text[start_byte..]
            .match_indices(needle.as_str())
            .map(|(i, _)| i + start_byte)
            .chain(
                text[..start_byte]
                    .match_indices(needle.as_str())
                    .map(|(i, _)| i),
            );
        for byte in candidates {
            let from = self.byte_to_utf16(byte);
            let to = self.byte_to_utf16(byte + needle.len());
            let already = selection
                .ranges
                .iter()
                .any(|r| r.from() == from && r.to() == to);
            if !already {
                let mut ranges = selection.ranges.clone();
                ranges.push(SelRange::new(from, to));
                let main = ranges.len() - 1;
                return Selection { ranges, main }.normalized(len);
            }
        }
        selection
    }

    /// Selects every occurrence of the main range's text (or of the word at
    /// the caret). The occurrence at the original main range stays main.
    pub fn select_all_occurrences(&self, selection: &Selection) -> Selection {
        let len = self.len_utf16();
        let selection = selection.clone().normalized(len);
        let main = selection.main_range();
        let target = if main.is_empty() {
            match self.word_at(main.head) {
                Some(word) => word,
                None => return selection,
            }
        } else {
            main
        };
        let needle = self.slice_utf16(target.from(), target.to());
        if needle.is_empty() {
            return selection;
        }
        let text = self.text();
        let mut ranges = Vec::new();
        let mut main_index = 0;
        for (byte, _) in text.match_indices(needle.as_str()) {
            let from = self.byte_to_utf16(byte);
            let to = self.byte_to_utf16(byte + needle.len());
            if from == target.from() {
                main_index = ranges.len();
            }
            ranges.push(SelRange::new(from, to));
        }
        Selection {
            ranges,
            main: main_index,
        }
        .normalized(len)
    }

    /// Range of the word touching `pos` (UTF-16), if any.
    fn word_at(&self, pos: usize) -> Option<SelRange> {
        let rope = self.rope();
        let char_idx = rope.utf16_cu_to_char(pos.min(self.len_utf16()));
        let mut start = char_idx;
        while start > 0 && is_word_char(rope.char(start - 1)) {
            start -= 1;
        }
        let mut end = char_idx;
        while end < rope.len_chars() && is_word_char(rope.char(end)) {
            end += 1;
        }
        (start < end).then(|| SelRange::new(self.char_to_utf16(start), self.char_to_utf16(end)))
    }

    fn slice_utf16(&self, from: usize, to: usize) -> String {
        let rope = self.rope();
        let from = rope.utf16_cu_to_char(from.min(self.len_utf16()));
        let to = rope.utf16_cu_to_char(to.min(self.len_utf16()));
        rope.slice(from..to).to_string()
    }

    fn line_start_utf16(&self, line: usize) -> usize {
        self.char_to_utf16(self.rope().line_to_char(line))
    }

    /// Line length in UTF-16 units, excluding the line break.
    fn line_len_utf16(&self, line: usize) -> usize {
        let rope = self.rope();
        let start = rope.line_to_char(line);
        let end = if line + 1 < rope.len_lines() {
            rope.line_to_char(line + 1) - 1
        } else {
            rope.len_chars()
        };
        self.char_to_utf16(end) - self.char_to_utf16(start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(sel: &Selection) -> Vec<(usize, usize)> {
        sel.ranges.iter().map(|r| (r.anchor, r.head)).collect()
    }

    #[test]
    fn clone_caret_up_and_down_clamps_column() {
        let doc = Document::new("abcdef\nab\nabcdef\n");
        // Caret at column 5 of line 2 (offset 10 + 5).
        let sel = doc.add_caret(&Selection::cursor(15), CaretDirection::Up);
        assert_eq!(
            ranges(&sel),
            vec![(9, 9), (15, 15)],
            "clamped to end of 'ab'"
        );
        assert_eq!(sel.main, 0);

        let sel = doc.add_caret(&sel, CaretDirection::Up);
        assert_eq!(ranges(&sel), vec![(2, 2), (9, 9), (15, 15)]);

        // Already at the first line: unchanged.
        let again = doc.add_caret(&sel, CaretDirection::Up);
        assert_eq!(again.ranges, sel.ranges);

        let down = doc.add_caret(&Selection::cursor(3), CaretDirection::Down);
        assert_eq!(ranges(&down), vec![(3, 3), (9, 9)]);
    }

    #[test]
    fn select_next_occurrence_selects_word_then_adds_matches() {
        let doc = Document::new("let foo = foo + bar(foo);");
        let sel = doc.select_next_occurrence(&Selection::cursor(5));
        assert_eq!(ranges(&sel), vec![(4, 7)]);
        let sel = doc.select_next_occurrence(&sel);
        assert_eq!(ranges(&sel), vec![(4, 7), (10, 13)]);
        assert_eq!(sel.main, 1);
        let sel = doc.select_next_occurrence(&sel);
        assert_eq!(ranges(&sel), vec![(4, 7), (10, 13), (20, 23)]);
        // All occurrences selected: unchanged.
        let same = doc.select_next_occurrence(&sel);
        assert_eq!(same.ranges, sel.ranges);
    }

    #[test]
    fn select_next_occurrence_wraps_around() {
        let doc = Document::new("ab ab ab");
        let sel = Selection::single(SelRange::new(6, 8));
        let sel = doc.select_next_occurrence(&sel);
        assert_eq!(ranges(&sel), vec![(0, 2), (6, 8)]);
        assert_eq!(sel.main, 0);
    }

    #[test]
    fn select_all_occurrences_with_unicode() {
        let doc = Document::new("é😀x é😀x\né😀x");
        let sel = doc.select_all_occurrences(&Selection::single(SelRange::new(5, 9)));
        assert_eq!(ranges(&sel), vec![(0, 4), (5, 9), (10, 14)]);
        assert_eq!(sel.main, 1);
    }

    #[test]
    fn select_all_from_caret_in_whitespace_is_noop() {
        let doc = Document::new("a  b");
        let sel = doc.select_all_occurrences(&Selection::cursor(2));
        assert_eq!(sel, Selection::cursor(2));
    }
}
