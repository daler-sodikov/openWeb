//! Symbol extraction with tags queries.

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, QueryCursor};

use crate::registry::HighlightLanguage;

/// A symbol definition found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolDef {
    /// Symbol name.
    pub name: String,
    /// Kind from the `@definition.<kind>` capture (`function`, `class`, ...).
    pub kind: String,
    /// Zero-based line of the name.
    pub line: u32,
    /// Zero-based UTF-16 column of the name.
    pub column: u32,
}

impl HighlightLanguage {
    /// Extracts symbol definitions from `text`. Empty if the language has no
    /// tags query or the text cannot be parsed.
    pub fn symbols(&self, text: &str) -> Vec<SymbolDef> {
        let Some(tags) = &self.tags else {
            return Vec::new();
        };
        let mut parser = Parser::new();
        if parser.set_language(&self.def.language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(text, None) else {
            return Vec::new();
        };
        let names = tags.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(tags, tree.root_node(), text.as_bytes());
        let mut out: Vec<SymbolDef> = Vec::new();
        while let Some(m) = matches.next() {
            let mut name_node = None;
            let mut kind = None;
            for capture in m.captures() {
                let capture_name = names.get(capture.index as usize).copied().unwrap_or("");
                if capture_name == "name" {
                    name_node = Some(capture.node);
                } else if let Some(k) = capture_name.strip_prefix("definition.") {
                    kind = Some(k);
                }
            }
            let (Some(node), Some(kind)) = (name_node, kind) else {
                continue;
            };
            let Some(name) = text.get(node.byte_range()) else {
                continue;
            };
            let point = node.start_position();
            let line_start = node.start_byte() - point.column;
            let column = text
                .get(line_start..node.start_byte())
                .map_or(0, |s| s.encode_utf16().count());
            let symbol = SymbolDef {
                name: name.to_owned(),
                kind: kind.to_owned(),
                line: u32::try_from(point.row).unwrap_or(u32::MAX),
                column: u32::try_from(column).unwrap_or(u32::MAX),
            };
            if !out.iter().any(|s| {
                s.name == symbol.name && s.line == symbol.line && s.column == symbol.column
            }) {
                out.push(symbol);
            }
        }
        out.sort_by_key(|s| (s.line, s.column));
        out
    }
}
