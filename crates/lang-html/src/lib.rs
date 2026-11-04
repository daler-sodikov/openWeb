//! HTML, Vue and Svelte language support. `<script>` and `<style>` blocks
//! are highlighted with the JavaScript/TypeScript and CSS/SCSS/LESS
//! languages through injections (resolved by name in the registry).
//!
//! Vue single-file components use the HTML grammar (no Vue grammar
//! compatible with our tree-sitter version exists on crates.io), so
//! template expressions (`{{ }}`, directives) are not highlighted yet.

use ferrule_syntax::{LanguageDef, Precedence};

/// Ours: embedded scripts and styles, honouring `lang="ts"`,
/// `lang="scss"` and `lang="less"`. The first matching pattern decides.
pub const SCRIPT_STYLE_INJECTIONS: &str = r#"
((script_element
  (start_tag (attribute (attribute_name) @_attr (quoted_attribute_value (attribute_value) @_lang)))
  (raw_text) @injection.content)
 (#eq? @_attr "lang")
 (#any-of? @_lang "ts" "typescript")
 (#set! injection.language "typescript"))

((script_element (raw_text) @injection.content)
 (#set! injection.language "javascript"))

((style_element
  (start_tag (attribute (attribute_name) @_attr (quoted_attribute_value (attribute_value) @_lang)))
  (raw_text) @injection.content)
 (#eq? @_attr "lang")
 (#any-of? @_lang "scss" "sass")
 (#set! injection.language "scss"))

((style_element
  (start_tag (attribute (attribute_name) @_attr (quoted_attribute_value (attribute_value) @_lang)))
  (raw_text) @injection.content)
 (#eq? @_attr "lang")
 (#eq? @_lang "less")
 (#set! injection.language "less"))

((style_element (raw_text) @injection.content)
 (#set! injection.language "css"))
"#;

/// Ours: Svelte `{expressions}` are JavaScript.
const SVELTE_EXPRESSIONS: &str = r#"
((svelte_raw_text) @injection.content
 (#set! injection.language "javascript"))
"#;

/// Pairs auto-closed in markup.
pub const MARKUP_AUTO_CLOSE: &[&str] = &["''", "\"\"", "{}"];

/// HTML (`.html`, `.htm`).
pub fn html() -> LanguageDef {
    LanguageDef::new(
        "html",
        "HTML",
        &["html", "htm"],
        tree_sitter_html::LANGUAGE.into(),
        vec![tree_sitter_html::HIGHLIGHTS_QUERY],
        Precedence::FirstWins,
    )
    .injections(vec![SCRIPT_STYLE_INJECTIONS])
    .markup_indent()
    .auto_close(&["''", "\"\""])
}

/// Vue single-file components (`.vue`), HTML grammar plus injections.
pub fn vue() -> LanguageDef {
    LanguageDef::new(
        "vue",
        "Vue",
        &["vue"],
        tree_sitter_html::LANGUAGE.into(),
        vec![tree_sitter_html::HIGHLIGHTS_QUERY],
        Precedence::FirstWins,
    )
    .injections(vec![SCRIPT_STYLE_INJECTIONS])
    .markup_indent()
    .auto_close(MARKUP_AUTO_CLOSE)
}

/// Svelte components (`.svelte`). The Svelte query extends the HTML one
/// (written for "later pattern wins").
pub fn svelte() -> LanguageDef {
    LanguageDef::new(
        "svelte",
        "Svelte",
        &["svelte"],
        tree_sitter_svelte_ng::LANGUAGE.into(),
        vec![
            tree_sitter_html::HIGHLIGHTS_QUERY,
            tree_sitter_svelte_ng::HIGHLIGHTS_QUERY,
        ],
        Precedence::LastWins,
    )
    .injections(vec![SCRIPT_STYLE_INJECTIONS, SVELTE_EXPRESSIONS])
    .markup_indent()
    .auto_close(MARKUP_AUTO_CLOSE)
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![html(), vue(), svelte()]
}
