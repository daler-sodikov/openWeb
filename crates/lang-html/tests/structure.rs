//! Breadcrumbs, folds and injections for markup.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use ferrule_buffer::Rope;
use ferrule_syntax::{LanguageRegistry, SyntaxTree, TokenClass};

#[test]
fn vue_elements_and_injected_script() {
    let text = "<template>\n  <div>\n    <p>hi</p>\n  </div>\n</template>\n<script>\nconst x = 1;\n</script>\n";
    let rope = Rope::from_str(text);
    let mut registry = LanguageRegistry::default();
    for def in ferrule_lang_html::languages()
        .into_iter()
        .chain(ferrule_lang_js::languages())
    {
        registry.register(def).unwrap();
    }
    let registry = Arc::new(registry);
    let vue = registry.for_name("vue").unwrap();
    let mut t = SyntaxTree::new(vue).unwrap().with_registry(registry);
    let labels: Vec<_> = t
        .breadcrumbs(&rope, text.find("hi").unwrap())
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, vec!["template", "div", "p"]);
    assert_eq!(t.folds(&rope).len(), 3, "template, div, script");
    let spans = t.highlight(&rope, 0..rope.len_bytes());
    let const_at = text.find("const").unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.start == const_at && s.class == TokenClass::Keyword)
    );
}
