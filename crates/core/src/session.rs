//! The editing session: the open project and its documents.
//!
//! Concurrency (ARCHITECTURE.md, section 6): the session map and each
//! document sit behind their own `std::sync::Mutex`. Locks are held only for
//! short synchronous sections and never across `.await`; all file IO,
//! parsing and editing runs inside `spawn_blocking`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ferrule_buffer::{Document, Edit, EditKind, LineEnding, Selection, Transaction};
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_project::{EntryKind, Project};
use ferrule_syntax::{LanguageRegistry, SyntaxError, SyntaxTree};
use ferrule_vfs::RecoveryStore;
use tokio_util::sync::CancellationToken;

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

struct DocState {
    doc: Document,
    syntax: Option<SyntaxTree>,
    line_ending: LineEnding,
    bom: bool,
    saved_version: u32,
    restored: bool,
}

impl DocState {
    fn dirty(&self) -> bool {
        self.doc.version() != self.saved_version
    }

    fn status(&self) -> ipc::DocStatus {
        ipc::DocStatus {
            version: self.doc.version(),
            dirty: self.dirty(),
        }
    }
}

struct DocHandle {
    id: DocId,
    path: PathBuf,
    rel_path: String,
    language: Option<String>,
    state: Mutex<DocState>,
    autosave: Mutex<Option<CancellationToken>>,
}

impl DocHandle {
    fn lock(&self) -> Result<MutexGuard<'_, DocState>, IpcError> {
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
struct State {
    project: Option<Arc<Project>>,
    docs: HashMap<DocId, Arc<DocHandle>>,
    by_path: HashMap<PathBuf, DocId>,
    next_id: u32,
}

/// The editing session. Cheap to share as `Arc<Session>`.
pub struct Session {
    state: Mutex<State>,
    recovery: RecoveryStore,
    registry: Arc<LanguageRegistry>,
    autosave_delay: Duration,
    started: Instant,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, IpcError> {
    mutex
        .lock()
        .map_err(|_| IpcError::new(IpcErrorKind::Internal, "internal state lock poisoned"))
}

async fn blocking<T: Send + 'static>(
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
    {
        registry.register(def)?;
    }
    Ok(registry)
}

impl Session {
    /// Creates a session that keeps recovery files in `recovery_dir`.
    pub fn new(recovery_dir: impl Into<PathBuf>) -> Result<Arc<Self>, SessionError> {
        Self::with_autosave_delay(recovery_dir, DEFAULT_AUTOSAVE_DELAY)
    }

    /// Like [`Session::new`] with a custom recovery-file delay (tests).
    pub fn with_autosave_delay(
        recovery_dir: impl Into<PathBuf>,
        autosave_delay: Duration,
    ) -> Result<Arc<Self>, SessionError> {
        Ok(Arc::new(Self {
            state: Mutex::new(State::default()),
            recovery: RecoveryStore::new(recovery_dir),
            registry: Arc::new(languages()?),
            autosave_delay,
            started: Instant::now(),
        }))
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn project(&self) -> Result<Arc<Project>, IpcError> {
        lock(&self.state)?
            .project
            .clone()
            .ok_or_else(|| IpcError::new(IpcErrorKind::Conflict, "no project is open"))
    }

    fn doc(&self, id: DocId) -> Result<Arc<DocHandle>, IpcError> {
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
        let project =
            blocking(move || Project::open(Path::new(&path)).map_err(project_error)).await?;
        let previous: Vec<Arc<DocHandle>> = {
            let mut state = lock(&self.state)?;
            state.project = Some(Arc::new(project.clone()));
            state.by_path.clear();
            state.docs.drain().map(|(_, doc)| doc).collect()
        };
        self.flush_closed(previous).await;
        tracing::info!(root = ?project.root(), "project opened");
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
            })
            .collect())
    }

    /// Opens a document, or returns the already open one. If a recovery
    /// file with different text exists, that text is restored as an unsaved
    /// change (undo returns to the text on disk).
    pub async fn open_document(&self, path: String) -> Result<ipc::OpenedDocument, IpcError> {
        let project = self.project()?;
        let abs = project.resolve(&path).map_err(project_error)?;
        if let Some(existing) = self.doc_by_path(&abs)? {
            return existing.opened();
        }

        let recovery = self.recovery.clone();
        let registry = self.registry.clone();
        let now = self.now_ms();
        let abs_for_load = abs.clone();
        let (state, language) = blocking(move || {
            let file = ferrule_vfs::read_text(&abs_for_load).map_err(vfs_error)?;
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
            let syntax = match language {
                Some(lang) => Some(SyntaxTree::new(lang).map_err(|e| {
                    IpcError::new(IpcErrorKind::Internal, e.to_string())
                })?),
                None => None,
            };
            Ok((
                DocState {
                    doc,
                    syntax,
                    line_ending: file.line_ending,
                    bom: file.bom,
                    saved_version: 0,
                    restored,
                },
                name,
            ))
        })
        .await?;

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
        let now = self.now_ms();
        let h = handle.clone();
        let status = blocking(move || {
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
        blocking(move || {
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
            Ok(ipc::Highlights { version, spans })
        })
        .await
    }

    /// Saves a document atomically with its original line endings and BOM,
    /// and deletes its recovery file.
    pub async fn save_document(&self, doc_id: DocId) -> Result<ipc::DocStatus, IpcError> {
        let handle = self.doc(doc_id)?;
        handle.cancel_autosave();
        let recovery = self.recovery.clone();
        blocking(move || {
            let mut state = handle.lock()?;
            let bytes = ferrule_vfs::encode(&state.doc.text(), state.line_ending, state.bom);
            ferrule_vfs::write_atomic(&handle.path, &bytes).map_err(|e| io_error(&e))?;
            state.saved_version = state.doc.version();
            state.restored = false;
            recovery.remove(&handle.path).map_err(|e| io_error(&e))?;
            tracing::info!(path = ?handle.path, "saved");
            Ok(state.status())
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
        self.flush_closed(vec![handle]).await;
        Ok(())
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
