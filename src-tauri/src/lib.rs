//! Ferrule desktop shell.
//!
//! This crate only wires Tauri to `ferrule-core`: it declares the typed IPC
//! commands, sets up the window and exports the TypeScript bindings. It holds
//! no business logic (see `docs/ARCHITECTURE.md`, section 1). Commands are
//! generic over the Tauri runtime so the headless test backend runs exactly
//! the same handler (section 7).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

mod events;

pub use events::{
    FsChanged, GitStatusEvent, IndexStatus, ProblemsChanged, ServerStatusChanged, TauriSink,
    TerminalExitedEvent, TerminalFrameEvent, encode as encode_event,
};

use ferrule_core::{Session, SessionConfig};
use ferrule_ipc::{
    AppInfo, CaretDirection, CaretInfo, ChangeTransaction, CodeActionView, CompletionDetails,
    CompletionList, DirEntry, DocDiagnostics, DocId, DocStatus, DocumentEdit, DocumentReload,
    DocumentReplace, EditorSelection, FileProblems, FindResults, FoldRanges, FrontendReadyReport,
    GitBlame, GitBranch, GitCommit, GitDeleteResult, GitDiff, GitDiffMode, GitLineChanges,
    GitStatus, Highlights, HoverResult, IpcError, KeyBinding, NavKind, NavTarget, OpenedDocument,
    PROTOCOL_VERSION, ProjectInfo, RefactoringPreview, RemoteUpdate, RenameTarget, ReplaceResult,
    SaveResult, SearchResults, SearchTab, ServerStatus, SignatureView, StartupTarget,
    TERMINAL_FLAGS, TerminalCell, TerminalInfo, TerminalKey, TerminalLink, TerminalList,
    TerminalSelectionKind, TerminalSize, TextSearch, TraceSample, UsageResults,
    WorkspaceEditResult,
};
use specta_typescript::Typescript;
use tauri::{Manager, Runtime, State};
use tracing_subscriber::EnvFilter;

/// Environment variable that enables end-to-end smoke mode. When set to `1`,
/// the app exits with code 0 after the frontend reports a correct render,
/// with code 1 if the report is wrong, and with code 2 if no report arrives
/// within [`SMOKE_TIMEOUT`].
pub const SMOKE_ENV: &str = "FERRULE_E2E_SMOKE";

/// How long smoke mode waits for the frontend before giving up.
pub const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);

const SMOKE_TIMEOUT_EXIT_CODE: i32 = 2;

type ExitFn = Box<dyn Fn(i32) + Send + Sync>;

/// End-to-end smoke mode state. `exit` is installed by the real app during
/// setup (the headless backend never enables smoke mode).
pub struct SmokeMode {
    enabled: bool,
    exit: OnceLock<ExitFn>,
}

impl SmokeMode {
    /// Creates the state; `enabled` comes from [`SMOKE_ENV`].
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            exit: OnceLock::new(),
        }
    }

    fn finish(&self, code: i32) {
        if self.enabled {
            match self.exit.get() {
                Some(exit) => exit(code),
                None => tracing::error!("smoke mode: no exit handler installed"),
            }
        }
    }
}

/// Folder (and optional file) passed on the command line, if any.
#[derive(Debug, Clone, Default)]
pub struct InitialProject(pub Option<StartupTarget>);

type SessionState<'a> = State<'a, Arc<Session>>;

/// Returns static information about the running application.
#[tauri::command]
#[specta::specta]
async fn app_info() -> AppInfo {
    ferrule_core::app_info()
}

/// Called once by the frontend after it has mounted.
#[tauri::command]
#[specta::specta]
async fn report_frontend_ready(
    smoke: State<'_, SmokeMode>,
    report: FrontendReadyReport,
) -> Result<(), IpcError> {
    let result = ferrule_core::check_frontend_ready(&report);
    match &result {
        Ok(()) => tracing::info!(?report, "frontend ready"),
        Err(err) => tracing::error!(%err, ?report, "frontend reported an unexpected state"),
    }
    smoke.finish(if result.is_ok() { 0 } else { 1 });
    result
}

/// Folder and file passed on the command line (`ferrule <folder> [file]`).
#[tauri::command]
#[specta::specta]
async fn initial_project(
    initial: State<'_, InitialProject>,
) -> Result<Option<StartupTarget>, IpcError> {
    Ok(initial.0.clone())
}

/// IntelliJ-style key bindings for this OS.
#[tauri::command]
#[specta::specta]
async fn keymap() -> Vec<KeyBinding> {
    ferrule_core::keymap()
}

/// Opens a folder as the project.
#[tauri::command]
#[specta::specta]
async fn open_project(session: SessionState<'_>, path: String) -> Result<ProjectInfo, IpcError> {
    session.open_project(path).await
}

/// Lists a project directory (`""` is the root).
#[tauri::command]
#[specta::specta]
async fn list_dir(session: SessionState<'_>, path: String) -> Result<Vec<DirEntry>, IpcError> {
    session.list_dir(path).await
}

/// Opens a document by project-relative path.
#[tauri::command]
#[specta::specta]
async fn open_document(
    session: SessionState<'_>,
    path: String,
) -> Result<OpenedDocument, IpcError> {
    session.open_document(path).await
}

/// Opens a file outside the project read-only, by absolute path.
#[tauri::command]
#[specta::specta]
async fn open_external(
    session: SessionState<'_>,
    path: String,
) -> Result<OpenedDocument, IpcError> {
    session.open_external(path).await
}

/// Applies view transactions.
#[tauri::command]
#[specta::specta]
async fn apply_changes(
    session: SessionState<'_>,
    doc_id: DocId,
    base_version: u32,
    transactions: Vec<ChangeTransaction>,
) -> Result<DocStatus, IpcError> {
    session
        .apply_changes(doc_id, base_version, transactions)
        .await
}

/// Undoes the last edit group.
#[tauri::command]
#[specta::specta]
async fn undo(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<Option<RemoteUpdate>, IpcError> {
    session.undo(doc_id, version).await
}

/// Redoes the last undone group.
#[tauri::command]
#[specta::specta]
async fn redo(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<Option<RemoteUpdate>, IpcError> {
    session.redo(doc_id, version).await
}

/// Clones the caret one line up or down.
#[tauri::command]
#[specta::specta]
async fn add_caret(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    selection: EditorSelection,
    direction: CaretDirection,
) -> Result<EditorSelection, IpcError> {
    session
        .add_caret(doc_id, version, selection, direction)
        .await
}

/// Adds the next occurrence of the selection.
#[tauri::command]
#[specta::specta]
async fn select_next_occurrence(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    selection: EditorSelection,
) -> Result<EditorSelection, IpcError> {
    session
        .select_next_occurrence(doc_id, version, selection)
        .await
}

/// Selects all occurrences of the selection.
#[tauri::command]
#[specta::specta]
async fn select_all_occurrences(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    selection: EditorSelection,
) -> Result<EditorSelection, IpcError> {
    session
        .select_all_occurrences(doc_id, version, selection)
        .await
}

/// Highlight spans for a UTF-16 range.
#[tauri::command]
#[specta::specta]
async fn highlights(
    session: SessionState<'_>,
    doc_id: DocId,
    from: u32,
    to: u32,
) -> Result<Highlights, IpcError> {
    session.highlights(doc_id, from, to).await
}

/// Saves a document.
#[tauri::command]
#[specta::specta]
async fn save_document(session: SessionState<'_>, doc_id: DocId) -> Result<SaveResult, IpcError> {
    session.save_document(doc_id).await
}

/// Closes a document (unsaved text stays recoverable).
#[tauri::command]
#[specta::specta]
async fn close_document(session: SessionState<'_>, doc_id: DocId) -> Result<(), IpcError> {
    session.close_document(doc_id).await
}

/// Enter: newline with language-aware indentation, computed in Rust.
#[tauri::command]
#[specta::specta]
async fn new_line(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    selection: EditorSelection,
) -> Result<RemoteUpdate, IpcError> {
    session.new_line(doc_id, version, selection).await
}

/// Reloads a document from disk (one undoable step).
#[tauri::command]
#[specta::specta]
async fn reload_document(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<DocumentReload, IpcError> {
    session.reload_document(doc_id, version).await
}

/// Whether the file on disk differs from the last loaded/saved text.
#[tauri::command]
#[specta::specta]
async fn disk_changed(session: SessionState<'_>, doc_id: DocId) -> Result<bool, IpcError> {
    session.disk_changed(doc_id).await
}

/// Breadcrumbs and the bracket pair at a UTF-16 offset.
#[tauri::command]
#[specta::specta]
async fn caret_info(
    session: SessionState<'_>,
    doc_id: DocId,
    offset: u32,
) -> Result<CaretInfo, IpcError> {
    session.caret_info(doc_id, offset).await
}

/// Folding ranges of a document.
#[tauri::command]
#[specta::specta]
async fn folds(session: SessionState<'_>, doc_id: DocId) -> Result<FoldRanges, IpcError> {
    session.folds(doc_id).await
}

/// Search Everywhere.
#[tauri::command]
#[specta::specta]
async fn search_everywhere(
    session: SessionState<'_>,
    query: String,
    tab: SearchTab,
) -> Result<SearchResults, IpcError> {
    session.search_everywhere(query, tab).await
}

/// Recently opened files, most recent first.
#[tauri::command]
#[specta::specta]
async fn recent_files(session: SessionState<'_>) -> Result<Vec<String>, IpcError> {
    session.recent_files().await
}

/// Find in Files (with optional replacement preview).
#[tauri::command]
#[specta::specta]
async fn find_in_files(
    session: SessionState<'_>,
    search: TextSearch,
) -> Result<FindResults, IpcError> {
    session.find_in_files(search).await
}

/// Replace in Files.
#[tauri::command]
#[specta::specta]
async fn replace_in_files(
    session: SessionState<'_>,
    search: TextSearch,
    replacement: String,
) -> Result<ReplaceResult, IpcError> {
    session.replace_in_files(search, replacement).await
}

/// Hover / Quick Documentation at a UTF-16 offset.
#[tauri::command]
#[specta::specta]
async fn hover(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
) -> Result<Option<HoverResult>, IpcError> {
    session.hover(doc_id, version, offset).await
}

/// Go to Declaration / Type Declaration / Implementation.
#[tauri::command]
#[specta::specta]
async fn goto_definition(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
    kind: NavKind,
) -> Result<Vec<NavTarget>, IpcError> {
    session.goto_definition(doc_id, version, offset, kind).await
}

/// Completions at the caret.
#[tauri::command]
#[specta::specta]
async fn completion(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
    trigger: Option<String>,
) -> Result<Option<CompletionList>, IpcError> {
    session.completion(doc_id, version, offset, trigger).await
}

/// Filters the current completion session without another server request.
#[tauri::command]
#[specta::specta]
async fn filter_completion(
    session: SessionState<'_>,
    completion_session: u32,
    prefix: String,
) -> Result<Option<CompletionList>, IpcError> {
    session.filter_completion(completion_session, prefix).await
}

/// Loads completion documentation on demand.
#[tauri::command]
#[specta::specta]
async fn completion_details(
    session: SessionState<'_>,
    completion_session: u32,
    index: u32,
) -> Result<CompletionDetails, IpcError> {
    session.completion_details(completion_session, index).await
}

/// Prepares the edit for the selected completion.
#[tauri::command]
#[specta::specta]
async fn accept_completion(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    completion_session: u32,
    index: u32,
    caret: u32,
) -> Result<DocumentEdit, IpcError> {
    session
        .accept_completion(doc_id, version, completion_session, index, caret)
        .await
}

/// Signature help at the caret.
#[tauri::command]
#[specta::specta]
async fn signature_help(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
    trigger: Option<String>,
) -> Result<Option<SignatureView>, IpcError> {
    session
        .signature_help(doc_id, version, offset, trigger)
        .await
}

/// Finds references to the symbol at the caret.
#[tauri::command]
#[specta::specta]
async fn find_usages(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
) -> Result<UsageResults, IpcError> {
    session.find_usages(doc_id, version, offset).await
}

/// Finds the renameable range under the caret.
#[tauri::command]
#[specta::specta]
async fn prepare_rename(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
) -> Result<Option<RenameTarget>, IpcError> {
    session.prepare_rename(doc_id, version, offset).await
}

/// Previews a rename without changing files.
#[tauri::command]
#[specta::specta]
async fn rename_preview(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    offset: u32,
    new_name: String,
) -> Result<RefactoringPreview, IpcError> {
    session
        .rename_preview(doc_id, version, offset, new_name)
        .await
}

/// Applies a previously previewed rename or refactoring.
#[tauri::command]
#[specta::specta]
async fn apply_refactoring(
    session: SessionState<'_>,
    id: u32,
) -> Result<WorkspaceEditResult, IpcError> {
    session.apply_refactoring(id).await
}

/// Lists quick fixes and refactorings for a range.
#[tauri::command]
#[specta::specta]
async fn code_actions(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    from: u32,
    to: u32,
) -> Result<Vec<CodeActionView>, IpcError> {
    session.code_actions(doc_id, version, from, to).await
}

/// Applies one quick fix or refactoring.
#[tauri::command]
#[specta::specta]
async fn apply_code_action(
    session: SessionState<'_>,
    id: u32,
) -> Result<WorkspaceEditResult, IpcError> {
    session.apply_code_action(id).await
}

/// Organizes imports through the TypeScript language server.
#[tauri::command]
#[specta::specta]
async fn organize_imports(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<WorkspaceEditResult, IpcError> {
    session.organize_imports(doc_id, version).await
}

/// Sorts import declarations in the open document.
#[tauri::command]
#[specta::specta]
async fn sort_imports(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<WorkspaceEditResult, IpcError> {
    session.sort_imports(doc_id, version).await
}

/// Formats the open document through its language server.
#[tauri::command]
#[specta::specta]
async fn format_document(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
) -> Result<WorkspaceEditResult, IpcError> {
    session.format_document(doc_id, version).await
}

/// Diagnostics of an open document (UTF-16 offsets of its current text).
#[tauri::command]
#[specta::specta]
async fn doc_diagnostics(
    session: SessionState<'_>,
    doc_id: DocId,
) -> Result<DocDiagnostics, IpcError> {
    session.doc_diagnostics(doc_id).await
}

/// Problems of every file.
#[tauri::command]
#[specta::specta]
async fn problems(session: SessionState<'_>) -> Result<Vec<FileProblems>, IpcError> {
    session.problems().await
}

/// Status of the project's language servers.
#[tauri::command]
#[specta::specta]
async fn language_servers(session: SessionState<'_>) -> Result<Vec<ServerStatus>, IpcError> {
    session.language_servers().await
}

/// Restarts the TypeScript language server.
#[tauri::command]
#[specta::specta]
async fn restart_language_server(session: SessionState<'_>) -> Result<(), IpcError> {
    session.restart_language_server().await
}

/// Whether typing-path timing is on (`FERRULE_TYPING_TRACE`).
#[tauri::command]
#[specta::specta]
async fn typing_trace_enabled(session: SessionState<'_>) -> Result<bool, IpcError> {
    Ok(session.typing_trace_enabled())
}

/// Appends typing-path timings from the view to the trace file.
#[tauri::command]
#[specta::specta]
async fn record_typing_trace(
    session: SessionState<'_>,
    samples: Vec<TraceSample>,
) -> Result<(), IpcError> {
    session.record_typing_trace(samples).await
}

/// The project's terminals, and why none can start if so.
#[tauri::command]
#[specta::specta]
async fn terminals(session: SessionState<'_>) -> Result<TerminalList, IpcError> {
    session.terminals().await
}

/// Starts a terminal in the project root.
#[tauri::command]
#[specta::specta]
async fn open_terminal(
    session: SessionState<'_>,
    size: TerminalSize,
) -> Result<TerminalInfo, IpcError> {
    session.open_terminal(size).await
}

/// Sends typed text to a terminal.
#[tauri::command]
#[specta::specta]
async fn terminal_input(session: SessionState<'_>, id: u32, text: String) -> Result<(), IpcError> {
    session.terminal_input(id, text).await
}

/// Sends a key (arrows, function keys, Ctrl/Alt combinations).
#[tauri::command]
#[specta::specta]
async fn terminal_key(
    session: SessionState<'_>,
    id: u32,
    key: TerminalKey,
) -> Result<(), IpcError> {
    session.terminal_key(id, key).await
}

/// Pastes text into a terminal.
#[tauri::command]
#[specta::specta]
async fn terminal_paste(session: SessionState<'_>, id: u32, text: String) -> Result<(), IpcError> {
    session.terminal_paste(id, text).await
}

/// Resizes a terminal.
#[tauri::command]
#[specta::specta]
async fn terminal_resize(
    session: SessionState<'_>,
    id: u32,
    size: TerminalSize,
) -> Result<(), IpcError> {
    session.terminal_resize(id, size).await
}

/// Scrolls a terminal (positive: back into the scrollback).
#[tauri::command]
#[specta::specta]
async fn terminal_scroll(session: SessionState<'_>, id: u32, lines: i32) -> Result<(), IpcError> {
    session.terminal_scroll(id, lines).await
}

/// Selects terminal text from `anchor` to `head`.
#[tauri::command]
#[specta::specta]
async fn terminal_select(
    session: SessionState<'_>,
    id: u32,
    anchor: TerminalCell,
    head: TerminalCell,
    kind: TerminalSelectionKind,
) -> Result<(), IpcError> {
    session.terminal_select(id, anchor, head, kind).await
}

/// Clears a terminal's selection.
#[tauri::command]
#[specta::specta]
async fn terminal_clear_selection(session: SessionState<'_>, id: u32) -> Result<(), IpcError> {
    session.terminal_clear_selection(id).await
}

/// A terminal's selected text.
#[tauri::command]
#[specta::specta]
async fn terminal_selection_text(
    session: SessionState<'_>,
    id: u32,
) -> Result<Option<String>, IpcError> {
    session.terminal_selection_text(id).await
}

/// The file link under a terminal cell, if any.
#[tauri::command]
#[specta::specta]
async fn terminal_link_at(
    session: SessionState<'_>,
    id: u32,
    cell: TerminalCell,
) -> Result<Option<TerminalLink>, IpcError> {
    session.terminal_link_at(id, cell).await
}

/// Renames a terminal tab.
#[tauri::command]
#[specta::specta]
async fn rename_terminal(
    session: SessionState<'_>,
    id: u32,
    title: String,
) -> Result<TerminalInfo, IpcError> {
    session.rename_terminal(id, title).await
}

/// Closes a terminal and ends its processes.
#[tauri::command]
#[specta::specta]
async fn close_terminal(session: SessionState<'_>, id: u32) -> Result<(), IpcError> {
    session.close_terminal(id).await
}

/// The view drew a terminal frame.
#[tauri::command]
#[specta::specta]
async fn terminal_ack(session: SessionState<'_>, id: u32, seq: u32) -> Result<(), IpcError> {
    session.terminal_ack(id, seq).await
}

/// The latest Git status of the project.
#[tauri::command]
#[specta::specta]
async fn git_status(session: SessionState<'_>) -> Result<GitStatus, IpcError> {
    session.git_status().await
}

/// Asks for a Git status refresh (the result arrives as an event).
#[tauri::command]
#[specta::specta]
async fn git_refresh(session: SessionState<'_>) -> Result<(), IpcError> {
    session.git_refresh().await
}

/// Lines of a document that differ from the Git index.
#[tauri::command]
#[specta::specta]
async fn git_line_changes(
    session: SessionState<'_>,
    doc_id: DocId,
) -> Result<GitLineChanges, IpcError> {
    session.git_line_changes(doc_id).await
}

/// The edit that rolls a changed region back to the index text.
#[tauri::command]
#[specta::specta]
async fn git_rollback_hunk(
    session: SessionState<'_>,
    doc_id: DocId,
    version: u32,
    line: u32,
    lines: u32,
) -> Result<DocumentReplace, IpcError> {
    session
        .git_rollback_hunk(doc_id, version, line, lines)
        .await
}

/// A project file against HEAD.
#[tauri::command]
#[specta::specta]
async fn git_diff(
    session: SessionState<'_>,
    path: String,
    mode: GitDiffMode,
) -> Result<GitDiff, IpcError> {
    session.git_diff(path, mode).await
}

/// Who last changed each line of a document.
#[tauri::command]
#[specta::specta]
async fn git_blame(session: SessionState<'_>, doc_id: DocId) -> Result<GitBlame, IpcError> {
    session.git_blame(doc_id).await
}

/// Commits of the current branch, or of one file.
#[tauri::command]
#[specta::specta]
async fn git_log(
    session: SessionState<'_>,
    path: Option<String>,
) -> Result<Vec<GitCommit>, IpcError> {
    session.git_log(path).await
}

/// Local and remote branches.
#[tauri::command]
#[specta::specta]
async fn git_branches(session: SessionState<'_>) -> Result<Vec<GitBranch>, IpcError> {
    session.git_branches().await
}

/// Checks out a branch.
#[tauri::command]
#[specta::specta]
async fn git_checkout(
    session: SessionState<'_>,
    name: String,
    remote: bool,
) -> Result<(), IpcError> {
    session.git_checkout(name, remote).await
}

/// Creates a branch.
#[tauri::command]
#[specta::specta]
async fn git_create_branch(
    session: SessionState<'_>,
    name: String,
    checkout: bool,
) -> Result<(), IpcError> {
    session.git_create_branch(name, checkout).await
}

/// Deletes a local branch.
#[tauri::command]
#[specta::specta]
async fn git_delete_branch(
    session: SessionState<'_>,
    name: String,
    force: bool,
) -> Result<GitDeleteResult, IpcError> {
    session.git_delete_branch(name, force).await
}

/// Stages files.
#[tauri::command]
#[specta::specta]
async fn git_stage(session: SessionState<'_>, paths: Vec<String>) -> Result<(), IpcError> {
    session.git_stage(paths).await
}

/// Unstages files.
#[tauri::command]
#[specta::specta]
async fn git_unstage(session: SessionState<'_>, paths: Vec<String>) -> Result<(), IpcError> {
    session.git_unstage(paths).await
}

/// Saves, stages the checked files and commits.
#[tauri::command]
#[specta::specta]
async fn git_commit(
    session: SessionState<'_>,
    message: String,
    amend: bool,
    paths: Vec<String>,
) -> Result<String, IpcError> {
    session.git_commit(message, amend, paths).await
}

/// The last commit's message (for Amend).
#[tauri::command]
#[specta::specta]
async fn git_last_commit_message(session: SessionState<'_>) -> Result<Option<String>, IpcError> {
    session.git_last_commit_message().await
}

/// Builds the tauri-specta registry of every IPC command and constant.
///
/// The same builder serves the runtime invoke handler and the TypeScript
/// export, so the bindings cannot drift from what the app actually handles.
pub fn specta_builder<R: Runtime>() -> tauri_specta::Builder<R> {
    tauri_specta::Builder::<R>::new()
        .commands(tauri_specta::collect_commands![
            app_info,
            report_frontend_ready,
            initial_project,
            keymap,
            open_project,
            list_dir,
            open_document,
            open_external,
            apply_changes,
            undo,
            redo,
            add_caret,
            select_next_occurrence,
            select_all_occurrences,
            highlights,
            save_document,
            close_document,
            new_line,
            reload_document,
            disk_changed,
            caret_info,
            folds,
            search_everywhere,
            recent_files,
            find_in_files,
            replace_in_files,
            hover,
            goto_definition,
            completion,
            filter_completion,
            completion_details,
            accept_completion,
            signature_help,
            find_usages,
            prepare_rename,
            rename_preview,
            apply_refactoring,
            code_actions,
            apply_code_action,
            organize_imports,
            sort_imports,
            format_document,
            doc_diagnostics,
            problems,
            language_servers,
            restart_language_server,
            typing_trace_enabled,
            record_typing_trace,
            terminals,
            open_terminal,
            terminal_input,
            terminal_key,
            terminal_paste,
            terminal_resize,
            terminal_scroll,
            terminal_select,
            terminal_clear_selection,
            terminal_selection_text,
            terminal_link_at,
            rename_terminal,
            close_terminal,
            terminal_ack,
            git_status,
            git_refresh,
            git_line_changes,
            git_rollback_hunk,
            git_diff,
            git_blame,
            git_log,
            git_branches,
            git_checkout,
            git_create_branch,
            git_delete_branch,
            git_stage,
            git_unstage,
            git_commit,
            git_last_commit_message,
        ])
        .events(tauri_specta::collect_events![
            FsChanged,
            IndexStatus,
            ServerStatusChanged,
            ProblemsChanged,
            TerminalFrameEvent,
            TerminalExitedEvent,
            GitStatusEvent
        ])
        .constant("PROTOCOL_VERSION", PROTOCOL_VERSION)
        .constant("TERMINAL_FLAGS", TERMINAL_FLAGS)
}

/// Writes the generated TypeScript bindings to `path`.
pub fn export_bindings(path: &Path) -> anyhow::Result<()> {
    let header = "// Generated by `bun run gen:ipc` from the Rust IPC definitions. Do not edit.\n\
                  // This file is not committed; see docs/ARCHITECTURE.md, section 3.2.";
    specta_builder::<tauri::Wry>()
        .export(Typescript::default().header(header), path)
        .map_err(|err| anyhow::anyhow!("exporting TypeScript bindings failed: {err}"))
}

/// Registers shared state and the invoke handler. Used by the real app and
/// the headless test backend; each manages its own `Arc<Session>`.
/// (No typed events exist yet, so `mount_events` is not needed.)
pub fn configure<R: Runtime>(
    builder: tauri::Builder<R>,
    initial: InitialProject,
    smoke: SmokeMode,
) -> tauri::Builder<R> {
    builder
        .manage(initial)
        .manage(smoke)
        .invoke_handler(specta_builder::<R>().invoke_handler())
}

/// The PTY and terminal emulator built into this binary, if any.
pub fn terminal_backend() -> Option<Arc<dyn ferrule_terminal::Backend>> {
    #[cfg(feature = "native")]
    {
        Some(Arc::new(ferrule_terminal::native::NativeBackend))
    }
    #[cfg(not(feature = "native"))]
    {
        None
    }
}

/// Initializes logging (`RUST_LOG`, default `info`).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // `try_init` only fails if a global subscriber is already installed,
    // in which case the existing one is kept.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Starts the desktop application and blocks until it exits.
pub fn run() -> anyhow::Result<()> {
    init_tracing();
    let smoke_enabled = std::env::var(SMOKE_ENV).is_ok_and(|value| value == "1");
    let mut args = std::env::args_os().skip(1);
    let initial = InitialProject(args.next().map(|folder| {
        let path = PathBuf::from(folder);
        StartupTarget {
            folder: std::path::absolute(&path)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
            file: args.next().map(|f| f.to_string_lossy().replace('\\', "/")),
        }
    }));

    configure(
        tauri::Builder::default(),
        initial,
        SmokeMode::new(smoke_enabled),
    )
    .plugin(tauri_plugin_dialog::init())
    .setup(move |app| {
        specta_builder::<tauri::Wry>().mount_events(app);
        let data_dir = app.path().app_data_dir()?;
        let settings_file = app.path().app_config_dir()?.join("settings.json");
        tracing::info!(?data_dir, ?settings_file, "data directory and settings");
        let sink = Arc::new(TauriSink(app.handle().clone()));
        let mut config = SessionConfig::new(data_dir);
        config.settings_file = settings_file;
        // Typing-path timing for ferrule-perf (docs/ARCHITECTURE.md, 4.8).
        config.typing_trace = std::env::var_os("FERRULE_TYPING_TRACE").map(PathBuf::from);
        config.terminal_backend = terminal_backend();
        app.manage(Session::with_config(config, sink)?);
        if smoke_enabled {
            tracing::info!("smoke mode enabled");
            let handle = app.handle().clone();
            let exit_handle = handle.clone();
            // `set` only fails if already set, which cannot happen here.
            let _ = app
                .state::<SmokeMode>()
                .exit
                .set(Box::new(move |code| exit_handle.exit(code)));
            std::thread::spawn(move || {
                std::thread::sleep(SMOKE_TIMEOUT);
                tracing::error!("smoke mode: frontend did not report within {SMOKE_TIMEOUT:?}");
                handle.exit(SMOKE_TIMEOUT_EXIT_CODE);
            });
        }
        Ok(())
    })
    .build(tauri::generate_context!())?
    .run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            // Shells and their children end with the app.
            if let Some(session) = app.try_state::<Arc<Session>>() {
                session.shutdown_terminals();
            }
        }
    });
    Ok(())
}
