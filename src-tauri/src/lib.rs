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

use ferrule_core::Session;
use ferrule_ipc::{
    AppInfo, CaretDirection, ChangeTransaction, DirEntry, DocId, DocStatus, EditorSelection,
    FrontendReadyReport, Highlights, IpcError, KeyBinding, OpenedDocument, PROTOCOL_VERSION,
    ProjectInfo, RemoteUpdate,
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

/// Folder passed on the command line, if any.
#[derive(Debug, Clone, Default)]
pub struct InitialProject(pub Option<String>);

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

/// Folder passed on the command line (`ferrule <path>`), if any.
#[tauri::command]
#[specta::specta]
async fn initial_project(initial: State<'_, InitialProject>) -> Result<Option<String>, IpcError> {
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
async fn save_document(session: SessionState<'_>, doc_id: DocId) -> Result<DocStatus, IpcError> {
    session.save_document(doc_id).await
}

/// Closes a document (unsaved text stays recoverable).
#[tauri::command]
#[specta::specta]
async fn close_document(session: SessionState<'_>, doc_id: DocId) -> Result<(), IpcError> {
    session.close_document(doc_id).await
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
            apply_changes,
            undo,
            redo,
            add_caret,
            select_next_occurrence,
            select_all_occurrences,
            highlights,
            save_document,
            close_document,
        ])
        .constant("PROTOCOL_VERSION", PROTOCOL_VERSION)
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
    let initial = InitialProject(std::env::args_os().nth(1).map(|arg| {
        let path = PathBuf::from(arg);
        std::path::absolute(&path)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }));

    configure(
        tauri::Builder::default(),
        initial,
        SmokeMode::new(smoke_enabled),
    )
    .plugin(tauri_plugin_dialog::init())
    .setup(move |app| {
        let recovery_dir = app.path().app_data_dir()?.join("recovery");
        tracing::info!(?recovery_dir, "recovery directory");
        app.manage(Session::new(recovery_dir)?);
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
    .run(tauri::generate_context!())?;
    Ok(())
}
