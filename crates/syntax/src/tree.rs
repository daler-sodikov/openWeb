//! Incremental syntax tree for one document, with injected languages.

use std::cmp::Reverse;
use std::ops::Range;
use std::sync::Arc;

use ferrule_buffer::{ByteEdit, Rope};
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Node, Parser, Point, QueryCursor, Tree};

use crate::highlight::{HighlightSpan, TokenClass};
use crate::registry::{HighlightLanguage, LanguageRegistry, Precedence, SyntaxError};

/// An embedded-language region (e.g. a `<script>` body) with its own tree.
struct Injected {
    range: Range<usize>,
    language: Arc<HighlightLanguage>,
    tree: Tree,
}

/// Parser state for one document. Edits mark the tree stale; the next
/// request reparses incrementally. Injected regions are parsed on demand
/// for the requested range and cached until the next edit.
pub struct SyntaxTree {
    language: Arc<HighlightLanguage>,
    parser: Parser,
    pub(crate) tree: Option<Tree>,
    stale: bool,
    registry: Option<Arc<LanguageRegistry>>,
    injected: Vec<Injected>,
}

impl std::fmt::Debug for SyntaxTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxTree")
            .field("language", &self.language.id())
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

/// Reads rope chunks for tree-sitter.
fn reader<'a>(rope: &'a Rope) -> impl FnMut(usize, Point) -> &'a [u8] + 'a {
    let len = rope.len_bytes();
    move |byte: usize, _: Point| -> &[u8] {
        if byte >= len {
            return &[];
        }
        let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - chunk_start..]
    }
}

/// Text of a node for query predicates.
pub(crate) fn node_text(rope: &Rope, node: Node<'_>) -> String {
    let start = rope.byte_to_char(node.start_byte().min(rope.len_bytes()));
    let end = rope.byte_to_char(node.end_byte().min(rope.len_bytes()));
    rope.slice(start..end).to_string()
}

impl SyntaxTree {
    /// Creates an unparsed tree for `language`. Without a registry (see
    /// [`SyntaxTree::with_registry`]) embedded languages are not highlighted.
    pub fn new(language: Arc<HighlightLanguage>) -> Result<Self, SyntaxError> {
        let mut parser = Parser::new();
        parser
            .set_language(&language.def.language)
            .map_err(|err| SyntaxError::Grammar {
                language: language.id(),
                message: err.to_string(),
            })?;
        Ok(Self {
            language,
            parser,
            tree: None,
            stale: true,
            registry: None,
            injected: Vec::new(),
        })
    }

    /// Resolves injected languages through `registry`.
    pub fn with_registry(mut self, registry: Arc<LanguageRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// The language of this tree.
    pub fn language(&self) -> &Arc<HighlightLanguage> {
        &self.language
    }

    /// Records edits (in the order they were applied to the rope).
    pub fn edit(&mut self, edits: &[ByteEdit]) {
        if let Some(tree) = &mut self.tree {
            for e in edits {
                tree.edit(&InputEdit {
                    start_byte: e.start_byte,
                    old_end_byte: e.old_end_byte,
                    new_end_byte: e.new_end_byte,
                    start_position: point(e.start_point),
                    old_end_position: point(e.old_end_point),
                    new_end_position: point(e.new_end_point),
                });
            }
        }
        if !edits.is_empty() {
            self.stale = true;
            self.injected.clear();
        }
    }

    /// Reparses if needed (incrementally when a previous tree exists).
    pub fn ensure_parsed(&mut self, rope: &Rope) {
        if !self.stale && self.tree.is_some() {
            return;
        }
        let mut read = reader(rope);
        // `parse_with_options` returns None only on cancellation/timeouts,
        // which we do not configure; keep the old tree in that case.
        if let Some(tree) = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None)
        {
            self.tree = Some(tree);
            self.stale = false;
            self.injected.clear();
        }
    }

    /// Highlight spans intersecting `range` (bytes), clipped to it, sorted
    /// and non-overlapping. Embedded languages are painted over the host.
    pub fn highlight(&mut self, rope: &Rope, range: Range<usize>) -> Vec<HighlightSpan> {
        self.ensure_parsed(rope);
        let range = range.start.min(rope.len_bytes())..range.end.min(rope.len_bytes());
        if range.is_empty() {
            return Vec::new();
        }
        self.load_injections(rope, range.clone());
        let Some(tree) = &self.tree else {
            return Vec::new();
        };

        let mut paint = vec![0u8; range.len()];
        paint_captures(&self.language, tree, rope, &range, &mut paint);
        for injected in &self.injected {
            if injected.range.start < range.end && injected.range.end > range.start {
                paint_captures(&injected.language, &injected.tree, rope, &range, &mut paint);
            }
        }

        let mut spans = Vec::new();
        let mut i = 0;
        while i < paint.len() {
            let code = paint[i];
            let mut j = i + 1;
            while j < paint.len() && paint[j] == code {
                j += 1;
            }
            if let Some(class) = TokenClass::from_code(code) {
                spans.push(HighlightSpan {
                    start: range.start + i,
                    end: range.start + j,
                    class,
                });
            }
            i = j;
        }
        spans
    }

    /// Finds and parses (or reuses) injected regions intersecting `range`.
    fn load_injections(&mut self, rope: &Rope, range: Range<usize>) {
        let (Some(query), Some(registry), Some(tree)) =
            (&self.language.injections, &self.registry, &self.tree)
        else {
            return;
        };
        let names = query.capture_names();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range);
        let text = |node: Node<'_>| {
            let start = rope.byte_to_char(node.start_byte());
            let end = rope.byte_to_char(node.end_byte());
            rope.slice(start..end).chunks().map(str::as_bytes)
        };
        let mut matches = cursor.matches(query, tree.root_node(), text);
        // (content range, pattern index, language name); the first matching
        // pattern decides the language of a region.
        let mut found: Vec<(tree_sitter::Range, usize, String)> = Vec::new();
        while let Some(m) = matches.next() {
            let mut content = None;
            let mut language = None;
            for capture in m.captures() {
                match names.get(capture.index as usize).copied() {
                    Some("injection.content") => content = Some(capture.node),
                    Some("injection.language") => language = Some(node_text(rope, capture.node)),
                    _ => {}
                }
            }
            if language.is_none() {
                language = query
                    .property_settings(m.pattern_index)
                    .iter()
                    .find(|p| &*p.key == "injection.language")
                    .and_then(|p| p.value.as_deref().map(str::to_owned));
            }
            if let (Some(node), Some(language)) = (content, language) {
                found.push((node.range(), m.pattern_index, language));
            }
        }
        found.sort_by_key(|(r, p, _)| (r.start_byte, Reverse(r.end_byte), *p));
        found.dedup_by_key(|(r, _, _)| (r.start_byte, r.end_byte));

        for (region, _, name) in found {
            let bytes = region.start_byte..region.end_byte;
            let Some(language) = registry.for_name(&name) else {
                continue;
            };
            if bytes.is_empty()
                || self
                    .injected
                    .iter()
                    .any(|i| i.range == bytes && i.language.id() == language.id())
            {
                continue;
            }
            let mut parser = Parser::new();
            if parser.set_language(&language.def.language).is_err()
                || parser.set_included_ranges(&[region]).is_err()
            {
                continue;
            }
            let mut read = reader(rope);
            if let Some(tree) = parser.parse_with_options(&mut read, None, None) {
                self.injected.push(Injected {
                    range: bytes,
                    language,
                    tree,
                });
            }
        }
    }
}

/// Paints the highlight captures of `tree` within `range` into `paint`.
/// Inner nodes override outer ones; for the same node the language's
/// [`Precedence`] decides.
fn paint_captures(
    language: &HighlightLanguage,
    tree: &Tree,
    rope: &Rope,
    range: &Range<usize>,
    paint: &mut [u8],
) {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range.clone());
    let text = |node: Node<'_>| {
        let start = rope.byte_to_char(node.start_byte());
        let end = rope.byte_to_char(node.end_byte());
        rope.slice(start..end).chunks().map(str::as_bytes)
    };
    let mut captures = cursor.captures(&language.query, tree.root_node(), text);
    // (start, end, pattern, class)
    let mut items: Vec<(usize, usize, usize, Option<TokenClass>)> = Vec::new();
    while let Some((m, index)) = captures.next() {
        let capture = m.captures()[*index];
        let class = language
            .classes
            .get(capture.index as usize)
            .copied()
            .flatten();
        items.push((
            capture.node.start_byte(),
            capture.node.end_byte(),
            m.pattern_index,
            class,
        ));
    }
    // Outer nodes first; for the same node, the winning pattern first.
    let last_wins = language.def.precedence == Precedence::LastWins;
    items.sort_by_key(|&(s, e, p, _)| {
        (
            s,
            Reverse(e),
            if last_wins {
                Reverse(p)
            } else {
                Reverse(usize::MAX - p)
            },
        )
    });
    items.dedup_by_key(|&mut (s, e, _, _)| (s, e));
    for (start, end, _, class) in items {
        let s = start.max(range.start) - range.start;
        let e = end.min(range.end).saturating_sub(range.start);
        if s < e {
            paint[s..e].fill(class.map_or(0, TokenClass::code));
        }
    }
}

fn point(p: ferrule_buffer::Point) -> Point {
    Point {
        row: p.row,
        column: p.column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{LanguageDef, Precedence};
    use ferrule_buffer::{Document, Edit, EditKind, Selection, Transaction};

    fn json() -> Arc<HighlightLanguage> {
        Arc::new(
            HighlightLanguage::new(LanguageDef::new(
                "json",
                "JSON",
                &["json"],
                tree_sitter_json::LANGUAGE.into(),
                vec![tree_sitter_json::HIGHLIGHTS_QUERY],
                Precedence::FirstWins,
            ))
            .unwrap(),
        )
    }

    fn full(tree: &mut SyntaxTree, doc: &Document) -> Vec<HighlightSpan> {
        tree.highlight(doc.rope(), 0..doc.rope().len_bytes())
    }

    #[test]
    fn highlights_json_keys_and_values() {
        let doc = Document::new(r#"{"a": 1, "b": [true, "x"]}"#);
        let mut tree = SyntaxTree::new(json()).unwrap();
        let rendered = crate::render_spans(doc.rope(), &full(&mut tree, &doc));
        assert!(rendered.contains("property \"\\\"a\\\"\""), "{rendered}");
        assert!(rendered.contains("number \"1\""), "{rendered}");
        assert!(rendered.contains("constant \"true\""), "{rendered}");
        assert!(rendered.contains("string \"\\\"x\\\"\""), "{rendered}");
    }

    #[test]
    fn incremental_reparse_matches_fresh_parse() {
        let mut doc = Document::new("{\n  \"name\": \"ferrule\",\n  \"n\": 1\n}\n");
        let lang = json();
        let mut tree = SyntaxTree::new(lang.clone()).unwrap();
        full(&mut tree, &doc);

        let edits = [
            vec![Edit::new(11, 11, "-ide")],
            vec![Edit::new(2, 2, "\"new\": [false, null],\n  ")],
            vec![Edit::new(0, 1, "")], // break the document
            vec![Edit::new(0, 0, "{")],
        ];
        for (i, e) in edits.into_iter().enumerate() {
            let out = doc
                .apply(
                    doc.version(),
                    &[Transaction {
                        edits: e,
                        kind: EditKind::Other,
                        selection_before: Selection::cursor(0),
                        selection_after: Selection::cursor(0),
                    }],
                    i as u64,
                )
                .unwrap();
            tree.edit(&out.byte_edits);
            let incremental = full(&mut tree, &doc);
            let mut fresh = SyntaxTree::new(lang.clone()).unwrap();
            assert_eq!(incremental, full(&mut fresh, &doc), "after edit {i}");
        }
    }

    #[test]
    fn range_is_clipped() {
        let doc = Document::new(r#"{"key": "value"}"#);
        let mut tree = SyntaxTree::new(json()).unwrap();
        let spans = tree.highlight(doc.rope(), 3..12);
        assert!(spans.iter().all(|s| s.start >= 3 && s.end <= 12));
        assert!(!spans.is_empty());
        assert!(tree.highlight(doc.rope(), 100..200).is_empty());
    }
}
