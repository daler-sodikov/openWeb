//! JavaScript, TypeScript, JSX, TSX and JSON language support.
//!
//! Tree-sitter grammars with the queries bundled with them, plus small
//! queries of our own (marked below). `oxc`-based in-process features arrive
//! with M3 (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// Ours: symbols the bundled tags queries do not cover (type aliases, enums).
const EXTRA_TS_TAGS: &str = r#"
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
"#;

/// Pairs auto-closed in JavaScript-family languages (including backticks).
pub const JS_AUTO_CLOSE: &[&str] = &["()", "[]", "{}", "''", "\"\"", "``"];

/// TypeScript (`.ts`, `.mts`, `.cts`).
pub fn typescript() -> LanguageDef {
    LanguageDef::new(
        "typescript",
        "TypeScript",
        &["ts", "mts", "cts"],
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        Precedence::LastWins,
    )
    .aliases(&["ts"])
    .tags(vec![
        tree_sitter_javascript::TAGS_QUERY,
        tree_sitter_typescript::TAGS_QUERY,
        EXTRA_TS_TAGS,
    ])
    .auto_close(JS_AUTO_CLOSE)
}

/// TypeScript with JSX (`.tsx`).
pub fn tsx() -> LanguageDef {
    LanguageDef::new(
        "tsx",
        "TypeScript JSX",
        &["tsx"],
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        Precedence::LastWins,
    )
    .tags(vec![
        tree_sitter_javascript::TAGS_QUERY,
        tree_sitter_typescript::TAGS_QUERY,
        EXTRA_TS_TAGS,
    ])
    .markup_indent()
    .auto_close(JS_AUTO_CLOSE)
}

/// JavaScript and JSX (`.js`, `.mjs`, `.cjs`, `.jsx`).
pub fn javascript() -> LanguageDef {
    LanguageDef::new(
        "javascript",
        "JavaScript",
        &["js", "mjs", "cjs", "jsx"],
        tree_sitter_javascript::LANGUAGE.into(),
        vec![
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        Precedence::LastWins,
    )
    .aliases(&["js", "jsx", "mjs", "cjs"])
    .tags(vec![tree_sitter_javascript::TAGS_QUERY])
    .markup_indent()
    .auto_close(JS_AUTO_CLOSE)
}

/// JSON, with comments allowed (`.json`, `.jsonc`).
pub fn json() -> LanguageDef {
    LanguageDef::new(
        "json",
        "JSON",
        &["json", "jsonc"],
        tree_sitter_json::LANGUAGE.into(),
        vec![tree_sitter_json::HIGHLIGHTS_QUERY],
        Precedence::FirstWins,
    )
    .aliases(&["jsonc"])
    .auto_close(&["[]", "{}", "\"\""])
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
