//! Language definitions and the extension registry.

use std::path::Path;
use std::sync::Arc;

use tree_sitter::{Language, Query};

use crate::highlight::{TokenClass, class_for_capture};

/// Errors from building languages.
#[derive(Debug, thiserror::Error)]
pub enum SyntaxError {
    /// A highlight query did not compile against its grammar.
    #[error("highlight query for {language} is invalid: {message}")]
    Query {
        /// Language id.
        language: &'static str,
        /// Compiler message.
        message: String,
    },
    /// The grammar's ABI version is not supported by the tree-sitter runtime.
    #[error("grammar for {language} is incompatible: {message}")]
    Grammar {
        /// Language id.
        language: &'static str,
        /// Error message.
        message: String,
    },
}

/// Which pattern wins when several patterns of a highlight query capture the
/// same node. The bundled grammar queries are written for different
/// conventions (e.g. JavaScript lists generic patterns first, JSON lists them
/// last), so each language states its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precedence {
    /// The earliest matching pattern wins.
    FirstWins,
    /// The latest matching pattern wins.
    LastWins,
}

/// Static description of a language, provided by a `lang-*` crate.
pub struct LanguageDef {
    /// Stable id (`typescript`, `tsx`, `css`, ...).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Lowercase file extensions without the dot.
    pub extensions: &'static [&'static str],
    /// The tree-sitter grammar.
    pub language: Language,
    /// Highlight queries, concatenated in order.
    pub highlight_queries: Vec<&'static str>,
    /// Pattern precedence for overlapping captures of the same node.
    pub precedence: Precedence,
}

/// A language with its compiled highlight query.
pub struct HighlightLanguage {
    pub(crate) def: LanguageDef,
    pub(crate) query: Query,
    pub(crate) classes: Vec<Option<TokenClass>>,
}

impl std::fmt::Debug for HighlightLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightLanguage")
            .field("id", &self.def.id)
            .finish_non_exhaustive()
    }
}

impl HighlightLanguage {
    /// Compiles the highlight query of `def`.
    pub fn new(def: LanguageDef) -> Result<Self, SyntaxError> {
        let source = def.highlight_queries.join("\n");
        let query = Query::new(&def.language, &source).map_err(|err| SyntaxError::Query {
            language: def.id,
            message: err.to_string(),
        })?;
        let classes = query
            .capture_names()
            .iter()
            .map(|n| class_for_capture(n))
            .collect();
        Ok(Self {
            def,
            query,
            classes,
        })
    }

    /// Stable language id.
    pub fn id(&self) -> &'static str {
        self.def.id
    }

    /// Display name.
    pub fn name(&self) -> &'static str {
        self.def.name
    }
}

/// All known languages, looked up by file extension.
#[derive(Debug, Default)]
pub struct LanguageRegistry {
    languages: Vec<Arc<HighlightLanguage>>,
}

impl LanguageRegistry {
    /// Compiles and registers a language.
    pub fn register(&mut self, def: LanguageDef) -> Result<(), SyntaxError> {
        self.languages.push(Arc::new(HighlightLanguage::new(def)?));
        Ok(())
    }

    /// The language for a file path, by extension (case-insensitive).
    pub fn for_path(&self, path: &Path) -> Option<Arc<HighlightLanguage>> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        self.languages
            .iter()
            .find(|l| l.def.extensions.contains(&ext.as_str()))
            .cloned()
    }
}
