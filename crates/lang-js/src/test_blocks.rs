//! Test blocks (`describe`, `it`, `test` and their modifiers) found with a
//! tree-sitter query, for the gutter's run icons (`docs/ARCHITECTURE.md`,
//! 4.10).

use std::path::Path;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

/// Ours: calls whose first argument is a string (filtered in Rust by the
/// callee: `describe`, `it`, `test`, `suite` and allowed modifiers).
const QUERY: &str = r#"
(call_expression
  function: [(identifier) (member_expression) (call_expression)] @callee
  arguments: (arguments . [(string) (template_string)] @title)) @block
"#;

/// Roots of test calls.
const SUITES: &[&str] = &["describe", "suite"];
const TESTS: &[&str] = &["it", "test"];
/// Modifiers that keep a call a test block (Vitest, Jest, Playwright).
const MODIFIERS: &[&str] = &[
    "only",
    "skip",
    "todo",
    "concurrent",
    "sequential",
    "shuffle",
    "each",
    "for",
    "skipIf",
    "runIf",
    "fixme",
    "fail",
    "fails",
    "serial",
    "parallel",
    "describe",
];
/// Modifiers called with their own arguments first: `describe.each(t)("x")`.
const CALLED: &[&str] = &["each", "for", "skipIf", "runIf"];

/// What a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestBlockKind {
    /// `describe` (or Playwright's `test.describe`).
    Suite,
    /// `it` or `test`.
    Test,
}

/// A test or suite in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestBlock {
    /// Suite or test.
    pub kind: TestBlockKind,
    /// Titles from the outermost suite to this block, as written
    /// (parameterized titles keep their `%s` / `$name` placeholders).
    pub name: Vec<String>,
    /// Zero-based line of the call.
    pub line: u32,
    /// Zero-based line where the call ends (its body included).
    pub end_line: u32,
}

fn grammar(path: &Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        "ts" | "mts" | "cts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "mjs" | "cjs" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    }
}

/// The block kind of a callee such as `test.describe.only`, or `None` if
/// it is not a test call.
fn callee_kind(callee: Node<'_>, text: &str) -> Option<TestBlockKind> {
    let (chain, called) = if callee.kind() == "call_expression" {
        // `describe.each(table)`: the chain is the inner function.
        (callee.child_by_field_name("function")?, true)
    } else {
        (callee, false)
    };
    let source = text.get(chain.byte_range())?;
    let segments: Vec<&str> = source.split('.').map(str::trim).collect();
    let (root, rest) = segments.split_first()?;
    if !SUITES.contains(root) && !TESTS.contains(root) {
        return None;
    }
    if !rest.iter().all(|m| MODIFIERS.contains(m)) {
        return None;
    }
    if called && !rest.last().is_some_and(|m| CALLED.contains(m)) {
        return None;
    }
    if SUITES.contains(root) || rest.contains(&"describe") {
        Some(TestBlockKind::Suite)
    } else {
        Some(TestBlockKind::Test)
    }
}

/// The title of a string or template-string literal.
fn title(node: Node<'_>, text: &str) -> Option<String> {
    let source = text.get(node.byte_range())?;
    let inner = source.get(1..source.len().checked_sub(1)?)?;
    Some(inner.to_owned())
}

/// The test blocks of a JavaScript or TypeScript file (empty for other
/// files or text that does not parse).
pub fn test_blocks(path: &Path, text: &str) -> Vec<TestBlock> {
    let Some(language) = grammar(path) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(&language, QUERY) else {
        return Vec::new();
    };
    let names = query.capture_names();
    let index = |name: &str| names.iter().position(|n| *n == name);
    let (Some(block_i), Some(callee_i), Some(title_i)) =
        (index("block"), index("callee"), index("title"))
    else {
        return Vec::new();
    };
    // Blocks with their byte ranges, in source order.
    let mut found: Vec<(std::ops::Range<usize>, TestBlockKind, String, u32, u32)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        let node_of = |i: usize| {
            m.captures()
                .iter()
                .find(|c| c.index as usize == i)
                .map(|c| c.node)
        };
        let (Some(block), Some(callee), Some(title_node)) =
            (node_of(block_i), node_of(callee_i), node_of(title_i))
        else {
            continue;
        };
        let (Some(kind), Some(title)) = (callee_kind(callee, text), title(title_node, text)) else {
            continue;
        };
        let row = |r: usize| u32::try_from(r).unwrap_or(u32::MAX);
        let (line, end_line) = (
            row(block.start_position().row),
            row(block.end_position().row),
        );
        found.push((block.byte_range(), kind, title, line, end_line));
    }
    found.sort_by_key(|(range, ..)| range.start);
    found
        .iter()
        .map(|(range, kind, title, line, end_line)| {
            let mut name: Vec<String> = found
                .iter()
                .filter(|(outer, outer_kind, ..)| {
                    *outer_kind == TestBlockKind::Suite
                        && outer.start < range.start
                        && range.end <= outer.end
                })
                .map(|(_, _, outer_title, ..)| outer_title.clone())
                .collect();
            name.push(title.clone());
            TestBlock {
                kind: *kind,
                name,
                line: *line,
                end_line: *end_line,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(path: &str, text: &str) -> Vec<(TestBlockKind, String, u32)> {
        test_blocks(Path::new(path), text)
            .into_iter()
            .map(|b| (b.kind, b.name.join(" > "), b.line))
            .collect()
    }

    #[test]
    fn blocks_know_where_they_end() {
        let text = "describe(\"math\", () => {\n  it(\"adds\", () => {\n    run();\n  });\n});\n";
        let ends: Vec<(u32, u32)> = test_blocks(Path::new("a.test.js"), text)
            .iter()
            .map(|b| (b.line, b.end_line))
            .collect();
        assert_eq!(ends, [(0, 4), (1, 3)]);
    }

    #[test]
    fn finds_nested_blocks_with_modifiers_and_tables() {
        let text = r#"import { describe, it, expect } from "vitest";
describe("math", () => {
  it("adds", () => {});
  it.skip('is skipped', () => {});
  describe.each([[1]])("table %i", (n) => {
    test.only(`case`, () => {});
  });
  it.each([[1, 2]])("sums %i", () => {});
  expect("not a test").toBe("x");
  helper("not a test", () => {});
});
test("top level", () => {});
"#;
        use TestBlockKind::{Suite, Test};
        assert_eq!(
            summary("a.test.ts", text),
            [
                (Suite, "math".into(), 1),
                (Test, "math > adds".into(), 2),
                (Test, "math > is skipped".into(), 3),
                (Suite, "math > table %i".into(), 4),
                (Test, "math > table %i > case".into(), 5),
                (Test, "math > sums %i".into(), 7),
                (Test, "top level".into(), 11),
            ]
        );
    }

    #[test]
    fn playwright_describe_and_hooks() {
        let text = r#"import { test, expect } from "@playwright/test";
test.describe("e2e", () => {
  test.beforeEach(async () => {});
  test("works", async ({ page }) => {
    await test.step("inner step", async () => {});
  });
  test.describe.serial("serial", () => {
    test.fixme("later", async () => {});
  });
});
"#;
        use TestBlockKind::{Suite, Test};
        assert_eq!(
            summary("e2e/x.spec.tsx", text),
            [
                (Suite, "e2e".into(), 1),
                (Test, "e2e > works".into(), 3),
                (Suite, "e2e > serial".into(), 6),
                (Test, "e2e > serial > later".into(), 7),
            ]
        );
    }

    #[test]
    fn other_files_have_none() {
        assert!(test_blocks(Path::new("a.css"), "describe('x', () => {})").is_empty());
        assert_eq!(summary("a.test.js", "it('js', () => {})").len(), 1);
    }
}
