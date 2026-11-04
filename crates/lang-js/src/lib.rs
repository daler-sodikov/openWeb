//! JavaScript, TypeScript, JSX, TSX and JSON language support.
//!
//! Tree-sitter grammars with the queries bundled with them, plus small
//! queries of our own (marked below), and the definition of the TypeScript
//! language server ([`typescript_server`]). `oxc`-based in-process features
//! arrive with M3b (`docs/ROADMAP.md`).

use ferrule_syntax::{LanguageDef, Precedence};

/// Ours: symbols the bundled tags queries do not cover (type aliases, enums).
const EXTRA_TS_TAGS: &str = r#"
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
"#;

/// Ours, appended last (last pattern wins): the bundled queries color every
/// capitalized identifier as a type/constructor, which also hits function
/// names such as React components. Declarations and calls of functions keep
/// the function color.
const FUNCTION_OVERRIDES: &str = r#"
(function_declaration name: (identifier) @function)
(generator_function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function)
(variable_declarator
  name: (identifier) @function
  value: [(arrow_function) (function_expression)])
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
            FUNCTION_OVERRIDES,
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
            FUNCTION_OVERRIDES,
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
            FUNCTION_OVERRIDES,
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

/// An external language server: how to find and start it, and which files
/// it serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageServerDef {
    /// Display name in the UI ("TypeScript").
    pub name: &'static str,
    /// Executable name looked up in `node_modules/.bin` and on `PATH`.
    pub program: &'static str,
    /// Arguments.
    pub args: &'static [&'static str],
    /// User setting naming an explicit executable.
    pub setting: &'static str,
    /// What to tell the user when it is not installed.
    pub install_hint: &'static str,
    /// File extension (lowercase) → LSP language id.
    pub languages: &'static [(&'static str, &'static str)],
}

impl LanguageServerDef {
    /// The LSP language id for a file extension, if this server handles it.
    pub fn language_id(&self, extension: &str) -> Option<&'static str> {
        let extension = extension.to_ascii_lowercase();
        self.languages
            .iter()
            .find(|(ext, _)| *ext == extension)
            .map(|(_, id)| *id)
    }
}

/// `typescript-language-server` (Apache-2.0), which drives the `tsserver` of
/// the project's or a global TypeScript. Started with `--stdio`.
pub fn typescript_server() -> LanguageServerDef {
    LanguageServerDef {
        name: "TypeScript",
        program: "typescript-language-server",
        args: &["--stdio"],
        setting: "typescript.serverPath",
        install_hint: "install it with `bun add -g typescript-language-server typescript` \
                       (or npm/pnpm), or set typescript.serverPath in settings.json",
        languages: &[
            ("ts", "typescript"),
            ("mts", "typescript"),
            ("cts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("mjs", "javascript"),
            ("cjs", "javascript"),
            ("jsx", "javascriptreact"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrule_syntax::HighlightLanguage;

    #[test]
    fn typescript_server_language_ids() {
        let server = typescript_server();
        assert_eq!(server.language_id("TSX"), Some("typescriptreact"));
        assert_eq!(server.language_id("mjs"), Some("javascript"));
        assert_eq!(server.language_id("jsx"), Some("javascriptreact"));
        assert_eq!(server.language_id("json"), None);
    }

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
