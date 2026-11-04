//! Text search and replace over whole documents.

use regex::{Regex, RegexBuilder};

/// What to search for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextQuery {
    /// Literal text or regex.
    pub pattern: String,
    /// Interpret `pattern` as a regular expression.
    pub regex: bool,
    /// Match case exactly.
    pub case_sensitive: bool,
    /// Only match whole words.
    pub whole_word: bool,
}

/// Errors from building a query.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The regular expression is invalid.
    #[error("invalid regular expression: {0}")]
    InvalidRegex(String),
}

impl TextQuery {
    /// Compiles the query. `^`/`$` match at line boundaries.
    pub fn compile(&self) -> Result<Regex, SearchError> {
        let body = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        let body = if self.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        RegexBuilder::new(&body)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .size_limit(1 << 22)
            .build()
            .map_err(|err| SearchError::InvalidRegex(err.to_string()))
    }
}

/// One match in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    /// Zero-based line of the match start.
    pub line: usize,
    /// The line's text (without line break), truncated to 400 chars.
    pub line_text: String,
    /// Match start and end in the document (UTF-16 offsets).
    pub start: usize,
    /// Match end (UTF-16, exclusive).
    pub end: usize,
    /// Match start and end within `line_text` (UTF-16), clipped to it.
    pub column_start: usize,
    /// End column within `line_text` (UTF-16, exclusive).
    pub column_end: usize,
    /// Byte range of the match in the document (for replacement previews).
    pub byte_range: std::ops::Range<usize>,
}

const MAX_LINE_CHARS: usize = 400;

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Finds up to `limit` non-empty matches of `re` in `text`.
pub fn find_matches(re: &Regex, text: &str, limit: usize) -> Vec<TextMatch> {
    let mut out = Vec::new();
    // Incremental position tracking: (byte, utf16 offset, line, line start byte).
    let mut byte = 0;
    let mut offset16 = 0;
    let mut line = 0;
    let mut line_start = 0;
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        if out.len() >= limit {
            break;
        }
        let skipped = &text[byte..m.start()];
        offset16 += utf16_len(skipped);
        for (i, _) in skipped.match_indices('\n') {
            line += 1;
            line_start = byte + i + 1;
        }
        byte = m.start();
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |i| line_start + i);
        let full_line = &text[line_start..line_end];
        let line_text: String = full_line.chars().take(MAX_LINE_CHARS).collect();
        let column_start = utf16_len(&text[line_start..m.start()]);
        let column_end = column_start + utf16_len(&text[m.start()..m.end().min(line_end)]);
        let line_len = utf16_len(&line_text);
        out.push(TextMatch {
            line,
            start: offset16,
            end: offset16 + utf16_len(m.as_str()),
            column_start: column_start.min(line_len),
            column_end: column_end.min(line_len),
            line_text,
            byte_range: m.range(),
        });
    }
    out
}

/// The text that replaces `m` (expands `$1`, `${name}` in regex mode).
pub fn replacement_for(
    re: &Regex,
    query: &TextQuery,
    text: &str,
    m: &TextMatch,
    replacement: &str,
) -> String {
    if !query.regex {
        return replacement.to_owned();
    }
    match re.captures_at(text, m.byte_range.start) {
        Some(caps) if caps.get(0).is_some_and(|c| c.range() == m.byte_range) => {
            let mut out = String::new();
            caps.expand(replacement, &mut out);
            out
        }
        _ => replacement.to_owned(),
    }
}

/// A replacement as an edit on the original text (UTF-16 offsets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// Start offset.
    pub from: usize,
    /// End offset.
    pub to: usize,
    /// Replacement text.
    pub insert: String,
}

/// Replaces every match. Returns the new text and the edits (sorted,
/// relative to the original text).
pub fn replace_all(
    re: &Regex,
    query: &TextQuery,
    text: &str,
    replacement: &str,
) -> (String, Vec<Replacement>) {
    let matches = find_matches(re, text, usize::MAX);
    let mut out = String::with_capacity(text.len());
    let mut edits = Vec::with_capacity(matches.len());
    let mut last = 0;
    for m in &matches {
        let insert = replacement_for(re, query, text, m, replacement);
        out.push_str(&text[last..m.byte_range.start]);
        out.push_str(&insert);
        last = m.byte_range.end;
        edits.push(Replacement {
            from: m.start,
            to: m.end,
            insert,
        });
    }
    out.push_str(&text[last..]);
    (out, edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pattern: &str) -> TextQuery {
        TextQuery {
            pattern: pattern.into(),
            ..TextQuery::default()
        }
    }

    #[test]
    fn literal_search_is_case_insensitive_by_default_and_escapes() {
        let text = "a.b A.B axb\nsecond a.b";
        let re = q("a.b").compile().unwrap();
        let found = find_matches(&re, text, 100);
        assert_eq!(found.len(), 3);
        assert_eq!(
            (found[2].line, found[2].column_start, found[2].column_end),
            (1, 7, 10)
        );
        assert_eq!(found[2].line_text, "second a.b");
        let re = TextQuery {
            case_sensitive: true,
            ..q("a.b")
        }
        .compile()
        .unwrap();
        assert_eq!(find_matches(&re, text, 100).len(), 2);
    }

    #[test]
    fn regex_whole_word_and_limits() {
        let text = "count counter recount count";
        let re = TextQuery {
            whole_word: true,
            ..q("count")
        }
        .compile()
        .unwrap();
        let found = find_matches(&re, text, 100);
        assert_eq!(
            found.iter().map(|m| m.start).collect::<Vec<_>>(),
            vec![0, 22]
        );
        let re = TextQuery {
            regex: true,
            ..q(r"c\w+t")
        }
        .compile()
        .unwrap();
        assert_eq!(find_matches(&re, text, 2).len(), 2);
        assert!(
            TextQuery {
                regex: true,
                ..q("(")
            }
            .compile()
            .is_err()
        );
    }

    #[test]
    fn utf16_offsets_and_empty_matches() {
        let text = "😀 é x\nx";
        let re = q("x").compile().unwrap();
        let found = find_matches(&re, text, 10);
        assert_eq!(found[0].start, 5, "😀 is two UTF-16 units");
        assert_eq!(found[0].column_start, 5);
        assert_eq!(found[1].line, 1);
        let re = TextQuery {
            regex: true,
            ..q("^")
        }
        .compile()
        .unwrap();
        assert!(find_matches(&re, text, 10).is_empty());
    }

    #[test]
    fn replace_literal_and_with_groups() {
        let text = "foo(1) foo(22)";
        let query = TextQuery {
            regex: true,
            ..q(r"foo\((\d+)\)")
        };
        let re = query.compile().unwrap();
        let (new, edits) = replace_all(&re, &query, text, "bar[$1]");
        assert_eq!(new, "bar[1] bar[22]");
        assert_eq!(edits[1].from, 7);
        assert_eq!(edits[1].insert, "bar[22]");

        let literal = q("$1");
        let re = literal.compile().unwrap();
        let (new, _) = replace_all(&re, &literal, "pay $1 now", "$2");
        assert_eq!(new, "pay $2 now", "no expansion in literal mode");
    }
}
