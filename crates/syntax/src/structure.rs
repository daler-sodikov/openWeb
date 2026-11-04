//! Structure from the syntax tree: folding ranges, bracket pairs and
//! breadcrumbs. Positions are byte offsets in the rope. Language
//! differences are handled by node kinds the grammars share (brackets as
//! first/last children, `*_tag` children of elements, `name`/`key` fields).

use std::ops::Range;

use ferrule_buffer::Rope;
use tree_sitter::Node;

use crate::tree::{SyntaxTree, node_text};

/// A foldable range: the text between `start` and `end` is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    /// First hidden byte.
    pub start: usize,
    /// End of the hidden text (exclusive).
    pub end: usize,
}

/// A matched bracket pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BracketPair {
    /// The opening bracket.
    pub open: Range<usize>,
    /// The closing bracket.
    pub close: Range<usize>,
}

/// One breadcrumb: a named structural node around a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    /// Display label.
    pub label: String,
    /// Node kind (`function_declaration`, `element`, ...).
    pub kind: String,
    /// Where the node starts.
    pub start: usize,
}

const OPENERS: [(&str, &str); 3] = [("{", "}"), ("[", "]"), ("(", ")")];
/// Nodes folded from the end of their first line to their end.
const LINE_FOLD_KINDS: &[&str] = &[
    "block_mapping_pair",
    "block_sequence_item",
    "section",
    "fenced_code_block",
];
const START_TAGS: &[&str] = &["start_tag", "jsx_opening_element"];
const END_TAGS: &[&str] = &["end_tag", "jsx_closing_element"];
const MAX_LABEL: usize = 60;

fn closer_of(open: &str) -> Option<&'static str> {
    OPENERS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

fn is_bracket(kind: &str) -> bool {
    OPENERS.iter().any(|(o, c)| *o == kind || *c == kind)
}

fn line_end_byte(rope: &Rope, byte: usize) -> usize {
    let line = rope.byte_to_line(byte.min(rope.len_bytes()));
    if line + 1 < rope.len_lines() {
        rope.line_to_byte(line + 1) - 1
    } else {
        rope.len_bytes()
    }
}

/// End of `node` without trailing whitespace (sections include blank lines).
fn trimmed_end(rope: &Rope, node: Node<'_>) -> usize {
    let text = node_text(rope, node);
    node.start_byte() + text.trim_end().len()
}

fn fold_for(rope: &Rope, node: Node<'_>) -> Option<FoldRange> {
    let rows = node.start_position().row..node.end_position().row;
    if rows.is_empty() {
        return None;
    }
    let kind = node.kind();
    let count = node.child_count();
    let first = node.child(0);
    let last = count.checked_sub(1).and_then(|i| node.child(i));
    let range = if let (Some(first), Some(last)) = (first, last)
        && !first.is_named()
        && closer_of(first.kind()).is_some_and(|c| c == last.kind())
    {
        (first.end_byte(), last.start_byte())
    } else if kind.contains("comment") {
        (line_end_byte(rope, node.start_byte()), node.end_byte())
    } else if let (Some(open), Some(close)) =
        (find_child(node, START_TAGS), find_child(node, END_TAGS))
    {
        (open.end_byte(), close.start_byte())
    } else if LINE_FOLD_KINDS.contains(&kind) {
        (
            line_end_byte(rope, node.start_byte()),
            trimmed_end(rope, node),
        )
    } else {
        return None;
    };
    let (start, end) = range;
    // Only fold if something on a later line is hidden.
    (start < end && rope.byte_to_line(end) > rope.byte_to_line(start))
        .then_some(FoldRange { start, end })
}

fn find_child<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| kinds.contains(&c.kind()))
}

fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > MAX_LABEL {
        format!("{}…", line.chars().take(MAX_LABEL).collect::<String>())
    } else {
        line.to_owned()
    }
}

/// Kinds with a `name` field that are worth a breadcrumb.
fn is_named_scope(kind: &str) -> bool {
    kind.ends_with("_declaration")
        || kind.ends_with("_definition")
        || kind.ends_with("_signature")
        || kind == "variable_declarator"
        || kind == "class"
        || kind == "function_expression"
        || kind == "module"
        || kind == "internal_module"
        || kind == "mixin_statement"
}

fn crumb_label(rope: &Rope, node: Node<'_>) -> Option<String> {
    let text = |n: Node<'_>| short(&node_text(rope, n));
    match node.kind() {
        "element" => find_child(node, &["start_tag", "self_closing_tag"])
            .and_then(|t| find_child(t, &["tag_name"]))
            .map(text),
        "jsx_element" => node
            .child_by_field_name("open_tag")
            .and_then(|t| t.child_by_field_name("name"))
            .map(text),
        "jsx_self_closing_element" => node.child_by_field_name("name").map(text),
        "rule_set" => find_child(node, &["selectors"]).map(text),
        "pair" | "block_mapping_pair" | "flow_pair" => node.child_by_field_name("key").map(|k| {
            let label = text(k);
            label.trim_matches(|c| c == '"' || c == '\'').to_owned()
        }),
        "section" => find_child(node, &["atx_heading", "setext_heading"])
            .map(|h| text(h).trim_start_matches('#').trim().to_owned()),
        kind if is_named_scope(kind) => node.child_by_field_name("name").map(text),
        _ => None,
    }
    .filter(|l| !l.is_empty())
}

impl SyntaxTree {
    /// Folding ranges, sorted by start; at most one (the largest) per
    /// starting line.
    pub fn folds(&mut self, rope: &Rope) -> Vec<FoldRange> {
        self.ensure_parsed(rope);
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let mut folds: Vec<FoldRange> = Vec::new();
        let mut cursor = tree.walk();
        'walk: loop {
            let node = cursor.node();
            if let Some(fold) = fold_for(rope, node) {
                folds.push(fold);
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    continue 'walk;
                }
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        folds.sort_by_key(|f| (rope.byte_to_line(f.start), std::cmp::Reverse(f.end)));
        folds.dedup_by_key(|f| rope.byte_to_line(f.start));
        folds
    }

    /// The bracket pair touching `byte` (the bracket after it, else the one
    /// before it). Brackets in strings and comments are not tokens, so they
    /// never match.
    pub fn bracket_pair(&mut self, rope: &Rope, byte: usize) -> Option<BracketPair> {
        self.ensure_parsed(rope);
        let tree = self.tree.as_ref()?;
        let root = tree.root_node();
        let candidates = [Some(byte), byte.checked_sub(1)];
        for at in candidates.into_iter().flatten() {
            let Some(node) = root.descendant_for_byte_range(at, at + 1) else {
                continue;
            };
            if node.is_named() || !is_bracket(node.kind()) || node.start_byte() != at {
                continue;
            }
            let parent = node.parent()?;
            let count = parent.child_count();
            let (first, last) = (parent.child(0)?, parent.child(count.checked_sub(1)?)?);
            if first.id() == last.id() || closer_of(first.kind()) != Some(last.kind()) {
                continue;
            }
            if node.id() == first.id() || node.id() == last.id() {
                return Some(BracketPair {
                    open: first.byte_range(),
                    close: last.byte_range(),
                });
            }
        }
        None
    }

    /// Breadcrumbs at `byte`, outermost first.
    pub fn breadcrumbs(&mut self, rope: &Rope, byte: usize) -> Vec<Crumb> {
        self.ensure_parsed(rope);
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let Some(mut node) = tree.root_node().descendant_for_byte_range(byte, byte) else {
            return Vec::new();
        };
        let mut crumbs = Vec::new();
        loop {
            if let Some(label) = crumb_label(rope, node) {
                crumbs.push(Crumb {
                    label,
                    kind: node.kind().to_owned(),
                    start: node.start_byte(),
                });
            }
            match node.parent() {
                Some(parent) => node = parent,
                None => break,
            }
        }
        crumbs.reverse();
        crumbs
    }
}
