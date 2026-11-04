//! CSS, SCSS and LESS language support: tree-sitter grammars and their
//! bundled highlight queries. Variables (`--custom`, `$scss`, `@less`) are
//! colored.

use ferrule_syntax::{LanguageDef, Precedence};

/// Ours, appended last: custom properties stay variables when the base CSS
/// query is combined with a "later pattern wins" extension query.
const CUSTOM_PROPERTIES: &str = r#"
((property_name) @variable (#match? @variable "^(--|\\$|@)"))
((plain_value) @variable (#match? @variable "^--"))
"#;

/// Pairs auto-closed in style sheets.
pub const CSS_AUTO_CLOSE: &[&str] = &["()", "[]", "{}", "''", "\"\""];

/// CSS (`.css`).
pub fn css() -> LanguageDef {
    LanguageDef::new(
        "css",
        "CSS",
        &["css"],
        tree_sitter_css::LANGUAGE.into(),
        vec![tree_sitter_css::HIGHLIGHTS_QUERY],
        Precedence::FirstWins,
    )
    .aliases(&["postcss"])
    .colored_variables()
    .auto_close(CSS_AUTO_CLOSE)
}

/// SCSS (`.scss`).
pub fn scss() -> LanguageDef {
    LanguageDef::new(
        "scss",
        "SCSS",
        &["scss"],
        tree_sitter_scss::language(),
        // The SCSS query extends the CSS one.
        vec![
            tree_sitter_css::HIGHLIGHTS_QUERY,
            tree_sitter_scss::HIGHLIGHTS_QUERY,
            CUSTOM_PROPERTIES,
        ],
        Precedence::LastWins,
    )
    .aliases(&["sass"])
    .colored_variables()
    .auto_close(CSS_AUTO_CLOSE)
}

/// LESS (`.less`).
pub fn less() -> LanguageDef {
    LanguageDef::new(
        "less",
        "LESS",
        &["less"],
        tree_sitter_less::language(),
        // The LESS query extends the CSS one.
        vec![
            tree_sitter_css::HIGHLIGHTS_QUERY,
            tree_sitter_less::HIGHLIGHTS_QUERY,
            CUSTOM_PROPERTIES,
        ],
        Precedence::LastWins,
    )
    .colored_variables()
    .auto_close(CSS_AUTO_CLOSE)
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![css(), scss(), less()]
}
