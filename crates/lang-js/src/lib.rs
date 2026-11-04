//! JavaScript, TypeScript, JSX, TSX and JSON language support.
//!
//! M1: tree-sitter grammars and the highlight queries bundled with them.
//! `oxc`-based in-process features arrive with M3 (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// TypeScript (`.ts`, `.mts`, `.cts`).
pub fn typescript() -> LanguageDef {
    LanguageDef {
        id: "typescript",
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlight_queries: vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        precedence: Precedence::LastWins,
    }
}

/// TypeScript with JSX (`.tsx`).
pub fn tsx() -> LanguageDef {
    LanguageDef {
        id: "tsx",
        name: "TypeScript JSX",
        extensions: &["tsx"],
        language: tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlight_queries: vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        precedence: Precedence::LastWins,
    }
}

/// JavaScript and JSX (`.js`, `.mjs`, `.cjs`, `.jsx`).
pub fn javascript() -> LanguageDef {
    LanguageDef {
        id: "javascript",
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        language: tree_sitter_javascript::LANGUAGE.into(),
        highlight_queries: vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        precedence: Precedence::LastWins,
    }
}

/// JSON, with comments allowed (`.json`, `.jsonc`).
pub fn json() -> LanguageDef {
    LanguageDef {
        id: "json",
        name: "JSON",
        extensions: &["json", "jsonc"],
        language: tree_sitter_json::LANGUAGE.into(),
        highlight_queries: vec![tree_sitter_json::HIGHLIGHTS_QUERY],
        precedence: Precedence::FirstWins,
    }
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![typescript(), tsx(), javascript(), json()]
}
