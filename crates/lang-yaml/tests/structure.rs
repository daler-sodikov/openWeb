//! Breadcrumbs and folds for YAML.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use ferrule_buffer::Rope;
use ferrule_syntax::{HighlightLanguage, SyntaxTree};

#[test]
fn yaml_structure() {
    let text = "jobs:\n  test:\n    steps:\n      - run: bun test\n";
    let rope = Rope::from_str(text);
    let mut t = SyntaxTree::new(Arc::new(
        HighlightLanguage::new(ferrule_lang_yaml::yaml()).unwrap(),
    ))
    .unwrap();
    let labels: Vec<_> = t
        .breadcrumbs(&rope, text.find("bun").unwrap())
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["jobs", "test", "steps", "run"]);
    let folds: Vec<_> = t
        .folds(&rope)
        .iter()
        .map(|f| rope.byte_to_line(f.start))
        .collect();
    assert_eq!(folds, vec![0, 1, 2]);
}
