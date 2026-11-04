//! Enter indentation and indent-unit detection.
//!
//! Rules (applied per selection range):
//! - the new line keeps the indentation of the current line;
//! - after an opening bracket (`{`, `(`, `[`) or, in markup languages, an
//!   opening tag, the new line is indented one unit deeper;
//! - if the caret sits between such an opener and its closer (`{|}`,
//!   `<div>|</div>`), the closer moves to its own line at the original
//!   indentation;
//! - trailing whitespace before the caret is removed.

use crate::document::{Document, Edit};
use crate::selection::{SelRange, Selection};

/// Language-specific indentation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndentRules {
    /// Indent after opening markup tags (HTML, JSX/TSX).
    pub markup_tags: bool,
}

/// HTML elements that never have a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Detects the indentation unit of a text: a tab, or the most common
/// increase in leading spaces between consecutive non-blank lines. Defaults
/// to two spaces.
pub fn detect_indent_unit(text: &str) -> String {
    let mut tabs = 0usize;
    let mut deltas = [0usize; 9];
    let mut prev = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            prev = 0;
            continue;
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > prev && spaces - prev <= 8 {
            deltas[spaces - prev] += 1;
        }
        prev = spaces;
    }
    let space_lines: usize = deltas.iter().sum();
    if tabs > space_lines {
        return "\t".to_owned();
    }
    let mut best = 2;
    for width in [2, 4, 8, 3, 1, 5, 6, 7] {
        if deltas[width] > deltas[best] {
            best = width;
        }
    }
    " ".repeat(best)
}

fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
}

fn closer_for(open: char) -> Option<char> {
    match open {
        '{' => Some('}'),
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

/// Whether `s` ends with an opening (non-void, non-self-closing) tag.
fn ends_with_open_tag(s: &str) -> bool {
    if !s.ends_with('>') || s.ends_with("/>") {
        return false;
    }
    let Some(lt) = s.rfind('<') else {
        return false;
    };
    let tag = &s[lt + 1..s.len() - 1];
    if tag.starts_with(['/', '!', '?']) || tag.contains('>') {
        return false;
    }
    let name: String = tag
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '.')
        .collect();
    !name.is_empty() && !VOID_ELEMENTS.contains(&name.to_ascii_lowercase().as_str())
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl Document {
    /// Computes the edits for pressing Enter at every selection range, and
    /// the selection afterwards. Edits are relative to the current text.
    pub fn newline_and_indent(
        &self,
        selection: &Selection,
        unit: &str,
        rules: IndentRules,
    ) -> (Vec<Edit>, Selection) {
        let len = self.len_utf16();
        let selection = selection.clone().normalized(len);
        let rope = self.rope();
        let mut edits = Vec::with_capacity(selection.ranges.len());
        let mut carets = Vec::with_capacity(selection.ranges.len());
        let mut delta: isize = 0;
        let mut prev_end = 0usize;

        for range in &selection.ranges {
            let (from, to) = (range.from(), range.to());
            let from_char = rope.utf16_cu_to_char(from);
            let line = rope.char_to_line(from_char);
            let line_start = self.char_to_utf16(rope.line_to_char(line));
            let before = self.text_between(line_start, from);
            let to_line = rope.char_to_line(rope.utf16_cu_to_char(to));
            let to_line_end = if to_line + 1 < rope.len_lines() {
                self.char_to_utf16(rope.line_to_char(to_line + 1) - 1)
            } else {
                len
            };
            let after = self.text_between(to, to_line_end);

            let before_trim = before.trim_end_matches([' ', '\t']);
            let trailing = utf16_len(&before[before_trim.len()..]);
            // On a blank line, keep the whitespace typed so far.
            let base = leading_ws(if before_trim.is_empty() {
                &before
            } else {
                before_trim
            })
            .to_owned();
            let after_trim = after.trim_start_matches([' ', '\t']);

            let bracket_closer = before_trim.chars().last().and_then(closer_for);
            let open_tag = rules.markup_tags && ends_with_open_tag(before_trim);
            let opens = bracket_closer.is_some() || open_tag;
            let inner = if opens {
                format!("{base}{unit}")
            } else {
                base.clone()
            };
            let closes = match bracket_closer {
                Some(closer) => after_trim.starts_with(closer),
                None => open_tag && after_trim.starts_with("</"),
            };

            let edit_from = (from - trailing).max(prev_end);
            let edit_to = if closes {
                to + (utf16_len(&after) - utf16_len(after_trim))
            } else {
                to
            };
            let insert = if closes {
                format!("\n{inner}\n{base}")
            } else {
                format!("\n{inner}")
            };
            let caret = (edit_from as isize + delta) as usize + 1 + utf16_len(&inner);
            delta += utf16_len(&insert) as isize - (edit_to - edit_from) as isize;
            prev_end = edit_to;
            edits.push(Edit::new(edit_from, edit_to, insert));
            carets.push(SelRange::cursor(caret));
        }
        let main = selection.main.min(carets.len().saturating_sub(1));
        (
            edits,
            Selection {
                ranges: carets,
                main,
            },
        )
    }

    fn text_between(&self, from: usize, to: usize) -> String {
        let rope = self.rope();
        let a = rope.utf16_cu_to_char(from.min(self.len_utf16()));
        let b = rope.utf16_cu_to_char(to.min(self.len_utf16()));
        rope.slice(a..b.max(a)).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{EditKind, Transaction};

    /// Applies Enter at `|` in `text` and returns the text with `|` at the caret.
    fn enter(text: &str, rules: IndentRules) -> String {
        let pos = text.find('|').unwrap();
        let clean = text.replace('|', "");
        let mut doc = Document::new(&clean);
        let at = utf16_len(&clean[..pos]);
        let (edits, sel) = doc.newline_and_indent(&Selection::cursor(at), "  ", rules);
        doc.apply(
            0,
            &[Transaction {
                edits,
                kind: EditKind::Typing,
                selection_before: Selection::cursor(at),
                selection_after: sel.clone(),
            }],
            0,
        )
        .unwrap();
        let mut out = doc.text();
        let caret = sel.main_range().head;
        let byte = doc.utf16_to_byte_clamped(caret);
        out.insert(byte, '|');
        out
    }

    const CODE: IndentRules = IndentRules { markup_tags: false };
    const MARKUP: IndentRules = IndentRules { markup_tags: true };

    #[test]
    fn keeps_indentation() {
        assert_eq!(enter("  let a = 1;|", CODE), "  let a = 1;\n  |");
        assert_eq!(enter("\tfoo();|", CODE), "\tfoo();\n\t|");
    }

    #[test]
    fn indents_after_open_bracket_and_splits_pairs() {
        assert_eq!(enter("if (x) {|", CODE), "if (x) {\n  |");
        assert_eq!(enter("  call(|)", CODE), "  call(\n    |\n  )");
        assert_eq!(enter("const a = [|];", CODE), "const a = [\n  |\n];");
        assert_eq!(enter("f({|  })", CODE), "f({\n  |\n})");
    }

    #[test]
    fn removes_trailing_whitespace_before_caret() {
        assert_eq!(enter("  foo();   |", CODE), "  foo();\n  |");
        assert_eq!(enter("    |", CODE), "\n    |");
    }

    #[test]
    fn splits_in_the_middle_of_a_line() {
        assert_eq!(enter("  a|b", CODE), "  a\n  |b");
    }

    #[test]
    fn markup_tags_indent_only_where_enabled() {
        assert_eq!(enter("<div>|</div>", MARKUP), "<div>\n  |\n</div>");
        assert_eq!(
            enter("  <main className=\"app\">|", MARKUP),
            "  <main className=\"app\">\n    |"
        );
        assert_eq!(enter("<img src=\"a\">|", MARKUP), "<img src=\"a\">\n|");
        assert_eq!(enter("<br/>|", MARKUP), "<br/>\n|");
        assert_eq!(enter("</div>|", MARKUP), "</div>\n|");
        assert_eq!(enter("<div>|", CODE), "<div>\n|");
    }

    #[test]
    fn multiple_carets() {
        let mut doc = Document::new("a {\nb {");
        let sel = Selection {
            ranges: vec![SelRange::cursor(3), SelRange::cursor(7)],
            main: 1,
        };
        let (edits, after) = doc.newline_and_indent(&sel, "    ", CODE);
        doc.apply(
            0,
            &[Transaction {
                edits,
                kind: EditKind::Typing,
                selection_before: sel,
                selection_after: after.clone(),
            }],
            0,
        )
        .unwrap();
        assert_eq!(doc.text(), "a {\n    \nb {\n    ");
        assert_eq!(
            after.ranges,
            vec![SelRange::cursor(8), SelRange::cursor(17)]
        );
        assert_eq!(after.main, 1);
    }

    #[test]
    fn detects_indent_unit() {
        assert_eq!(detect_indent_unit("a\n    b\n        c\n    d\n"), "    ");
        assert_eq!(detect_indent_unit("a\n  b\n    c\n"), "  ");
        assert_eq!(detect_indent_unit("a\n\tb\n\t\tc\n"), "\t");
        assert_eq!(detect_indent_unit("no indentation"), "  ");
    }
}
