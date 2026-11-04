//! Rope-based text buffer: edits, versions, selections, multi-cursor, undo/redo
//! and offset conversions. Pure, no IO.
//!
//! Positions in the public API are **UTF-16 code-unit offsets** (the unit used
//! by CodeMirror, JS strings and LSP), see `docs/ARCHITECTURE.md`, 3.3. Text
//! inside a [`Document`] always uses `\n` line endings; callers normalize with
//! [`normalize_line_endings`] when loading and restore the original ending with
//! [`LineEnding::apply`] when saving.

mod document;
mod history;
mod indent;
mod multicursor;
mod selection;
mod text;

pub use document::{
    ApplyOutcome, BufferError, ByteEdit, Document, Edit, EditKind, Point, Transaction,
};
pub use history::UndoOutcome;
pub use indent::{IndentRules, detect_indent_unit};
pub use multicursor::CaretDirection;
pub use ropey::Rope;
pub use selection::{SelRange, Selection};
pub use text::{LineEnding, normalize_line_endings};
