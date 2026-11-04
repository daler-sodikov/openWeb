//! HTML language support.
//!
//! M1: the tree-sitter HTML grammar and its bundled highlight query.
//! Embedded `<script>`/`<style>` highlighting (injections) and Vue/Svelte
//! arrive later (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// HTML (`.html`, `.htm`).
pub fn html() -> LanguageDef {
    LanguageDef {
        id: "html",
        name: "HTML",
        extensions: &["html", "htm"],
        language: tree_sitter_html::LANGUAGE.into(),
        highlight_queries: vec![tree_sitter_html::HIGHLIGHTS_QUERY],
        precedence: Precedence::FirstWins,
        tags_queries: Vec::new(),
        markup_indent: true,
    }
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![html()]
}
