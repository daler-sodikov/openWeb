//! CSS language support.
//!
//! M1: the tree-sitter CSS grammar and its bundled highlight query. SCSS and
//! LESS arrive in M2 (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// CSS (`.css`).
pub fn css() -> LanguageDef {
    LanguageDef {
        id: "css",
        name: "CSS",
        extensions: &["css"],
        language: tree_sitter_css::LANGUAGE.into(),
        highlight_queries: vec![tree_sitter_css::HIGHLIGHTS_QUERY],
        precedence: Precedence::FirstWins,
        tags_queries: Vec::new(),
        markup_indent: false,
    }
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![css()]
}
