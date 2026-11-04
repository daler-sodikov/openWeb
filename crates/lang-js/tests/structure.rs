//! Folding ranges, bracket pairs and breadcrumbs on real grammars.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ferrule_buffer::Rope;
use ferrule_syntax::{HighlightLanguage, LanguageDef, SyntaxTree};

const APP: &str = r#"import { Counter } from "./Counter";

/* A multi-line
   comment */
export function App({ title }: { title: string }) {
  const label = "a { not a bracket }";
  return (
    <main className="app">
      <Counter start={1} />
    </main>
  );
}
"#;

fn tree(def: LanguageDef) -> SyntaxTree {
    SyntaxTree::new(Arc::new(HighlightLanguage::new(def).unwrap())).unwrap()
}

fn pos(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not found"))
}

#[test]
fn tsx_breadcrumbs() {
    let rope = Rope::from_str(APP);
    let mut t = tree(ferrule_lang_js::tsx());
    let at = pos(APP, "start={1}");
    let labels: Vec<_> = t
        .breadcrumbs(&rope, at)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["App", "main", "Counter"]);
    let at = pos(APP, "\"a {");
    let labels: Vec<_> = t
        .breadcrumbs(&rope, at)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["App", "label"]);
}

#[test]
fn tsx_folds() {
    let rope = Rope::from_str(APP);
    let mut t = tree(ferrule_lang_js::tsx());
    let folds = t.folds(&rope);
    let lines: Vec<_> = folds
        .iter()
        .map(|f| (rope.byte_to_line(f.start) + 1, rope.byte_to_line(f.end) + 1))
        .collect();
    // comment (3-4), function body (5-12), return parens (7-11), <main> (8-10)
    assert_eq!(lines, vec![(3, 4), (5, 12), (7, 11), (8, 10)]);
    let body = folds[1];
    assert_eq!(&APP[body.start - 1..body.start], "{");
    assert_eq!(&APP[body.end..body.end + 1], "}");
}

#[test]
fn brackets_ignore_strings() {
    let rope = Rope::from_str(APP);
    let mut t = tree(ferrule_lang_js::tsx());
    let open = pos(APP, "{ title }:");
    let pair = t.bracket_pair(&rope, open).unwrap();
    assert_eq!(pair.open, open..open + 1);
    assert_eq!(&APP[pair.close.clone()], "}");
    assert_eq!(pair.close.start, pos(APP, " }: {") + 1);
    // Caret after the closing bracket also matches.
    assert_eq!(t.bracket_pair(&rope, pair.close.end).unwrap(), pair);
    // A brace inside a string is not a bracket.
    assert!(t.bracket_pair(&rope, pos(APP, "{ not")).is_none());
}

#[test]
fn json_breadcrumbs_use_keys() {
    let text = "{\n  \"scripts\": {\n    \"build\": \"vite build\"\n  }\n}\n";
    let rope = Rope::from_str(text);
    let mut t = tree(ferrule_lang_js::json());
    let labels: Vec<_> = t
        .breadcrumbs(&rope, pos(text, "vite"))
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["scripts", "build"]);
    assert_eq!(t.folds(&rope).len(), 2);
}
