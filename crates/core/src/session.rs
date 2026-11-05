//! The editing session: the open project and its documents.
//!
//! Concurrency (ARCHITECTURE.md, section 6): the session map and each
//! document sit behind their own `std::sync::Mutex`. Locks are held only for
//! short synchronous sections and never across `.await`; all file IO,
//! parsing and editing runs inside `spawn_blocking`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use std::sync::atomic::AtomicU64;

use ferrule_buffer::{
    Document, Edit, EditKind, IndentRules, LineEnding, Rope, Selection, Transaction,
    detect_indent_unit,
};
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_lsp_client::lsp_types::Uri;
use ferrule_project::{EntryKind, Project};
use ferrule_settings::{EditorConfig, EndOfLine, resolve_editorconfig};
use ferrule_syntax::{DEFAULT_AUTO_CLOSE, LanguageRegistry, SyntaxError, SyntaxTree};
use ferrule_vfs::RecoveryStore;
use tokio_util::sync::CancellationToken;

use crate::events::{EventSink, SessionEvent};
use crate::lsp::{LanguageService, SyncKey};
use crate::project_runtime::{ProjectRuntime, index_project};
use crate::trace::TypingTrace;

use crate::convert::{
    buffer_error, edit_from_ipc, edit_to_ipc, io_error, kind_from_ipc, project_error,
    selection_from_ipc, selection_to_ipc, to_u32, to_usize, token_to_ipc, vfs_error,
};

/// Default delay between the last edit and writing the recovery file.
pub const DEFAULT_AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

/// Errors creating a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// A built-in language failed to load.
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
}

pub(crate) struct DocState {
    pub doc: Document,
    syntax: Option<SyntaxTree>,
    line_ending: LineEnding,
    bom: bool,
    /// Version and text at the last load/save/reload. "Modified" compares
    /// text, so undoing back to the saved text clears it.
    saved_version: u32,
    saved_text: Rope,
    restored: bool,
    indent_unit: String,
    indent_rules: IndentRules,
    tab_width: u32,
    auto_close: Vec<String>,
    editorconfig: EditorConfig,
    /// The language server run that has this document open, if any.
    pub lsp_key: Option<SyncKey>,
}

impl DocState {
    pub(crate) fn indent_unit(&self) -> &str {
        &self.indent_unit
    }

    pub(crate) fn tab_width(&self) -> u32 {
        self.tab_width
    }

    pub fn dirty(&self) -> bool {
        // Comparing ropes is O(1) when the lengths differ (the usual case
        // while typing) and only walks the text when they are equal.
        self.doc.version() != self.saved_version && *self.doc.rope() != self.saved_text
    }

    fn mark_saved(&mut self) {
        self.saved_version = self.doc.version();
        self.saved_text = self.doc.rope().clone();
        self.restored = false;
    }

    fn status(&self) -> ipc::DocStatus {
        ipc::DocStatus {
            version: self.doc.version(),
            dirty: self.dirty(),
        }
    }
}

pub(crate) struct DocHandle {
    pub id: DocId,
    pub path: PathBuf,
    pub rel_path: String,
    /// `file:` URI for the language server.
    pub uri: Option<Uri>,
    /// LSP language id if the project's language server handles the file.
    pub lsp_language: Option<&'static str>,
    /// Library code (outside the project or in `node_modules`): no edits.
    pub read_only: bool,
    language: Option<String>,
    state: Mutex<DocState>,
    autosave: Mutex<Option<CancellationToken>>,
}

impl DocHandle {
    pub fn lock(&self) -> Result<MutexGuard<'_, DocState>, IpcError> {
        lock(&self.state)
    }

    fn opened(&self) -> Result<ipc::OpenedDocument, IpcError> {
        let state = self.lock()?;
        Ok(ipc::OpenedDocument {
            doc_id: self.id,
            path: self.rel_path.clone(),
            text: state.doc.text(),
            version: state.doc.version(),
            language: self.language.clone(),
            dirty: state.dirty(),
            restored: state.restored,
            indent_unit: state.indent_unit.clone(),
            tab_width: state.tab_width,
            auto_close: state.auto_close.clone(),
            read_only: self.read_only,
        })
    }

    /// Writes the recovery file if dirty, deletes it otherwise.
    fn sync_recovery(&self, recovery: &RecoveryStore) {
        let result = match self.lock() {
            Ok(state) if state.dirty() => recovery.save(&self.path, &state.doc.text()),
            Ok(_) => recovery.remove(&self.path),
            Err(err) => {
                tracing::error!(%err, "document lock poisoned; recovery not written");
                return;
            }
        };
        if let Err(err) = result {
            tracing::error!(path = ?self.path, %err, "writing recovery file failed");
        }
    }

    fn cancel_autosave(&self) {
        if let Ok(mut slot) = lock(&self.autosave)
            && let Some(token) = slot.take()
        {
            token.cancel();
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    project: Option<Arc<ProjectRuntime>>,
    pub docs: HashMap<DocId, Arc<DocHandle>>,
    by_path: HashMap<PathBuf, DocId>,
    next_id: u32,
}

/// Where a session keeps its files, and its platform services.
#[derive(Clone)]
pub struct SessionConfig {
    /// Recovery files, index caches and project data.
    pub data_dir: PathBuf,
    /// The user settings file (`settings.json`).
    pub settings_file: PathBuf,
    /// Delay between the last edit and writing the recovery file.
    pub autosave_delay: Duration,
    /// Typing-path timing file (`FERRULE_TYPING_TRACE`), off when `None`.
    pub typing_trace: Option<PathBuf>,
    /// PTY and terminal emulator; `None` when the build has none (terminals
    /// then report why they cannot start).
    pub terminal_backend: Option<Arc<dyn ferrule_terminal::Backend>>,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("data_dir", &self.data_dir)
            .field("settings_file", &self.settings_file)
            .field("autosave_delay", &self.autosave_delay)
            .field("typing_trace", &self.typing_trace)
            .field("terminal_backend", &self.terminal_backend.is_some())
            .finish()
    }
}

impl SessionConfig {
    /// Defaults: `settings.json` inside `data_dir`, the default autosave
    /// delay.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            settings_file: data_dir.join("settings.json"),
            data_dir,
            autosave_delay: DEFAULT_AUTOSAVE_DELAY,
            typing_trace: None,
            terminal_backend: None,
        }
    }
}

/// The editing session. Cheap to share as `Arc<Session>`.
pub struct Session {
    pub(crate) state: Mutex<State>,
    self_weak: Weak<Session>,
    recovery: RecoveryStore,
    pub(crate) registry: Arc<LanguageRegistry>,
    autosave_delay: Duration,
    started: Instant,
    data_dir: PathBuf,
    pub(crate) settings_file: PathBuf,
    terminal_backend: Option<Arc<dyn ferrule_terminal::Backend>>,
    pub(crate) trace: Option<Arc<TypingTrace>>,
    pub(crate) sink: Arc<dyn EventSink>,
    /// Incremented by every Find in Files request; older requests stop.
    pub(crate) find_counter: Arc<AtomicU64>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, IpcError> {
    mutex
        .lock()
        .map_err(|_| IpcError::new(IpcErrorKind::Internal, "internal state lock poisoned"))
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tokio::task::spawn_blocking(f).await.map_err(|err| {
        IpcError::new(
            IpcErrorKind::Internal,
            format!("background task failed: {err}"),
        )
    })?
}

fn languages() -> Result<LanguageRegistry, SyntaxError> {
    let mut registry = LanguageRegistry::default();
    for def in ferrule_lang_js::languages()
        .into_iter()
        .chain(ferrule_lang_css::languages())
        .chain(ferrule_lang_html::languages())
        .chain(ferrule_lang_yaml::languages())
        .chain(ferrule_lang_markdown::languages())
    {
        registry.register(def)?;
    }
    Ok(registry)
}

impl Session {
    /// Creates a session storing recovery files, index caches and project
    /// data under `data_dir`, and pushing events to `sink`.
    pub fn new(
        data_dir: impl Into<PathBuf>,
        sink: Arc<dyn EventSink>,
    ) -> Result<Arc<Self>, SessionError> {
        Self::with_autosave_delay(data_dir, sink, DEFAULT_AUTOSAVE_DELAY)
    }

    /// Like [`Session::new`] with a custom recovery-file delay (tests).
    pub fn with_autosave_delay(
        data_dir: impl Into<PathBuf>,
        sink: Arc<dyn EventSink>,
        autosave_delay: Duration,
    ) -> Result<Arc<Self>, SessionError> {
        let mut config = SessionConfig::new(data_dir);
        config.autosave_delay = autosave_delay;
        Self::with_config(config, sink)
    }

    /// A session with explicit locations (the app passes its config
    /// directory for `settings.json`).
    pub fn with_config(
        config: SessionConfig,
        sink: Arc<dyn EventSink>,
    ) -> Result<Arc<Self>, SessionError> {
        let SessionConfig {
            data_dir,
            settings_file,
            autosave_delay,
            typing_trace,
            terminal_backend,
        } = config;
        let registry = Arc::new(languages()?);
        Ok(Arc::new_cyclic(move |weak| Self {
            state: Mutex::new(State::default()),
            self_weak: weak.clone(),
            recovery: RecoveryStore::new(data_dir.join("recovery")),
            registry,
            autosave_delay,
            started: Instant::now(),
            data_dir,
            settings_file,
            terminal_backend,
            trace: typing_trace.map(|path| Arc::new(TypingTrace::new(path))),
            sink,
            find_counter: Arc::new(AtomicU64::new(0)),
        }))
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub(crate) fn runtime(&self) -> Result<Arc<ProjectRuntime>, IpcError> {
        lock(&self.state)?
            .project
            .clone()
            .ok_or_else(|| IpcError::new(IpcErrorKind::Conflict, "no project is open"))
    }

    fn project(&self) -> Result<Arc<Project>, IpcError> {
        Ok(self.runtime()?.project.clone())
    }

    /// Every open document.
    pub(crate) fn open_docs(&self) -> Vec<Arc<DocHandle>> {
        lock(&self.state)
            .map(|s| s.docs.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn open_doc_at(&self, path: &Path) -> Result<Option<Arc<DocHandle>>, IpcError> {
        self.doc_by_path(path)
    }

    pub(crate) fn relative_path(&self, path: &Path) -> Option<String> {
        self.runtime().ok()?.project.relativize(path)
    }

    pub(crate) fn self_arc(&self) -> Result<Arc<Self>, IpcError> {
        self.self_weak
            .upgrade()
            .ok_or_else(|| IpcError::new(IpcErrorKind::Conflict, "the editing session has ended"))
    }

    /// Whether `service` belongs to the current project.
    pub(crate) fn is_current_service(&self, service: &Arc<LanguageService>) -> bool {
        self.runtime()
            .is_ok_and(|runtime| Arc::ptr_eq(&runtime.lsp, service))
    }

    /// The project's language service if it handles `doc`.
    fn lsp_for(&self, doc: &DocHandle) -> Option<Arc<LanguageService>> {
        doc.lsp_language?;
        self.runtime().ok().map(|runtime| runtime.lsp.clone())
    }

    pub(crate) fn doc(&self, id: DocId) -> Result<Arc<DocHandle>, IpcError> {
        lock(&self.state)?.docs.get(&id).cloned().ok_or_else(|| {
            IpcError::new(
                IpcErrorKind::NotFound,
                format!("document {} is not open", id.0),
            )
        })
    }

    /// Opens a folder, replacing the current project. Open documents of the
    /// previous project are closed; their unsaved text stays in recovery files.
    pub async fn open_project(
        self: &Arc<Self>,
        path: String,
    ) -> Result<ipc::ProjectInfo, IpcError> {
        let data_dir = self.data_dir.clone();
        let sink = self.sink.clone();
        let settings_file = self.settings_file.clone();
        let session = Arc::downgrade(self);
        let terminal_backend = self.terminal_backend.clone();
        let runtime = blocking(move || {
            let project = Project::open(Path::new(&path)).map_err(project_error)?;
            Ok(Arc::new(ProjectRuntime::new(
                project,
                &data_dir,
                sink,
                settings_file,
                session,
                terminal_backend,
            )))
        })
        .await?;
        let (previous_docs, previous_runtime) = {
            let mut state = lock(&self.state)?;
            let old = state.project.replace(runtime.clone());
            state.by_path.clear();
            (
                state.docs.drain().map(|(_, doc)| doc).collect::<Vec<_>>(),
                old,
            )
        };
        if let Some(old) = previous_runtime {
            old.cancel.cancel();
            old.terminals.close_all();
            // Shut the old project's server down in the background.
            tokio::spawn(old.lsp.stop());
        }
        self.flush_closed(previous_docs).await;
        let project = runtime.project.clone();
        tracing::info!(root = ?project.root(), "project opened");

        // Index in the background; the tree is usable immediately.
        self.sink
            .emit(SessionEvent::IndexStatus(runtime.index_state()));
        let registry = self.registry.clone();
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let task =
                tokio::task::spawn_blocking(move || index_project(&runtime, &registry, &sink));
            if let Err(err) = task.await {
                tracing::error!(%err, "indexing task failed");
            }
        });
        Ok(ipc::ProjectInfo {
            name: project.name().to_owned(),
            root: project.root().to_string_lossy().into_owned(),
        })
    }

    async fn flush_closed(&self, docs: Vec<Arc<DocHandle>>) {
        if docs.is_empty() {
            return;
        }
        let recovery = self.recovery.clone();
        let result = tokio::task::spawn_blocking(move || {
            for doc in docs {
                doc.cancel_autosave();
                doc.sync_recovery(&recovery);
            }
        })
        .await;
        if let Err(err) = result {
            tracing::error!(%err, "flushing recovery files failed");
        }
    }

    /// Lists a project directory (`""` is the root).
    pub async fn list_dir(&self, path: String) -> Result<Vec<ipc::DirEntry>, IpcError> {
        let project = self.project()?;
        let entries = blocking(move || project.list_dir(&path).map_err(project_error)).await?;
        Ok(entries
            .into_iter()
            .map(|e| ipc::DirEntry {
                name: e.name,
                path: e.path,
                kind: match e.kind {
                    EntryKind::Directory => ipc::EntryKind::Directory,
                    EntryKind::File => ipc::EntryKind::File,
                },
                ignored: e.ignored,
            })
            .collect())
    }

    /// Opens a document, or returns the already open one. If a recovery
    /// file with different text exists, that text is restored as an unsaved
    /// change (undo returns to the text on disk).
    pub async fn open_document(&self, path: String) -> Result<ipc::OpenedDocument, IpcError> {
        let runtime = self.runtime()?;
        let abs = runtime.project.resolve(&path).map_err(project_error)?;
        // Packages are library code: shown read-only like files outside.
        let read_only = path.split('/').any(|part| part == "node_modules");
        self.open_path(runtime, abs, path, read_only).await
    }

    /// Opens a file outside the project (a definition in `lib.dom.d.ts`, a
    /// global package) read-only, by absolute path.
    pub async fn open_external(&self, path: String) -> Result<ipc::OpenedDocument, IpcError> {
        let runtime = self.runtime()?;
        let abs = PathBuf::from(&path);
        if !abs.is_absolute() {
            return Err(IpcError::new(
                IpcErrorKind::InvalidArgument,
                format!("{path} is not an absolute path"),
            ));
        }
        if let Some(rel) = runtime.project.relativize(&abs) {
            return self.open_document(rel).await;
        }
        self.open_path(runtime, abs, path, true).await
    }

    async fn open_path(
        &self,
        runtime: Arc<ProjectRuntime>,
        abs: PathBuf,
        path: String,
        read_only: bool,
    ) -> Result<ipc::OpenedDocument, IpcError> {
        if let Some(existing) = self.doc_by_path(&abs)? {
            return existing.opened();
        }
        let recent_path = path.clone();

        let recovery = self.recovery.clone();
        let registry = self.registry.clone();
        let now = self.now_ms();
        let abs_for_load = abs.clone();
        let (state, language) = blocking(move || {
            let file = ferrule_vfs::read_text(&abs_for_load).map_err(vfs_error)?;
            let saved_text = Rope::from_str(&file.text);
            let editorconfig = resolve_editorconfig(&abs_for_load);
            let indent_unit = editorconfig.indent_unit(&detect_indent_unit(&file.text));
            let tab_width = editorconfig.display_tab_width();
            let mut doc = Document::new(&file.text);
            let mut restored = false;
            if let Some(entry) = recovery.load(&abs_for_load) {
                if entry.text == file.text {
                    recovery.remove(&abs_for_load).map_err(|e| io_error(&e))?;
                } else {
                    let replace = Transaction {
                        edits: vec![Edit::new(0, doc.len_utf16(), entry.text)],
                        kind: EditKind::Other,
                        selection_before: Selection::cursor(0),
                        selection_after: Selection::cursor(0),
                    };
                    doc.apply(0, &[replace], now).map_err(buffer_error)?;
                    restored = true;
                    tracing::info!(path = ?abs_for_load, "restored unsaved changes from recovery file");
                }
            }
            let language = registry.for_path(&abs_for_load);
            let name = language.as_ref().map(|l| l.name().to_owned());
            let auto_close: Vec<String> = language
                .as_ref()
                .map_or(DEFAULT_AUTO_CLOSE, |l| l.auto_close())
                .iter()
                .map(|p| (*p).to_owned())
                .collect();
            let indent_rules = IndentRules {
                markup_tags: language.as_ref().is_some_and(|l| l.markup_indent()),
            };
            let syntax = match language {
                Some(lang) => Some(
                    SyntaxTree::new(lang)
                        .map_err(|e| IpcError::new(IpcErrorKind::Internal, e.to_string()))?
                        .with_registry(registry.clone()),
                ),
                None => None,
            };
            Ok((
                DocState {
                    doc,
                    syntax,
                    line_ending: file.line_ending,
                    bom: file.bom,
                    saved_version: 0,
                    saved_text,
                    restored,
                    indent_unit,
                    tab_width,
                    auto_close,
                    editorconfig,
                    indent_rules,
                    lsp_key: None,
                },
                name,
            ))
        })
        .await?;
        if !read_only {
            let touch = runtime.clone();
            // Best effort: a failed recent-files write is logged, not reported.
            tokio::task::spawn_blocking(move || touch.touch_recent(&recent_path));
        }

        let handle = {
            let mut s = lock(&self.state)?;
            // Another request may have opened it while we were loading.
            if let Some(id) = s.by_path.get(&abs).copied()
                && let Some(existing) = s.docs.get(&id).cloned()
            {
                drop(s);
                return existing.opened();
            }
            s.next_id += 1;
            let id = DocId(s.next_id);
            let handle = Arc::new(DocHandle {
                id,
                uri: ferrule_lsp_client::path_to_uri(&abs),
                lsp_language: runtime.lsp.language_id(&abs),
                read_only,
                path: abs.clone(),
                rel_path: path,
                language,
                state: Mutex::new(state),
                autosave: Mutex::new(None),
            });
            s.docs.insert(id, handle.clone());
            s.by_path.insert(abs, id);
            handle
        };
        if handle.lsp_language.is_some() {
            // Start the server (if needed) and open the document on it in the
            // background; the editor never waits for it.
            let service = runtime.lsp.clone();
            let doc = handle.clone();
            tokio::task::spawn_blocking(move || {
                service.ensure_started();
                if let Ok(mut state) = doc.lock() {
                    service.sync(&doc, &mut state, &[]);
                }
            });
        }
        handle.opened()
    }

    fn doc_by_path(&self, abs: &Path) -> Result<Option<Arc<DocHandle>>, IpcError> {
        let s = lock(&self.state)?;
        Ok(s.by_path.get(abs).and_then(|id| s.docs.get(id)).cloned())
    }

    /// Applies view transactions based on `base_version`.
    pub async fn apply_changes(
        self: &Arc<Self>,
        doc_id: DocId,
        base_version: u32,
        transactions: Vec<ipc::ChangeTransaction>,
    ) -> Result<ipc::DocStatus, IpcError> {
        let handle = self.doc(doc_id)?;
        refuse_read_only(&handle)?;
        let now = self.now_ms();
        let h = handle.clone();
        let lsp = self.lsp_for(&handle);
        let trace = self.trace.clone();
        let status = blocking(move || {
            let _span = tracing::debug_span!("apply_changes", doc = doc_id.0).entered();
            let started = Instant::now();
            let txs: Vec<Transaction> = transactions
                .iter()
                .map(|t| Transaction {
                    edits: t.edits.iter().map(edit_from_ipc).collect(),
                    kind: kind_from_ipc(t.kind),
                    selection_before: selection_from_ipc(&t.selection_before),
                    selection_after: selection_from_ipc(&t.selection_after),
                })
                .collect();
            let mut state = h.lock()?;
            let outcome = state
                .doc
                .apply(base_version, &txs, now)
                .map_err(buffer_error)?;
            if let Some(syntax) = &mut state.syntax {
                syntax.edit(&outcome.byte_edits);
            }
            if let Some(lsp) = &lsp {
                lsp.sync(&h, &mut state, &outcome.lsp_changes);
            }
            if let Some(trace) = &trace {
                trace.record("rust.apply_changes", started.elapsed());
            }
            Ok(state.status())
        })
        .await?;
        self.schedule_autosave(handle);
        Ok(status)
    }

    /// Undoes the last edit group. `None` if there is nothing to undo.
    pub async fn undo(
        self: &Arc<Self>,
        doc_id: DocId,
        version: u32,
    ) -> Result<Option<ipc::RemoteUpdate>, IpcError> {
        self.history_step(doc_id, version, true).await
    }

    /// Redoes the last undone group. `None` if there is nothing to redo.
    pub async fn redo(
        self: &Arc<Self>,
        doc_id: DocId,
        version: u32,
    ) -> Result<Option<ipc::RemoteUpdate>, IpcError> {
        self.history_step(doc_id, version, false).await
    }

    async fn history_step(
        self: &Arc<Self>,
        doc_id: DocId,
        version: u32,
        undo: bool,
    ) -> Result<Option<ipc::RemoteUpdate>, IpcError> {
        let handle = self.doc(doc_id)?;
        let h = handle.clone();
        let lsp = self.lsp_for(&handle);
        let update = blocking(move || {
            let mut state = h.lock()?;
            let outcome = if undo {
                state.doc.undo(version)
            } else {
                state.doc.redo(version)
            }
            .map_err(buffer_error)?;
            let Some(outcome) = outcome else {
                return Ok(None);
            };
            if let Some(syntax) = &mut state.syntax {
                syntax.edit(&outcome.byte_edits);
            }
            if let Some(lsp) = &lsp {
                lsp.sync(&h, &mut state, &outcome.lsp_changes);
            }
            Ok(Some(ipc::RemoteUpdate {
                updates: outcome
                    .updates
                    .iter()
                    .map(|edits| edits.iter().map(edit_to_ipc).collect())
                    .collect(),
                selection: selection_to_ipc(&outcome.selection),
                status: state.status(),
            }))
        })
        .await?;
        if update.is_some() {
            self.schedule_autosave(handle);
        }
        Ok(update)
    }

    /// Clones the caret one line up or down.
    pub async fn add_caret(
        &self,
        doc_id: DocId,
        version: u32,
        selection: ipc::EditorSelection,
        direction: ipc::CaretDirection,
    ) -> Result<ipc::EditorSelection, IpcError> {
        let direction = match direction {
            ipc::CaretDirection::Up => ferrule_buffer::CaretDirection::Up,
            ipc::CaretDirection::Down => ferrule_buffer::CaretDirection::Down,
        };
        self.selection_command(doc_id, version, selection, move |doc, sel| {
            doc.add_caret(sel, direction)
        })
        .await
    }

    /// Adds the next occurrence of the main selection (or selects the word).
    pub async fn select_next_occurrence(
        &self,
        doc_id: DocId,
        version: u32,
        selection: ipc::EditorSelection,
    ) -> Result<ipc::EditorSelection, IpcError> {
        self.selection_command(doc_id, version, selection, Document::select_next_occurrence)
            .await
    }

    /// Selects all occurrences of the main selection (or of the word).
    pub async fn select_all_occurrences(
        &self,
        doc_id: DocId,
        version: u32,
        selection: ipc::EditorSelection,
    ) -> Result<ipc::EditorSelection, IpcError> {
        self.selection_command(doc_id, version, selection, Document::select_all_occurrences)
            .await
    }

    async fn selection_command(
        &self,
        doc_id: DocId,
        version: u32,
        selection: ipc::EditorSelection,
        command: impl FnOnce(&Document, &Selection) -> Selection + Send + 'static,
    ) -> Result<ipc::EditorSelection, IpcError> {
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let state = handle.lock()?;
            if state.doc.version() != version {
                return Err(buffer_error(ferrule_buffer::BufferError::VersionMismatch {
                    expected: version,
                    actual: state.doc.version(),
                }));
            }
            let result = command(&state.doc, &selection_from_ipc(&selection));
            Ok(selection_to_ipc(&result))
        })
        .await
    }

    /// Highlight spans for the UTF-16 range `from..to` of the current version.
    pub async fn highlights(
        &self,
        doc_id: DocId,
        from: u32,
        to: u32,
    ) -> Result<ipc::Highlights, IpcError> {
        let handle = self.doc(doc_id)?;
        let trace = self.trace.clone();
        blocking(move || {
            let _span = tracing::debug_span!("highlights", doc = doc_id.0).entered();
            let started = Instant::now();
            let mut guard = handle.lock()?;
            let state = &mut *guard;
            let version = state.doc.version();
            let Some(syntax) = &mut state.syntax else {
                return Ok(ipc::Highlights {
                    version,
                    spans: Vec::new(),
                });
            };
            let doc = &state.doc;
            let start = doc.utf16_to_byte_clamped(to_usize(from));
            let end = doc.utf16_to_byte_clamped(to_usize(to));
            let spans = syntax
                .highlight(doc.rope(), start..end)
                .into_iter()
                .map(|s| ipc::HighlightSpan {
                    from: to_u32(doc.byte_to_utf16(s.start)),
                    to: to_u32(doc.byte_to_utf16(s.end)),
                    class: token_to_ipc(s.class),
                })
                .collect();
            if let Some(trace) = &trace {
                trace.record("rust.highlights", started.elapsed());
            }
            Ok(ipc::Highlights { version, spans })
        })
        .await
    }

    /// Enter at every caret: newline plus language-aware indentation,
    /// recorded as typing so it undoes together with the surrounding text.
    pub async fn new_line(
        self: &Arc<Self>,
        doc_id: DocId,
        version: u32,
        selection: ipc::EditorSelection,
    ) -> Result<ipc::RemoteUpdate, IpcError> {
        let handle = self.doc(doc_id)?;
        refuse_read_only(&handle)?;
        let now = self.now_ms();
        let h = handle.clone();
        let lsp = self.lsp_for(&handle);
        let update = blocking(move || {
            let mut state = h.lock()?;
            let before = selection_from_ipc(&selection);
            let (edits, after) =
                state
                    .doc
                    .newline_and_indent(&before, &state.indent_unit, state.indent_rules);
            let tx = Transaction {
                edits: edits.clone(),
                kind: EditKind::Typing,
                selection_before: before,
                selection_after: after.clone(),
            };
            let outcome = state.doc.apply(version, &[tx], now).map_err(buffer_error)?;
            if let Some(syntax) = &mut state.syntax {
                syntax.edit(&outcome.byte_edits);
            }
            if let Some(lsp) = &lsp {
                lsp.sync(&h, &mut state, &outcome.lsp_changes);
            }
            Ok(ipc::RemoteUpdate {
                updates: vec![edits.iter().map(edit_to_ipc).collect()],
                selection: selection_to_ipc(&after),
                status: state.status(),
            })
        })
        .await?;
        self.schedule_autosave(handle);
        Ok(update)
    }

    /// Reloads a document from disk. The change is one undoable step (only
    /// the differing middle part is replaced, so carets elsewhere stay put);
    /// afterwards the document is unmodified. No edits if the text matches.
    pub async fn reload_document(
        &self,
        doc_id: DocId,
        version: u32,
    ) -> Result<ipc::DocumentReload, IpcError> {
        let handle = self.doc(doc_id)?;
        handle.cancel_autosave();
        let recovery = self.recovery.clone();
        let now = self.now_ms();
        let lsp = self.lsp_for(&handle);
        blocking(move || {
            let file = ferrule_vfs::read_text(&handle.path).map_err(vfs_error)?;
            let mut state = handle.lock()?;
            if state.doc.version() != version {
                return Err(buffer_error(ferrule_buffer::BufferError::VersionMismatch {
                    expected: version,
                    actual: state.doc.version(),
                }));
            }
            let current = state.doc.text();
            let mut updates = Vec::new();
            if let Some(edit) = minimal_edit(&state.doc, &current, &file.text) {
                let tx = Transaction {
                    edits: vec![edit.clone()],
                    kind: EditKind::Other,
                    selection_before: Selection::cursor(edit.from),
                    selection_after: Selection::cursor(edit.from),
                };
                let outcome = state.doc.apply(version, &[tx], now).map_err(buffer_error)?;
                if let Some(syntax) = &mut state.syntax {
                    syntax.edit(&outcome.byte_edits);
                }
                if let Some(lsp) = &lsp {
                    lsp.sync(&handle, &mut state, &outcome.lsp_changes);
                }
                updates.push(vec![edit_to_ipc(&edit)]);
            }
            state.line_ending = file.line_ending;
            state.bom = file.bom;
            state.mark_saved();
            recovery.remove(&handle.path).map_err(|e| io_error(&e))?;
            Ok(ipc::DocumentReload {
                updates,
                status: state.status(),
            })
        })
        .await
    }

    /// Whether the file on disk differs from the document's last loaded or
    /// saved text (so a change event is not just our own save). A missing
    /// file counts as changed.
    pub async fn disk_changed(&self, doc_id: DocId) -> Result<bool, IpcError> {
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let disk = match ferrule_vfs::read_text(&handle.path) {
                Ok(file) => file.text,
                Err(_) => return Ok(true),
            };
            let state = handle.lock()?;
            Ok(state.saved_text != disk.as_str())
        })
        .await
    }

    /// Saves a document atomically with its original line endings and BOM,
    /// and deletes its recovery file.
    pub async fn save_document(&self, doc_id: DocId) -> Result<ipc::SaveResult, IpcError> {
        let handle = self.doc(doc_id)?;
        refuse_read_only(&handle)?;
        handle.cancel_autosave();
        let recovery = self.recovery.clone();
        let now = self.now_ms();
        let lsp = self.lsp_for(&handle);
        blocking(move || {
            let mut guard = handle.lock()?;
            let state = &mut *guard;
            // EditorConfig: whitespace and final newline as one undoable step.
            let edits = save_edits(&state.doc, &state.editorconfig);
            let mut updates = Vec::new();
            if !edits.is_empty() {
                let tx = Transaction {
                    edits: edits.clone(),
                    kind: EditKind::Other,
                    selection_before: Selection::cursor(0),
                    selection_after: Selection::cursor(0),
                };
                let outcome = state
                    .doc
                    .apply(state.doc.version(), &[tx], now)
                    .map_err(buffer_error)?;
                if let Some(syntax) = &mut state.syntax {
                    syntax.edit(&outcome.byte_edits);
                }
                if let Some(lsp) = &lsp {
                    lsp.sync(&handle, state, &outcome.lsp_changes);
                }
                updates.push(edits.iter().map(edit_to_ipc).collect());
            }
            match state.editorconfig.end_of_line {
                Some(EndOfLine::Lf) => state.line_ending = LineEnding::Lf,
                Some(EndOfLine::CrLf) => state.line_ending = LineEnding::CrLf,
                Some(EndOfLine::Cr) => tracing::warn!(
                    "end_of_line = cr is not supported; keeping the file's line endings"
                ),
                None => {}
            }
            match state.editorconfig.charset.as_deref() {
                Some("utf-8") => state.bom = false,
                Some("utf-8-bom") => state.bom = true,
                Some(other) => {
                    tracing::warn!(charset = other, "only UTF-8 is supported; saving as UTF-8")
                }
                None => {}
            }
            let bytes = ferrule_vfs::encode(&state.doc.text(), state.line_ending, state.bom);
            ferrule_vfs::write_atomic(&handle.path, &bytes).map_err(|e| io_error(&e))?;
            state.mark_saved();
            recovery.remove(&handle.path).map_err(|e| io_error(&e))?;
            if let Some(lsp) = &lsp {
                lsp.saved(&handle, state);
            }
            tracing::info!(path = ?handle.path, "saved");
            Ok(ipc::SaveResult {
                updates,
                status: state.status(),
            })
        })
        .await
    }

    /// Breadcrumbs and the bracket pair at a UTF-16 offset.
    pub async fn caret_info(&self, doc_id: DocId, offset: u32) -> Result<ipc::CaretInfo, IpcError> {
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let mut guard = handle.lock()?;
            let state = &mut *guard;
            let doc = &state.doc;
            let version = doc.version();
            let byte = doc.utf16_to_byte_clamped(to_usize(offset));
            let Some(syntax) = &mut state.syntax else {
                return Ok(ipc::CaretInfo {
                    version,
                    breadcrumbs: Vec::new(),
                    brackets: None,
                });
            };
            let breadcrumbs = syntax
                .breadcrumbs(doc.rope(), byte)
                .into_iter()
                .map(|c| ipc::Crumb {
                    label: c.label,
                    kind: c.kind,
                    offset: to_u32(doc.byte_to_utf16(c.start)),
                })
                .collect();
            let brackets = syntax
                .bracket_pair(doc.rope(), byte)
                .map(|p| ipc::BracketMatch {
                    open_from: to_u32(doc.byte_to_utf16(p.open.start)),
                    open_to: to_u32(doc.byte_to_utf16(p.open.end)),
                    close_from: to_u32(doc.byte_to_utf16(p.close.start)),
                    close_to: to_u32(doc.byte_to_utf16(p.close.end)),
                });
            Ok(ipc::CaretInfo {
                version,
                breadcrumbs,
                brackets,
            })
        })
        .await
    }

    /// Folding ranges of the current version.
    pub async fn folds(&self, doc_id: DocId) -> Result<ipc::FoldRanges, IpcError> {
        let handle = self.doc(doc_id)?;
        blocking(move || {
            let mut guard = handle.lock()?;
            let state = &mut *guard;
            let doc = &state.doc;
            let ranges = match &mut state.syntax {
                Some(syntax) => syntax
                    .folds(doc.rope())
                    .into_iter()
                    .map(|f| ipc::FoldSpan {
                        from: to_u32(doc.byte_to_utf16(f.start)),
                        to: to_u32(doc.byte_to_utf16(f.end)),
                    })
                    .collect(),
                None => Vec::new(),
            };
            Ok(ipc::FoldRanges {
                version: doc.version(),
                ranges,
            })
        })
        .await
    }

    /// Closes a document. Unsaved text is kept in its recovery file and
    /// restored when the file is opened again.
    pub async fn close_document(&self, doc_id: DocId) -> Result<(), IpcError> {
        let handle = {
            let mut s = lock(&self.state)?;
            let handle = s.docs.remove(&doc_id).ok_or_else(|| {
                IpcError::new(
                    IpcErrorKind::NotFound,
                    format!("document {} is not open", doc_id.0),
                )
            })?;
            s.by_path.remove(&handle.path);
            handle
        };
        if let Some(lsp) = self.lsp_for(&handle) {
            let doc = handle.clone();
            blocking(move || {
                let mut state = doc.lock()?;
                lsp.closed(&doc, &mut state);
                Ok(())
            })
            .await?;
        }
        self.flush_closed(vec![handle]).await;
        Ok(())
    }

    /// Whether typing-path timing is on (`FERRULE_TYPING_TRACE`).
    pub fn typing_trace_enabled(&self) -> bool {
        self.trace.is_some()
    }

    /// Appends the view's typing-path samples (and Rust's own) to the trace.
    pub async fn record_typing_trace(
        &self,
        samples: Vec<ipc::TraceSample>,
    ) -> Result<(), IpcError> {
        let Some(trace) = self.trace.clone() else {
            return Ok(());
        };
        blocking(move || trace.flush(&samples).map_err(|e| io_error(&e))).await
    }

    fn schedule_autosave(self: &Arc<Self>, handle: Arc<DocHandle>) {
        let token = CancellationToken::new();
        match lock(&handle.autosave) {
            Ok(mut slot) => {
                if let Some(previous) = slot.replace(token.clone()) {
                    previous.cancel();
                }
            }
            Err(err) => {
                tracing::error!(%err, "autosave lock poisoned");
                return;
            }
        }
        let recovery = self.recovery.clone();
        let delay = self.autosave_delay;
        tokio::spawn(async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = tokio::time::sleep(delay) => {
                    let result = tokio::task::spawn_blocking(move || handle.sync_recovery(&recovery)).await;
                    if let Err(err) = result {
                        tracing::error!(%err, "recovery task failed");
                    }
                }
            }
        });
    }
}

fn refuse_read_only(doc: &DocHandle) -> Result<(), IpcError> {
    if doc.read_only {
        Err(IpcError::new(
            IpcErrorKind::Conflict,
            format!("{} is read-only (library code)", doc.rel_path),
        ))
    } else {
        Ok(())
    }
}

/// The single edit turning `old` into `new`: everything between their
/// common prefix and common suffix. `None` if they are equal.
fn minimal_edit(doc: &Document, old: &str, new: &str) -> Option<Edit> {
    if old == new {
        return None;
    }
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let max_suffix = old.len().min(new.len()) - prefix;
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let from = doc.byte_to_utf16(prefix);
    let to = doc.byte_to_utf16(old.len() - suffix);
    Some(Edit::new(from, to, &new[prefix..new.len() - suffix]))
}

/// Edits applied on save by EditorConfig: `trim_trailing_whitespace` and
/// `insert_final_newline` (sorted, UTF-16, relative to the current text).
fn save_edits(doc: &Document, config: &EditorConfig) -> Vec<Edit> {
    let text = doc.text();
    let mut edits = Vec::new();
    if config.trim_trailing_whitespace == Some(true) {
        let mut line_start = 0;
        for line in text.split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let trimmed = body.trim_end_matches([' ', '\t']);
            if trimmed.len() < body.len() {
                let from = doc.byte_to_utf16(line_start + trimmed.len());
                let to = doc.byte_to_utf16(line_start + body.len());
                edits.push(Edit::new(from, to, ""));
            }
            line_start += line.len();
        }
    }
    match config.insert_final_newline {
        Some(true) if !text.is_empty() && !text.ends_with('\n') => {
            let end = doc.len_utf16();
            // Merge with a trim edit at the very end, keeping edits disjoint.
            match edits.last_mut() {
                Some(last) if last.to == end => last.insert.push('\n'),
                _ => edits.push(Edit::new(end, end, "\n")),
            }
        }
        Some(false) if text.ends_with('\n') => {
            let kept = text.trim_end_matches('\n').len();
            let from = doc.byte_to_utf16(kept);
            edits.retain(|e| e.from < from);
            edits.push(Edit::new(from, doc.len_utf16(), ""));
        }
        _ => {}
    }
    edits
}
