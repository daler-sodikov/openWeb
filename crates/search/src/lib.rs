//! Fuzzy matching (Search Everywhere, Go to File/Symbol) and text search and
//! replace (Find/Replace in Files). Pure functions: callers provide the
//! candidates and texts, so unsaved buffers are searched like files.

mod fuzzy;
mod text;

pub use fuzzy::{FuzzyMatch, fuzzy_match, match_path};
pub use text::{
    Replacement, SearchError, TextMatch, TextQuery, find_matches, replace_all, replacement_for,
};
