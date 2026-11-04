//! Breadcrumbs and folds for Markdown sections.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use ferrule_buffer::Rope;
use ferrule_syntax::{HighlightLanguage, SyntaxTree};

#[test]
fn markdown_sections() {
    let text = "# Guide\n\nIntro.\n\n## Install\n\nRun it.\n\n## Use\n\nDone.\n";
    let rope = Rope::from_str(text);
    let mut t = SyntaxTree::new(Arc::new(
        HighlightLanguage::new(ferrule_lang_markdown::markdown()).unwrap(),
    ))
    .unwrap();
    let labels: Vec<_> = t
        .breadcrumbs(&rope, text.find("Run").unwrap())
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["Guide", "Install"]);
    let folds: Vec<_> = t
        .folds(&rope)
        .iter()
        .map(|f| (rope.byte_to_line(f.start), rope.byte_to_line(f.end)))
        .collect();
    assert_eq!(folds, vec![(0, 10), (4, 6), (8, 10)]);
}
