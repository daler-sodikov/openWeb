//! JavaScript, TypeScript, JSX, TSX and JSON language support.
//!
//! M1: tree-sitter grammars and the highlight queries bundled with them.
//! `oxc`-based in-process features arrive with M3 (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// Symbols the bundled tags queries do not cover: type aliases and enums.
const EXTRA_TS_TAGS: &str = r#"
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
"#;

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
        tags_queries: vec![
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
            EXTRA_TS_TAGS,
        ],
        markup_indent: false,
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
        tags_queries: vec![
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
            EXTRA_TS_TAGS,
        ],
        markup_indent: true,
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
        tags_queries: vec![tree_sitter_javascript::TAGS_QUERY],
        markup_indent: true,
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
        tags_queries: Vec::new(),
        markup_indent: false,
    }
}

/// All languages of this crate.
pub fn languages() -> Vec<LanguageDef> {
    vec![typescript(), tsx(), javascript(), json()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrule_syntax::HighlightLanguage;

    #[test]
    fn extracts_typescript_symbols() {
        let lang = HighlightLanguage::new(tsx()).unwrap();
        let text = "export interface Props { a: number }\n\
                    type Mode = \"a\" | \"b\";\n\
                    enum Color { Red }\n\
                    export function App() {}\n\
                    const helper = () => 1;\n\
                    class Store {\n  load() {}\n}\n";
        let symbols: Vec<_> = lang
            .symbols(text)
            .into_iter()
            .map(|s| (s.name, s.kind, s.line, s.column))
            .collect();
        assert_eq!(
            symbols,
            vec![
                ("Props".into(), "interface".into(), 0, 17),
                ("Mode".into(), "type".into(), 1, 5),
                ("Color".into(), "enum".into(), 2, 5),
                ("App".into(), "function".into(), 3, 16),
                ("helper".into(), "function".into(), 4, 6),
                ("Store".into(), "class".into(), 5, 6),
                ("load".into(), "method".into(), 6, 2),
            ]
        );
    }

    #[test]
    fn json_has_no_symbols() {
        let lang = HighlightLanguage::new(json()).unwrap();
        assert!(!lang.has_symbols());
        assert!(lang.symbols("{}").is_empty());
    }
}
