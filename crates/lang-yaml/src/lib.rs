//! YAML language support: the tree-sitter grammar and its bundled highlight
//! query.

use ferrule_syntax::{LanguageDef, Precedence};

/// YAML (`.yaml`, `.yml`).
pub fn yaml() -> LanguageDef {
    LanguageDef::new(
        "yaml",
        "YAML",
        &["yaml", "yml"],
        tree_sitter_yaml::LANGUAGE.into(),
        vec![tree_sitter_yaml::HIGHLIGHTS_QUERY],
        Precedence::LastWins,
    )
    .aliases(&["yml"])
    .auto_close(&["[]", "{}", "''", "\"\""])
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![yaml()]
}
