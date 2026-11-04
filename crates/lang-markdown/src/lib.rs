//! Markdown language support. The block grammar parses the document; inline
//! content (emphasis, code spans, links) is parsed by the inline grammar
//! through an injection, and fenced code blocks by the language named in
//! the fence (`ts`, `css`, `yaml`, ...), all with the queries bundled with
//! the grammars.

use ferrule_syntax::{LanguageDef, Precedence};

/// Markdown (`.md`, `.markdown`).
pub fn markdown() -> LanguageDef {
    LanguageDef::new(
        "markdown",
        "Markdown",
        &["md", "markdown"],
        tree_sitter_md::LANGUAGE.into(),
        vec![tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        Precedence::LastWins,
    )
    .aliases(&["md"])
    .injections(vec![tree_sitter_md::INJECTION_QUERY_BLOCK])
    .auto_close(&["()", "[]", "``", "\"\""])
}

/// Inline Markdown; only used through the injection from [`markdown`].
pub fn markdown_inline() -> LanguageDef {
    LanguageDef::new(
        "markdown_inline",
        "Markdown (inline)",
        &[],
        tree_sitter_md::INLINE_LANGUAGE.into(),
        vec![tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
        Precedence::LastWins,
    )
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![markdown(), markdown_inline()]
}
