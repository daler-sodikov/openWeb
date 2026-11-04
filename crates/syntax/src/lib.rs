//! Shared tree-sitter machinery: language definitions, a registry keyed by
//! file extension, incremental parsing straight from the rope, and
//! highlighting as flat token spans.
//!
//! When several patterns capture the same node, the language's
//! [`Precedence`] decides which wins; inner nodes always override outer ones.
//! Captures that map to no token class (e.g. `@variable`) reset the color of
//! their node to plain text.

mod highlight;
mod registry;
mod structure;
mod symbols;
mod tree;

pub use highlight::{HighlightSpan, TokenClass, render_spans};
pub use registry::{
    DEFAULT_AUTO_CLOSE, HighlightLanguage, LanguageDef, LanguageRegistry, Precedence, SyntaxError,
};
pub use structure::{BracketPair, Crumb, FoldRange};
pub use symbols::SymbolDef;
pub use tree::SyntaxTree;

use std::sync::Arc;

/// Highlights a whole text with `def` and renders the spans with
/// [`render_spans`]. Used by the golden tests of the `lang-*` crates;
/// `registry` resolves embedded languages.
pub fn render_highlights(
    def: LanguageDef,
    text: &str,
    registry: Option<Arc<LanguageRegistry>>,
) -> Result<String, SyntaxError> {
    let language = Arc::new(HighlightLanguage::new(def)?);
    let mut tree = SyntaxTree::new(language)?;
    if let Some(registry) = registry {
        tree = tree.with_registry(registry);
    }
    let rope = ferrule_buffer::Rope::from_str(text);
    let spans = tree.highlight(&rope, 0..rope.len_bytes());
    Ok(render_spans(&rope, &spans))
}
