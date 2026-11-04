//! Serializable data types exchanged between the Rust backend and the
//! TypeScript frontend.
//!
//! Every type here derives [`specta::Type`] so that `ferrule-app` can export
//! TypeScript bindings for it. This crate holds no logic; see
//! `docs/ARCHITECTURE.md`, section 3, for the conventions.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Version of the IPC protocol. Bump on any breaking change to a command,
/// event or DTO so that a mismatched frontend refuses to run.
pub const PROTOCOL_VERSION: u32 = 3;

/// Operating system the backend runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    /// Linux and other freedesktop systems.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
    /// Any other target.
    Other,
}

/// Static information about the running application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// Product name shown in the UI.
    pub name: String,
    /// Application version (semver).
    pub version: String,
    /// IPC protocol version the backend speaks; see [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Operating system the backend runs on.
    pub platform: Platform,
}

/// What the frontend rendered after mounting. Sent once at startup; the
/// backend uses it to verify that the window, the bundled frontend and the
/// IPC round trip all work (the e2e smoke test depends on this).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrontendReadyReport {
    /// Product name as rendered in the DOM.
    pub rendered_app_name: String,
    /// `document.title` after mounting.
    pub document_title: String,
    /// Protocol version the frontend bindings were generated for.
    pub protocol_version: u32,
}

/// Category of an [`IpcError`]. The frontend switches on this, never on the
/// message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IpcErrorKind {
    /// The referenced entity does not exist.
    NotFound,
    /// An argument was malformed or out of range.
    InvalidArgument,
    /// The request was superseded or cancelled.
    Cancelled,
    /// A file system or process error occurred.
    Io,
    /// The request conflicts with current state (e.g. stale version).
    Conflict,
    /// A bug or unexpected condition in the backend.
    Internal,
}

/// Error returned by every fallible IPC command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{kind:?}: {message}")]
pub struct IpcError {
    /// Machine-readable category.
    pub kind: IpcErrorKind,
    /// Human-readable description, for logs and display only.
    pub message: String,
}

impl IpcError {
    /// Creates an error of the given kind.
    pub fn new(kind: IpcErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Identifier of an open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub struct DocId(pub u32);

/// The opened project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// Folder name.
    pub name: String,
    /// Absolute root path, for display.
    pub root: String,
}

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    /// A directory.
    Directory,
    /// A file.
    File,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    /// File name.
    pub name: String,
    /// Project-relative path with `/` separators.
    pub path: String,
    /// Directory or file.
    pub kind: EntryKind,
}

/// A document opened in the editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenedDocument {
    /// Document id.
    pub doc_id: DocId,
    /// Project-relative path.
    pub path: String,
    /// Full text (`\n` line endings).
    pub text: String,
    /// Current version; one step per transaction.
    pub version: u32,
    /// Language display name, if the file type is known.
    pub language: Option<String>,
    /// Whether there are unsaved changes.
    pub dirty: bool,
    /// Whether unsaved text was restored from a recovery file.
    pub restored: bool,
    /// Detected indentation unit (a tab or spaces), inserted by Tab.
    pub indent_unit: String,
}

/// Replace the UTF-16 range `from..to` with `insert`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    /// Start offset (UTF-16 code units).
    pub from: u32,
    /// End offset (UTF-16 code units).
    pub to: u32,
    /// Inserted text.
    pub insert: String,
}

/// One selection range; `anchor == head` is a caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRange {
    /// Fixed end (UTF-16 offset).
    pub anchor: u32,
    /// Moving end (UTF-16 offset).
    pub head: u32,
}

/// A multi-range selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EditorSelection {
    /// Ranges, sorted and non-overlapping.
    pub ranges: Vec<SelectionRange>,
    /// Index of the main range.
    pub main: u32,
}

/// Kind of user action behind a transaction (undo grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EditKind {
    /// Typing.
    Typing,
    /// Backspace/Delete.
    Deleting,
    /// Anything else (paste, cut, drop).
    Other,
}

/// One view transaction: edits relative to the text before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeTransaction {
    /// Sorted, non-overlapping edits.
    pub edits: Vec<TextEdit>,
    /// Kind of user action.
    pub kind: EditKind,
    /// Selection before the change.
    pub selection_before: EditorSelection,
    /// Selection after the change.
    pub selection_after: EditorSelection,
}

/// Version and modification state of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocStatus {
    /// Current version.
    pub version: u32,
    /// Whether there are unsaved changes.
    pub dirty: bool,
}

/// Edits computed by the backend (undo/redo), to apply in the view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUpdate {
    /// One edit list per version step, each relative to the text before it.
    pub updates: Vec<Vec<TextEdit>>,
    /// Selection after the update.
    pub selection: EditorSelection,
    /// Status after the update.
    pub status: DocStatus,
}

/// Direction for cloning a caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CaretDirection {
    /// One line up.
    Up,
    /// One line down.
    Down,
}

/// Highlight category of a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum TokenClass {
    Keyword,
    String,
    StringSpecial,
    Escape,
    Number,
    Constant,
    Comment,
    Function,
    Type,
    VariableBuiltin,
    Parameter,
    Property,
    Operator,
    Punctuation,
    Tag,
    Attribute,
}

/// A highlighted UTF-16 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HighlightSpan {
    /// Start offset.
    pub from: u32,
    /// End offset.
    pub to: u32,
    /// Token class.
    pub class: TokenClass,
}

/// Highlights for a range of a document version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Highlights {
    /// Version the spans belong to.
    pub version: u32,
    /// Sorted, non-overlapping spans.
    pub spans: Vec<HighlightSpan>,
}

/// Actions that can be bound to keys or run from Search Everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum KeyAction {
    Save,
    Undo,
    Redo,
    AddCaretAbove,
    AddCaretBelow,
    SelectNextOccurrence,
    SelectAllOccurrences,
    CloseTab,
    SearchEverywhere,
    GotoFile,
    GotoSymbol,
    GotoAction,
    GotoLine,
    RecentFiles,
    FindInFiles,
    ReplaceInFiles,
    OpenFolder,
}

/// Modifier used in double-tap gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Modifier {
    /// Ctrl.
    Control,
    /// Alt / Option.
    Alt,
    /// Shift.
    Shift,
}

/// How a binding is triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum KeyStroke {
    /// A key chord in CodeMirror notation (`Mod-Shift-z`).
    Chord {
        /// The chord.
        keys: String,
    },
    /// Press `modifier` twice, keep it held, then press `key`.
    DoubleModifier {
        /// The modifier.
        modifier: Modifier,
        /// The key (`ArrowUp`).
        key: String,
    },
    /// Tap `modifier` twice quickly (double Shift).
    DoubleTap {
        /// The modifier.
        modifier: Modifier,
    },
}

/// A key binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyBinding {
    /// The action.
    pub action: KeyAction,
    /// The key stroke.
    pub stroke: KeyStroke,
}

/// Search Everywhere tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum SearchTab {
    All,
    Files,
    Symbols,
    Actions,
}

/// One Search Everywhere result. `positions` are matched char indices (in
/// `path` for files, in `name` for symbols and actions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
#[allow(missing_docs)]
pub enum SearchItem {
    #[serde(rename_all = "camelCase")]
    File { path: String, positions: Vec<u32> },
    #[serde(rename_all = "camelCase")]
    Symbol {
        name: String,
        kind: String,
        path: String,
        line: u32,
        column: u32,
        positions: Vec<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Action {
        action: KeyAction,
        name: String,
        positions: Vec<u32>,
    },
}

/// Search Everywhere results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    /// Ranked items; for the All tab grouped as files, symbols, actions.
    pub items: Vec<SearchItem>,
    /// Whether the index is still being built (results may be incomplete).
    pub indexing: bool,
}

/// A Find/Replace in Files query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextSearch {
    /// Text or regex.
    pub pattern: String,
    /// Regex mode.
    pub regex: bool,
    /// Match case.
    pub case_sensitive: bool,
    /// Whole words only.
    pub whole_word: bool,
    /// Replacement for previews (`$1` expands in regex mode); `None` to only find.
    pub replacement: Option<String>,
}

/// One match of Find in Files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FindMatch {
    /// Project-relative path.
    pub path: String,
    /// Zero-based line.
    pub line: u32,
    /// Line text (truncated).
    pub line_text: String,
    /// Match start in the line (UTF-16).
    pub column_start: u32,
    /// Match end in the line (UTF-16).
    pub column_end: u32,
    /// Replacement preview, when a replacement was given.
    pub replacement: Option<String>,
}

/// Find in Files results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FindResults {
    /// Matches, by path then position.
    pub matches: Vec<FindMatch>,
    /// Whether the match limit was reached.
    pub truncated: bool,
    /// Files searched.
    pub files_searched: u32,
    /// Whether the index is still being built.
    pub indexing: bool,
}

/// Replacement edits for an open document, applied by the view so that they
/// go through the normal edit path (and undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReplace {
    /// Document.
    pub doc_id: DocId,
    /// Version the edits are based on.
    pub version: u32,
    /// Sorted edits.
    pub edits: Vec<TextEdit>,
}

/// Result of Replace in Files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceResult {
    /// Closed files rewritten on disk.
    pub files_written: u32,
    /// Total replacements (files + open documents).
    pub replacements: u32,
    /// Edits for open documents.
    pub open_documents: Vec<DocumentReplace>,
}

/// Result of reloading a document from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReload {
    /// Edits (one list per version step); empty if the text was unchanged.
    pub updates: Vec<Vec<TextEdit>>,
    /// Status afterwards.
    pub status: DocStatus,
}

/// What happened to a path on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FsChangeKind {
    /// Created or modified (the path exists).
    Changed,
    /// Removed (the path no longer exists).
    Removed,
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsChange {
    /// Project-relative path.
    pub path: String,
    /// What happened.
    pub kind: FsChangeKind,
}

/// Payload of the file-change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FsChangeBatch {
    /// Changed paths.
    pub changes: Vec<FsChange>,
}

/// Payload of the index-status event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexState {
    /// Whether indexing is in progress.
    pub indexing: bool,
    /// Indexed files.
    pub files: u32,
    /// Indexed symbols.
    pub symbols: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_serializes_camel_case() {
        let info = AppInfo {
            name: "Ferrule".into(),
            version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            platform: Platform::Macos,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "Ferrule",
                "version": "0.1.0",
                "protocolVersion": PROTOCOL_VERSION,
                "platform": "macos",
            })
        );
    }

    #[test]
    fn ipc_error_round_trips() {
        let err = IpcError::new(IpcErrorKind::NotFound, "no such file");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, r#"{"kind":"notFound","message":"no such file"}"#);
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, err);
        assert_eq!(err.to_string(), "NotFound: no such file");
    }
}
