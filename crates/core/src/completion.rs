//! Completion sessions (`docs/ARCHITECTURE.md`, 4.8): the server's items
//! are kept in Rust, ranked with the fuzzy matcher as the user types, and
//! turned into edits (snippets expanded, auto-imports added) on accept.

use ferrule_buffer::Document;
use ferrule_ipc as ipc;
use ferrule_lsp_client::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemTag, CompletionTextEdit, InsertTextFormat,
};
use ferrule_search::fuzzy_match;

use crate::convert::to_u32;
use crate::edits::doc_edits;

/// Items shown at most (the popup is virtualized, ranking is cheap).
pub(crate) const PAGE: usize = 200;

/// One completion request's items, kept until the next request.
pub(crate) struct CompletionSession {
    pub id: u32,
    pub doc: ipc::DocId,
    /// Start of the typed prefix (UTF-16, at the request's version).
    pub from: usize,
    pub items: Vec<CompletionItem>,
    /// Whether an item was resolved already (details, auto-import edits).
    pub resolved: Vec<bool>,
}

fn kind_name(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::METHOD) => "method",
        Some(CompletionItemKind::FUNCTION) => "function",
        Some(CompletionItemKind::CONSTRUCTOR) => "constructor",
        Some(CompletionItemKind::FIELD) => "field",
        Some(CompletionItemKind::VARIABLE) => "variable",
        Some(CompletionItemKind::CLASS) => "class",
        Some(CompletionItemKind::INTERFACE) => "interface",
        Some(CompletionItemKind::MODULE) => "module",
        Some(CompletionItemKind::PROPERTY) => "property",
        Some(CompletionItemKind::ENUM | CompletionItemKind::ENUM_MEMBER) => "enum",
        Some(CompletionItemKind::KEYWORD) => "keyword",
        Some(CompletionItemKind::SNIPPET) => "snippet",
        Some(CompletionItemKind::CONSTANT) => "constant",
        Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER) => "file",
        Some(CompletionItemKind::TYPE_PARAMETER) => "type",
        _ => "text",
    }
}

/// Items matching `prefix`, best first: exact label, fuzzy score, then the
/// server's `preselect`, `sortText`, and shorter labels.
pub(crate) fn rank(items: &[CompletionItem], prefix: &str) -> (Vec<ipc::CompletionItemView>, u32) {
    let prefix: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
    let mut scored: Vec<(bool, i64, usize, Vec<usize>)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let filter = item.filter_text.as_deref().unwrap_or(&item.label);
            let m = fuzzy_match(&prefix, filter)?;
            // Highlight on the label (it may differ from the filter text).
            let positions = if filter == item.label {
                m.positions
            } else {
                fuzzy_match(&prefix, &item.label)
                    .map(|l| l.positions)
                    .unwrap_or_default()
            };
            let exact = item.label.to_lowercase() == prefix.to_lowercase();
            Some((exact, m.score, index, positions))
        })
        .collect();
    scored.sort_by(|a, b| {
        let (ia, ib) = (&items[a.2], &items[b.2]);
        let sort = |i: &CompletionItem| i.sort_text.clone().unwrap_or_else(|| i.label.clone());
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| {
                ib.preselect
                    .unwrap_or(false)
                    .cmp(&ia.preselect.unwrap_or(false))
            })
            .then_with(|| sort(ia).cmp(&sort(ib)))
            .then_with(|| ia.label.len().cmp(&ib.label.len()))
    });
    let total = to_u32(scored.len());
    let views = scored
        .into_iter()
        .take(PAGE)
        .map(|(_, _, index, positions)| {
            let item = &items[index];
            let deprecated = item.deprecated.unwrap_or(false)
                || item
                    .tags
                    .as_ref()
                    .is_some_and(|t| t.contains(&CompletionItemTag::DEPRECATED));
            ipc::CompletionItemView {
                index: to_u32(index),
                label: item.label.clone(),
                kind: kind_name(item.kind).to_owned(),
                detail: item
                    .label_details
                    .as_ref()
                    .and_then(|d| d.description.clone())
                    .or_else(|| item.detail.clone()),
                matched: positions.into_iter().map(to_u32).collect(),
                deprecated,
            }
        })
        .collect();
    (views, total)
}

/// A snippet with its placeholders removed: the text and its tab stops
/// (each a list of linked UTF-16 ranges in the text; `$0` last).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snippet {
    pub text: String,
    pub stops: Vec<Vec<(usize, usize)>>,
}

/// Parses LSP snippet syntax: `$1`, `${1}`, `${1:placeholder}` (nested),
/// `${1|one,two|}` (first choice), `$name`/`${name:default}` (default or
/// nothing), `\$`, `\}` and `\\`. `$0` (or the end) is the final stop.
pub(crate) fn parse_snippet(source: &str) -> Snippet {
    let chars: Vec<char> = source.chars().collect();
    let mut text = String::new();
    let mut stops: Vec<(u32, (usize, usize))> = Vec::new();
    parse_into(&chars, &mut 0, &mut text, &mut stops, false);
    let mut numbers: Vec<u32> = stops.iter().map(|(n, _)| *n).collect();
    numbers.sort_unstable();
    numbers.dedup();
    // $0 goes last; the others in ascending order.
    if numbers.first() == Some(&0) {
        numbers.remove(0);
        numbers.push(0);
    }
    let mut ordered: Vec<Vec<(usize, usize)>> = numbers
        .iter()
        .map(|n| {
            stops
                .iter()
                .filter(|(m, _)| m == n)
                .map(|(_, r)| *r)
                .collect()
        })
        .collect();
    if !numbers.contains(&0) {
        let end = text.encode_utf16().count();
        ordered.push(vec![(end, end)]);
    }
    Snippet {
        text,
        stops: ordered,
    }
}

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Parses until the end or, inside a placeholder, an unescaped `}`.
fn parse_into(
    chars: &[char],
    at: &mut usize,
    text: &mut String,
    stops: &mut Vec<(u32, (usize, usize))>,
    nested: bool,
) {
    while let Some(&c) = chars.get(*at) {
        match c {
            '\\' if matches!(chars.get(*at + 1), Some('$' | '}' | '\\' | ',' | '|')) => {
                text.push(chars[*at + 1]);
                *at += 2;
            }
            '}' if nested => return,
            '$' => {
                *at += 1;
                parse_dollar(chars, at, text, stops);
            }
            _ => {
                text.push(c);
                *at += 1;
            }
        }
    }
}

fn read_number(chars: &[char], at: &mut usize) -> Option<u32> {
    let start = *at;
    while chars.get(*at).is_some_and(char::is_ascii_digit) {
        *at += 1;
    }
    chars[start..*at].iter().collect::<String>().parse().ok()
}

fn parse_dollar(
    chars: &[char],
    at: &mut usize,
    text: &mut String,
    stops: &mut Vec<(u32, (usize, usize))>,
) {
    match chars.get(*at) {
        Some(c) if c.is_ascii_digit() => {
            if let Some(n) = read_number(chars, at) {
                let pos = utf16(text);
                stops.push((n, (pos, pos)));
            }
        }
        Some('{') => {
            *at += 1;
            let start = utf16(text);
            if let Some(n) = read_number(chars, at) {
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        parse_into(chars, at, text, stops, true);
                    }
                    Some('|') => {
                        *at += 1;
                        let mut first = String::new();
                        let mut done_first = false;
                        while let Some(&c) = chars.get(*at) {
                            *at += 1;
                            match c {
                                '|' => break,
                                ',' => done_first = true,
                                '\\' => {
                                    if let Some(&next) = chars.get(*at) {
                                        if !done_first {
                                            first.push(next);
                                        }
                                        *at += 1;
                                    }
                                }
                                _ if !done_first => first.push(c),
                                _ => {}
                            }
                        }
                        text.push_str(&first);
                    }
                    _ => {}
                }
                stops.push((n, (start, utf16(text))));
            } else {
                // A variable: `${name}` or `${name:default}`.
                while chars
                    .get(*at)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    *at += 1;
                }
                if chars.get(*at) == Some(&':') {
                    *at += 1;
                    parse_into(chars, at, text, stops, true);
                }
            }
            // Skip the closing brace.
            if chars.get(*at) == Some(&'}') {
                *at += 1;
            }
        }
        Some(c) if c.is_alphabetic() || *c == '_' => {
            // `$name`: an unknown variable resolves to nothing.
            while chars
                .get(*at)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                *at += 1;
            }
        }
        _ => text.push('$'),
    }
}

/// The edits for accepting `item` with the caret at `caret`: the main edit
/// from the item's range start (or the session prefix start) to the caret,
/// plus its additional edits (auto-imports) unless they overlap. Snippet
/// stops and the caret are positions in the text after the edits.
pub(crate) fn accept_edit(
    doc: &Document,
    item: &CompletionItem,
    prefix_from: usize,
    caret: usize,
) -> Result<ipc::DocumentEdit, String> {
    let (start, new_text) = match &item.text_edit {
        Some(CompletionTextEdit::Edit(edit)) => (
            doc.offset_of(ferrule_buffer::LinePos::new(
                edit.range.start.line as usize,
                edit.range.start.character as usize,
            )),
            edit.new_text.clone(),
        ),
        Some(CompletionTextEdit::InsertAndReplace(edit)) => (
            doc.offset_of(ferrule_buffer::LinePos::new(
                edit.insert.start.line as usize,
                edit.insert.start.character as usize,
            )),
            edit.new_text.clone(),
        ),
        None => (
            prefix_from,
            item.insert_text
                .clone()
                .unwrap_or_else(|| item.label.clone()),
        ),
    };
    let start = start.min(caret);
    let snippet = (item.insert_text_format == Some(InsertTextFormat::SNIPPET))
        .then(|| parse_snippet(&new_text));
    let inserted = snippet
        .as_ref()
        .map_or_else(|| new_text.replace("\r\n", "\n"), |s| s.text.clone());
    let main = ipc::TextEdit {
        from: to_u32(start),
        to: to_u32(caret),
        insert: inserted.clone(),
    };
    let mut edits = vec![main.clone()];
    if let Some(extra) = &item.additional_text_edits {
        let extra = doc_edits(doc, extra)?;
        let overlaps = extra.iter().any(|e| e.from < main.to && main.from < e.to);
        if !overlaps {
            edits.extend(extra);
        }
    }
    edits.sort_by_key(|e| (e.from, e.to));
    // Where the main edit's text starts after all edits are applied.
    let shift: i64 = edits
        .iter()
        .filter(|e| e.from < main.from)
        .map(|e| utf16(&e.insert) as i64 - i64::from(e.to - e.from))
        .sum();
    let base = usize::try_from(i64::from(main.from) + shift).unwrap_or(0);
    let stops = snippet
        .map(|s| {
            s.stops
                .iter()
                .map(|ranges| {
                    ranges
                        .iter()
                        .map(|&(a, b)| ipc::TextRange {
                            from: to_u32(base + a),
                            to: to_u32(base + b),
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ipc::DocumentEdit {
        version: doc.version(),
        caret: Some(to_u32(base + utf16(&inserted))),
        edits,
        stops,
    })
}

#[cfg(test)]
mod tests {
    use ferrule_lsp_client::lsp_types::{Position, Range, TextEdit};

    use super::*;

    fn item(label: &str, sort: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            sort_text: Some(sort.into()),
            ..CompletionItem::default()
        }
    }

    #[test]
    fn prefers_exact_label_then_fuzzy_score() {
        let items = vec![
            item("formatPrice", "1"),
            item("forEach", "0"),
            item("fp", "2"),
            item("unrelated", "0"),
        ];
        let (views, total) = rank(&items, "fP");
        assert_eq!(total, 2);
        let labels: Vec<&str> = views.iter().map(|v| v.label.as_str()).collect();
        assert_eq!(labels, ["fp", "formatPrice"]);
        assert_eq!(views[1].matched, [0, 6]);
        // Empty prefix: the server's order.
        let (all, _) = rank(&items, "");
        assert_eq!(all[0].label, "forEach");
    }

    #[test]
    fn expands_snippets_into_text_and_stops() {
        let s = parse_snippet("for (const ${1:item} of ${2:items}) {\n\t$0\n}");
        assert_eq!(s.text, "for (const item of items) {\n\t\n}");
        assert_eq!(s.stops, [vec![(11, 15)], vec![(19, 24)], vec![(29, 29)]]);
        // Linked stops, choices, variables, escapes, nesting.
        let s = parse_snippet(
            "${1:a} = ${1:a}; ${2|let,const|} \\$x ${TM_FILENAME:file} ${3:f(${4:y})}",
        );
        assert_eq!(s.text, "a = a; let $x file f(y)");
        assert_eq!(s.stops[0], vec![(0, 1), (4, 5)]);
        assert_eq!(s.stops[1], vec![(7, 10)]);
        assert_eq!(s.stops[2], vec![(19, 23)]);
        assert_eq!(s.stops[3], vec![(21, 22)]);
        assert_eq!(s.stops.last(), Some(&vec![(23, 23)]));
    }

    #[test]
    fn accept_replaces_the_prefix_and_adds_imports_before_it() {
        let doc = Document::new("const n = forP\n");
        let mut item = item("formatPrice", "0");
        item.insert_text = Some("formatPrice($1)$0".into());
        item.insert_text_format = Some(InsertTextFormat::SNIPPET);
        item.additional_text_edits = Some(vec![TextEdit {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            new_text: "import { formatPrice } from \"./format\";\n".into(),
        }]);
        let edit = accept_edit(&doc, &item, 10, 14).unwrap();
        assert_eq!(edit.edits.len(), 2);
        assert_eq!((edit.edits[1].from, edit.edits[1].to), (10, 14));
        assert_eq!(edit.edits[1].insert, "formatPrice()");
        let import_len = to_u32(utf16("import { formatPrice } from \"./format\";\n"));
        // $1 inside the parentheses, $0 after them, shifted by the import.
        assert_eq!(edit.stops[0][0].from, import_len + 10 + 12);
        assert_eq!(edit.stops[1][0].from, import_len + 10 + 13);
    }
}
