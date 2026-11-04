//! Breadcrumbs and folds for style sheets.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use ferrule_buffer::Rope;
use ferrule_syntax::{HighlightLanguage, SyntaxTree};

#[test]
fn css_and_scss_structure() {
    let css = ".card > h1 {\n  color: red;\n}\n";
    let rope = Rope::from_str(css);
    let mut t = SyntaxTree::new(Arc::new(
        HighlightLanguage::new(ferrule_lang_css::css()).unwrap(),
    ))
    .unwrap();
    let labels: Vec<_> = t
        .breadcrumbs(&rope, css.find("red").unwrap())
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec![".card > h1"]);
    assert_eq!(t.folds(&rope).len(), 1);

    let scss = ".card {\n  .title {\n    color: red;\n  }\n}\n";
    let rope = Rope::from_str(scss);
    let mut t = SyntaxTree::new(Arc::new(
        HighlightLanguage::new(ferrule_lang_css::scss()).unwrap(),
    ))
    .unwrap();
    let labels: Vec<_> = t
        .breadcrumbs(&rope, scss.find("red").unwrap())
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec![".card", ".title"]);
    assert_eq!(t.folds(&rope).len(), 2);
}
