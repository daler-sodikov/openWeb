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
pub const PROTOCOL_VERSION: u32 = 8;

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
    /// Excluded by `.gitignore` (or inside `node_modules`); shown greyed.
    pub ignored: bool,
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
    /// Indentation unit (a tab or spaces), from EditorConfig or detected.
    pub indent_unit: String,
    /// Display width of a tab character.
    pub tab_width: u32,
    /// Pairs closed automatically when typing the opener (`"()"`, `"''"`).
    pub auto_close: Vec<String>,
    /// Library code (outside the project or in `node_modules`): view only.
    pub read_only: bool,
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
    Variable,
    Heading,
    Emphasis,
    Strong,
    Link,
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
    SplitRight,
    FoldCollapse,
    FoldExpand,
    FoldCollapseAll,
    FoldExpandAll,
    GotoMatchingBrace,
    QuickDocumentation,
    GotoDeclaration,
    GotoTypeDeclaration,
    GotoImplementation,
    ToggleProblems,
    RestartTypeScriptServer,
    Completion,
    ParameterInfo,
    FindUsages,
    Rename,
    ShowIntentions,
    OptimizeImports,
    SortImports,
    ReformatCode,
    ToggleTerminal,
    ToggleCommit,
    ToggleGitLog,
    GitBranches,
    ShowDiff,
    ToggleBlame,
    RollbackHunk,
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

/// One breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    /// Display label.
    pub label: String,
    /// Syntax node kind.
    pub kind: String,
    /// Where the node starts (UTF-16).
    pub offset: u32,
}

/// A matched bracket pair (UTF-16 ranges).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BracketMatch {
    /// Opening bracket start.
    pub open_from: u32,
    /// Opening bracket end.
    pub open_to: u32,
    /// Closing bracket start.
    pub close_from: u32,
    /// Closing bracket end.
    pub close_to: u32,
}

/// Structural information at the caret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CaretInfo {
    /// Version the information belongs to.
    pub version: u32,
    /// Breadcrumbs, outermost first.
    pub breadcrumbs: Vec<Crumb>,
    /// Bracket pair touching the caret.
    pub brackets: Option<BracketMatch>,
}

/// A foldable range; the text between `from` and `to` is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FoldSpan {
    /// Start (UTF-16).
    pub from: u32,
    /// End (UTF-16).
    pub to: u32,
}

/// Folding ranges of a document version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FoldRanges {
    /// Version the ranges belong to.
    pub version: u32,
    /// Ranges sorted by start, at most one per starting line.
    pub ranges: Vec<FoldSpan>,
}

/// Result of saving: edits applied on save (EditorConfig trimming and final
/// newline, one undoable step) and the status afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    /// Edits (one list per version step); empty if nothing changed.
    pub updates: Vec<Vec<TextEdit>>,
    /// Status after saving.
    pub status: DocStatus,
}

/// What to open at startup (`ferrule <folder> [file]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartupTarget {
    /// Absolute folder path.
    pub folder: String,
    /// File to open, relative to the folder (`/` separators).
    pub file: Option<String>,
}

/// Lifecycle state of a language server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ServerStateKind {
    /// Not started yet (no file of its languages opened).
    NotStarted,
    /// Spawning and initializing.
    Starting,
    /// Running and reporting progress (loading the project).
    Indexing,
    /// Running and idle.
    Ready,
    /// Died; restarting with backoff.
    Restarting,
    /// Gave up after repeated failures, or refused to run.
    Error,
    /// The server (or Node.js) is not installed or not found.
    NotFound,
    /// Stopped (project closed).
    Stopped,
}

/// Status of a language server, for the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// Server name ("TypeScript").
    pub name: String,
    /// State.
    pub state: ServerStateKind,
    /// Short status-bar text.
    pub message: String,
    /// Longer text for the tooltip: what was found where, or how to fix it.
    pub detail: String,
}

/// Severity of a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// An error.
    Error,
    /// A warning.
    Warning,
    /// Information.
    Information,
    /// A hint.
    Hint,
}

/// A problem reported by a language server, with a display location
/// (zero-based line, UTF-16 column).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// Severity.
    pub severity: Severity,
    /// Message.
    pub message: String,
    /// Reporting tool (`ts`, `eslint`).
    pub source: Option<String>,
    /// Code (`2304`).
    pub code: Option<String>,
    /// Start line.
    pub line: u32,
    /// Start column.
    pub column: u32,
    /// End line.
    pub end_line: u32,
    /// End column.
    pub end_column: u32,
}

/// All problems of one file (empty when the file has none left).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileProblems {
    /// Project-relative path.
    pub path: String,
    /// Problems in server order.
    pub problems: Vec<Problem>,
}

/// A diagnostic of an open document as a UTF-16 range of a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSpan {
    /// Start offset.
    pub from: u32,
    /// End offset.
    pub to: u32,
    /// Severity.
    pub severity: Severity,
    /// Message.
    pub message: String,
    /// Reporting tool.
    pub source: Option<String>,
    /// Code.
    pub code: Option<String>,
    /// Unused code (shown faded).
    pub unnecessary: bool,
    /// Deprecated symbol (shown struck through).
    pub deprecated: bool,
}

/// Diagnostics of an open document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocDiagnostics {
    /// Version the offsets refer to.
    pub version: u32,
    /// Sorted by start offset.
    pub items: Vec<DiagnosticSpan>,
}

/// Hover / Quick Documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HoverResult {
    /// Version the range refers to.
    pub version: u32,
    /// Start of the hovered symbol.
    pub from: u32,
    /// End of the hovered symbol.
    pub to: u32,
    /// The documentation as sanitized HTML rendered from Markdown in Rust
    /// (no scripts, no raw HTML, only http/https/mailto links).
    pub html: String,
}

/// What to navigate to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NavKind {
    /// Go to Declaration (definition).
    Definition,
    /// Go to Type Declaration.
    TypeDefinition,
    /// Go to Implementation.
    Implementation,
}

/// A navigation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NavTarget {
    /// Project-relative path, or an absolute path when outside the project.
    pub path: String,
    /// Whether the target is inside the project (only those can be opened).
    pub in_project: bool,
    /// Zero-based line.
    pub line: u32,
    /// Zero-based UTF-16 column.
    pub column: u32,
    /// The target line's text, trimmed.
    pub preview: String,
}

/// One typing-path timing from the view (`FERRULE_TYPING_TRACE`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TraceSample {
    /// Stage name (`view.echo_frame`).
    pub stage: String,
    /// Milliseconds since the key event.
    pub ms: f64,
}

/// A UTF-16 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextRange {
    /// Start offset.
    pub from: u32,
    /// End offset.
    pub to: u32,
}

/// One completion item as shown in the popup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemView {
    /// Index in the session (for details and accepting).
    pub index: u32,
    /// Label.
    pub label: String,
    /// Kind (`function`, `variable`, `class`, `keyword`, …).
    pub kind: String,
    /// Short detail or label description (`./utils/format`).
    pub detail: Option<String>,
    /// Character indices of the label that matched the typed prefix.
    pub matched: Vec<u32>,
    /// Deprecated symbol.
    pub deprecated: bool,
}

/// A ranked page of completion items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    /// Session id (filtering, details and accepting refer to it).
    pub session: u32,
    /// Start of the text the completion replaces (the typed prefix).
    pub from: u32,
    /// Items matching the prefix, best first (at most one page).
    pub items: Vec<CompletionItemView>,
    /// How many items matched in total.
    pub total: u32,
}

/// Details of one completion item, loaded lazily.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompletionDetails {
    /// Detail (signature, "Auto import from …").
    pub detail: Option<String>,
    /// Documentation as sanitized HTML.
    pub html: Option<String>,
}

/// Edits for one document version plus, for snippets, the tab stops in the
/// text after the edits (each stop may have several linked ranges; the
/// last one is the final caret position).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEdit {
    /// The version the edits apply to.
    pub version: u32,
    /// Sorted, non-overlapping edits.
    pub edits: Vec<TextEdit>,
    /// Snippet tab stops, in order; empty for plain text.
    pub stops: Vec<Vec<TextRange>>,
    /// Caret after the edit when there are no stops.
    pub caret: Option<u32>,
}

/// Signature help at the caret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignatureView {
    /// The signature, e.g. `formatPrice(value: number, currency?: string): string`.
    pub label: String,
    /// Each parameter's range in `label` (UTF-16).
    pub parameters: Vec<TextRange>,
    /// The parameter at the caret.
    pub active_parameter: Option<u32>,
    /// Which overload (0-based) of how many.
    pub active_signature: u32,
    /// Number of overloads.
    pub signatures: u32,
    /// Documentation of the signature or the active parameter (sanitized HTML).
    pub html: Option<String>,
}

/// One usage of a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based UTF-16 column.
    pub column: u32,
    /// Length of the usage on its line (UTF-16).
    pub length: u32,
    /// The line's text, trimmed.
    pub preview: String,
}

/// Usages in one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    /// Project-relative path, or absolute outside the project.
    pub path: String,
    /// Whether inside the project.
    pub in_project: bool,
    /// Usages in order.
    pub usages: Vec<Usage>,
}

/// Find Usages result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageResults {
    /// The symbol's name at the caret.
    pub symbol: String,
    /// Groups by file (project files first, sorted by path).
    pub groups: Vec<UsageGroup>,
}

/// What Rename would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameTarget {
    /// Range of the symbol at the caret.
    pub from: u32,
    /// End of the range.
    pub to: u32,
    /// The current name.
    pub placeholder: String,
}

/// One file in a refactoring preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFile {
    /// Project-relative path, or absolute outside the project.
    pub path: String,
    /// Edits in the file.
    pub count: u32,
    /// Library code: it will not be changed.
    pub read_only: bool,
}

/// A computed refactoring, applied with `apply_refactoring(id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefactoringPreview {
    /// Id for applying it.
    pub id: u32,
    /// Files and edit counts.
    pub files: Vec<PreviewFile>,
    /// Total edits.
    pub total: u32,
}

/// A file a workspace edit did not change, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    /// Path.
    pub path: String,
    /// Reason.
    pub reason: String,
}

/// Result of applying a workspace edit (rename, code action, formatting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEditResult {
    /// Edits for open documents: the view applies each as one undo step.
    pub open_documents: Vec<DocumentReplace>,
    /// Closed files rewritten on disk.
    pub files_written: Vec<String>,
    /// Files left unchanged.
    pub skipped: Vec<SkippedFile>,
}

/// A code action at the caret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionView {
    /// Id for applying it.
    pub id: u32,
    /// Title.
    pub title: String,
    /// Kind (`quickfix`, `refactor.extract`, `source.organizeImports`).
    pub kind: String,
    /// The server's preferred fix.
    pub preferred: bool,
    /// Why it cannot be applied, if disabled.
    pub disabled: Option<String>,
}

// Terminal (docs/ARCHITECTURE.md, 4.9)

/// Size of a terminal grid in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    /// Columns.
    pub cols: u32,
    /// Rows.
    pub rows: u32,
}

/// Bits of [`TerminalRun::flags`]. Exported to TypeScript as the constant
/// `TERMINAL_FLAGS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct TerminalFlagBits {
    pub bold: u32,
    pub dim: u32,
    pub italic: u32,
    pub underline: u32,
    pub strike: u32,
    pub inverse: u32,
    pub hidden: u32,
    pub selected: u32,
    pub wide: u32,
}

/// The values of the [`TerminalRun::flags`] bits.
pub const TERMINAL_FLAGS: TerminalFlagBits = TerminalFlagBits {
    bold: 1,
    dim: 1 << 1,
    italic: 1 << 2,
    underline: 1 << 3,
    strike: 1 << 4,
    inverse: 1 << 5,
    hidden: 1 << 6,
    selected: 1 << 7,
    wide: 1 << 8,
};

/// Consecutive cells of one style.
///
/// Colors are numbers: `0` is the theme's default color, `1..=16` is one of
/// the 16 theme colors (index plus one), and `0x0100_0000 | 0xRRGGBB` is a
/// 24-bit color (the backend resolves the other xterm palette colors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRun {
    /// Text, one character per cell (a wide character covers two cells).
    pub text: String,
    /// Foreground color.
    pub fg: u32,
    /// Background color.
    pub bg: u32,
    /// Attributes ([`TERMINAL_FLAGS`]).
    pub flags: u32,
}

/// One row of the viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRow {
    /// Row in the viewport, 0 at the top.
    pub row: u32,
    /// Runs from the first column.
    pub runs: Vec<TerminalRun>,
}

/// A cell of the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCell {
    /// Row in the viewport.
    pub row: u32,
    /// Column.
    pub col: u32,
}

/// A screen update (`terminal-frame-event`). The view draws it, then calls
/// `terminal_ack` with `seq`; the next frame waits for that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalFrame {
    /// Terminal id.
    pub id: u32,
    /// Sequence number.
    pub seq: u32,
    /// Grid size.
    pub size: TerminalSize,
    /// Whether `rows` holds every row (else only the changed ones).
    pub full: bool,
    /// Changed rows.
    pub rows: Vec<TerminalRow>,
    /// Cursor, when shown and inside the viewport.
    pub cursor: Option<TerminalCell>,
    /// Lines scrolled back from the bottom (0 follows the output).
    pub display_offset: u32,
    /// Lines of scrollback.
    pub history: u32,
    /// Tab title.
    pub title: String,
}

/// A terminal's shell exited (`terminal-exited-event`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExited {
    /// Terminal id.
    pub id: u32,
    /// Exit code, if known.
    pub code: Option<i32>,
}

/// An open terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    /// Id.
    pub id: u32,
    /// Tab title.
    pub title: String,
    /// Shell program.
    pub shell: String,
    /// Working directory it started in.
    pub cwd: String,
}

/// The project's terminals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalList {
    /// Why terminals cannot start in this build, if they cannot.
    pub unavailable: Option<String>,
    /// Open terminals, in opening order.
    pub terminals: Vec<TerminalInfo>,
}

/// A key the view sends as a key rather than text (arrows, function keys,
/// Ctrl/Alt combinations).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalKey {
    /// `KeyboardEvent.key`.
    pub key: String,
    /// Ctrl held.
    pub ctrl: bool,
    /// Alt (Option) held.
    pub alt: bool,
    /// Shift held.
    pub shift: bool,
}

/// A file link in terminal output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TerminalLink {
    /// Where it points.
    pub target: NavTarget,
    /// First column of the link on its row.
    pub start: u32,
    /// One past its last column.
    pub end: u32,
}

/// How a mouse selection grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TerminalSelectionKind {
    /// By character (drag).
    Simple,
    /// By word (double click).
    Word,
    /// By line (triple click).
    Line,
}

// Git (docs/ARCHITECTURE.md, 4.9)

/// How a file shows in the tree and the commit window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum GitFileKind {
    Conflicted,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
}

/// A changed, untracked or ignored file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// Project-relative path.
    pub path: String,
    /// Path before a rename.
    pub orig_path: Option<String>,
    /// Kind.
    pub kind: GitFileKind,
    /// Has changes staged in the index.
    pub staged: bool,
    /// Has changes not staged (untracked files count as unstaged).
    pub unstaged: bool,
}

/// The repository state (`git-status-event`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// Why Git features are off (no repository, git not installed), if so.
    pub unavailable: Option<String>,
    /// Current branch; `None` when detached or unavailable.
    pub branch: Option<String>,
    /// Abbreviated HEAD commit; `None` before the first commit.
    pub head: Option<String>,
    /// Upstream branch.
    pub upstream: Option<String>,
    /// Commits ahead of the upstream.
    pub ahead: u32,
    /// Commits behind it.
    pub behind: u32,
    /// Changed, untracked and ignored files inside the project.
    pub files: Vec<GitFile>,
    /// Folders containing changed or untracked files.
    pub changed_dirs: Vec<String>,
    /// Ignored folders (everything below is ignored).
    pub ignored_dirs: Vec<String>,
    /// Increases with every refresh.
    pub generation: u32,
}

/// A gutter hunk: lines of the document that differ from the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitHunk {
    /// Added, modified or deleted.
    pub kind: GitHunkKind,
    /// First document line (zero-based; for a deletion, the line after it).
    pub line: u32,
    /// Number of document lines (0 for a deletion).
    pub lines: u32,
    /// The index text of the lines it replaces.
    pub old_text: String,
}

/// Kind of a [`GitHunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum GitHunkKind {
    Added,
    Modified,
    Deleted,
}

/// Gutter hunks of a document version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitLineChanges {
    /// Document version they belong to.
    pub version: u32,
    /// Whether the file is tracked (untracked files have no markers).
    pub tracked: bool,
    /// Hunks in document order.
    pub hunks: Vec<GitHunk>,
}

/// Unified or side-by-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum GitDiffMode {
    Unified,
    SideBySide,
}

/// Kind of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum GitLineKind {
    Context,
    Added,
    Deleted,
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffLine {
    /// One-based line in the old version.
    pub old_line: Option<u32>,
    /// One-based line in the new version.
    pub new_line: Option<u32>,
    /// Kind.
    pub kind: GitLineKind,
    /// Text.
    pub text: String,
    /// Changed words: `[from, to]` UTF-16 pairs, flattened.
    pub words: Vec<u32>,
}

/// A row of the side-by-side view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitSplitRow {
    /// Old side.
    pub left: Option<GitDiffLine>,
    /// New side.
    pub right: Option<GitDiffLine>,
}

/// A file's changes against HEAD, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitDiff {
    /// Project-relative path.
    pub path: String,
    /// Label of the old side (`HEAD (abc123)`).
    pub old_label: String,
    /// Label of the new side.
    pub new_label: String,
    /// Rows for the unified mode (empty in side-by-side mode).
    pub unified: Vec<GitDiffLine>,
    /// Rows for the side-by-side mode (empty in unified mode).
    pub split: Vec<GitSplitRow>,
    /// Number of changed regions.
    pub hunks: u32,
}

/// Who last changed a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitBlameLine {
    /// Abbreviated commit (empty for uncommitted lines).
    pub commit: String,
    /// Author.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author time, seconds since the epoch.
    pub time: u32,
    /// Commit subject.
    pub summary: String,
    /// Whether the line is committed.
    pub committed: bool,
}

/// Blame of a document version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitBlame {
    /// Document version.
    pub version: u32,
    /// One entry per document line.
    pub lines: Vec<GitBlameLine>,
}

/// A commit in a history list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    /// Full id.
    pub id: String,
    /// Abbreviated id.
    pub short_id: String,
    /// Author.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author time, seconds since the epoch.
    pub time: u32,
    /// Subject.
    pub subject: String,
    /// Number of parents (2+ for merges).
    pub parents: u32,
}

/// A branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitBranch {
    /// Short name.
    pub name: String,
    /// Remote-tracking branch.
    pub remote: bool,
    /// Checked out.
    pub current: bool,
    /// Upstream of a local branch.
    pub upstream: Option<String>,
    /// Subject of its commit.
    pub subject: String,
}

/// Result of deleting a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum GitDeleteResult {
    /// Deleted.
    Deleted,
    /// Not deleted: not merged into the current branch (delete with force
    /// to drop its commits).
    NotMerged,
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
