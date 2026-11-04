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
pub const PROTOCOL_VERSION: u32 = 2;

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

/// Editor actions that have key bindings.
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
}

/// Modifier used in double-tap gestures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Modifier {
    /// Ctrl.
    Control,
    /// Alt / Option.
    Alt,
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
