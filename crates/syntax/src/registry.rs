//! Language definitions and the registry.

use std::path::Path;
use std::sync::Arc;

use tree_sitter::{Language, Query};

use crate::highlight::{TokenClass, class_for_capture};

/// Errors from building languages.
#[derive(Debug, thiserror::Error)]
pub enum SyntaxError {
    /// A query did not compile against its grammar.
    #[error("query for {language} is invalid: {message}")]
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

/// Bracket and quote pairs closed automatically when typing the opener.
pub const DEFAULT_AUTO_CLOSE: &[&str] = &["()", "[]", "{}", "''", "\"\""];

/// Static description of a language, provided by a `lang-*` crate.
pub struct LanguageDef {
    /// Stable id (`typescript`, `tsx`, `css`, ...).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Lowercase file extensions without the dot.
    pub extensions: &'static [&'static str],
    /// Other names used by injections (`ts`, `js`, `yml`, ...).
    pub aliases: &'static [&'static str],
    /// The tree-sitter grammar.
    pub language: Language,
    /// Highlight queries, concatenated in order.
    pub highlight_queries: Vec<&'static str>,
    /// Pattern precedence for overlapping captures of the same node.
    pub precedence: Precedence,
    /// Tags queries (`@name` + `@definition.<kind>`) for the symbol index;
    /// empty if the language has no symbols.
    pub tags_queries: Vec<&'static str>,
    /// Injection queries (`@injection.content` plus `@injection.language` or
    /// `#set! injection.language`): embedded languages.
    pub injection_queries: Vec<&'static str>,
    /// Whether Enter indents after opening markup tags.
    pub markup_indent: bool,
    /// Whether `@variable` captures are colored (CSS custom properties,
    /// SCSS/LESS variables) instead of plain text (JS identifiers).
    pub colored_variables: bool,
    /// Pairs closed automatically when typing the opener.
    pub auto_close: &'static [&'static str],
}

impl LanguageDef {
    /// A language with highlighting only; add features with the builder
    /// methods.
    pub fn new(
        id: &'static str,
        name: &'static str,
        extensions: &'static [&'static str],
        language: Language,
        highlight_queries: Vec<&'static str>,
        precedence: Precedence,
    ) -> Self {
        Self {
            id,
            name,
            extensions,
            aliases: &[],
            language,
            highlight_queries,
            precedence,
            tags_queries: Vec::new(),
            injection_queries: Vec::new(),
            markup_indent: false,
            colored_variables: false,
            auto_close: DEFAULT_AUTO_CLOSE,
        }
    }

    /// Sets the alias names.
    pub fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// Sets the tags queries.
    pub fn tags(mut self, queries: Vec<&'static str>) -> Self {
        self.tags_queries = queries;
        self
    }

    /// Sets the injection queries.
    pub fn injections(mut self, queries: Vec<&'static str>) -> Self {
        self.injection_queries = queries;
        self
    }

    /// Enables indentation after opening markup tags.
    pub fn markup_indent(mut self) -> Self {
        self.markup_indent = true;
        self
    }

    /// Colors `@variable` captures.
    pub fn colored_variables(mut self) -> Self {
        self.colored_variables = true;
        self
    }

    /// Sets the auto-close pairs.
    pub fn auto_close(mut self, pairs: &'static [&'static str]) -> Self {
        self.auto_close = pairs;
        self
    }
}

/// A language with its compiled queries.
pub struct HighlightLanguage {
    pub(crate) def: LanguageDef,
    pub(crate) query: Query,
    pub(crate) classes: Vec<Option<TokenClass>>,
    pub(crate) tags: Option<Query>,
    pub(crate) injections: Option<Query>,
}

impl std::fmt::Debug for HighlightLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightLanguage")
            .field("id", &self.def.id)
            .finish_non_exhaustive()
    }
}

fn compile(def: &LanguageDef, sources: &[&str], what: &str) -> Result<Option<Query>, SyntaxError> {
    if sources.is_empty() {
        return Ok(None);
    }
    Query::new(&def.language, &sources.join("\n"))
        .map(Some)
        .map_err(|err| SyntaxError::Query {
            language: def.id,
            message: format!("{what}: {err}"),
        })
}

impl HighlightLanguage {
    /// Compiles the queries of `def`.
    pub fn new(def: LanguageDef) -> Result<Self, SyntaxError> {
        let query = compile(&def, &def.highlight_queries, "highlights")?.ok_or_else(|| {
            SyntaxError::Query {
                language: def.id,
                message: "no highlight query".into(),
            }
        })?;
        let classes = query
            .capture_names()
            .iter()
            .map(|n| class_for_capture(n, def.colored_variables))
            .collect();
        let tags = compile(&def, &def.tags_queries, "tags")?;
        let injections = compile(&def, &def.injection_queries, "injections")?;
        Ok(Self {
            def,
            query,
            classes,
            tags,
            injections,
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

    /// Whether Enter indents after opening markup tags.
    pub fn markup_indent(&self) -> bool {
        self.def.markup_indent
    }

    /// Whether this language provides symbols.
    pub fn has_symbols(&self) -> bool {
        self.tags.is_some()
    }

    /// Pairs closed automatically when typing the opener.
    pub fn auto_close(&self) -> &'static [&'static str] {
        self.def.auto_close
    }

    fn answers_to(&self, name: &str) -> bool {
        self.def.id.eq_ignore_ascii_case(name)
            || self
                .def
                .aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(name))
    }
}

/// All known languages, looked up by file extension or name.
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

    /// Number of registered languages.
    pub fn len(&self) -> usize {
        self.languages.len()
    }

    /// Whether no language is registered.
    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }

    /// The language for a file path, by extension (case-insensitive).
    pub fn for_path(&self, path: &Path) -> Option<Arc<HighlightLanguage>> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        self.languages
            .iter()
            .find(|l| l.def.extensions.contains(&ext.as_str()))
            .cloned()
    }

    /// The language with this id or alias (case-insensitive), e.g. the name
    /// in a Markdown code fence or a `lang` attribute.
    pub fn for_name(&self, name: &str) -> Option<Arc<HighlightLanguage>> {
        let name = name.trim();
        self.languages.iter().find(|l| l.answers_to(name)).cloned()
    }
}
