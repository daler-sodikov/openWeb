//! Per-project background state: the index, its cache, the file watcher and
//! the recent-files list.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use crate::lsp::LanguageService;
use crate::session::Session;
use std::time::Duration;

use ferrule_indexer::Index;
use ferrule_ipc::{FsChange, FsChangeBatch, FsChangeKind, IndexState};
use ferrule_project::Project;
use ferrule_syntax::LanguageRegistry;
use ferrule_vfs::Watcher;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::convert::to_u32;
use crate::events::{EventSink, SessionEvent};

/// Delay after the last file-system event before a batch is processed.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);
/// Maximum number of recent files kept per project.
const MAX_RECENT: usize = 50;
/// Prefix of the temporary files written by atomic saves.
const SAVE_TEMP_PREFIX: &str = ".ferrule-save-";

#[derive(Serialize, Deserialize, Default)]
struct ProjectFile {
    format: u32,
    recent: Vec<String>,
    /// The user allowed this project to run commands (M5). Stored here, in
    /// Ferrule's data folder, never in the project.
    #[serde(default)]
    trusted: bool,
}

pub(crate) struct ProjectRuntime {
    pub project: Arc<Project>,
    pub index: RwLock<Option<Index>>,
    watcher: Mutex<Option<Watcher>>,
    pub cancel: CancellationToken,
    cache_file: PathBuf,
    project_file: PathBuf,
    pub recent: Mutex<Vec<String>>,
    /// The project's TypeScript language server.
    pub lsp: Arc<LanguageService>,
    /// The project's terminals.
    pub terminals: ferrule_terminal::Terminals,
    /// The Git repository containing the project.
    pub git: Arc<crate::git::GitState>,
    /// Runs of configurations and tests.
    pub runs: Arc<crate::runs::Runs>,
    /// Debug sessions (each one is also a run).
    pub debug: Arc<crate::debug::Debugs>,
    /// Whether the user trusted the project to run commands.
    trusted: AtomicBool,
    /// The background index build (awaited when the project closes).
    pub index_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// The Git repository discovery and first status, which run `git` in
    /// the project folder (awaited when the project closes).
    pub git_start: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// How long closing a project waits for each kind of work to stop.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// "terminal 3 (processes 120, 121)" for a close report.
fn terminal_still_running(id: u32, pids: &[u32]) -> String {
    if pids.is_empty() {
        format!("terminal {id}")
    } else {
        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
        format!("terminal {id} (processes {})", pids.join(", "))
    }
}

/// Awaits a background task, at most [`CLOSE_WAIT`]; `false` if it was
/// still running then.
async fn finished(slot: &Mutex<Option<tokio::task::JoinHandle<()>>>) -> bool {
    let task = slot.lock().ok().and_then(|mut t| t.take());
    match task {
        Some(task) => tokio::time::timeout(CLOSE_WAIT, task).await.is_ok(),
        None => true,
    }
}

impl Drop for ProjectRuntime {
    fn drop(&mut self) {
        // Stops a still-running index build for a project that was replaced.
        self.cancel.cancel();
        self.terminals.close_all();
    }
}

/// The pluggable backends a project's services start with.
pub(crate) struct Backends {
    /// PTY and emulator for terminals.
    pub terminal: Option<Arc<dyn ferrule_terminal::Backend>>,
    /// PTY and emulator for runs.
    pub run: Option<Arc<dyn ferrule_terminal::Backend>>,
    /// Attaches to a program started for debugging.
    pub debug: Option<Arc<dyn crate::debug::DebugConnector>>,
}

impl ProjectRuntime {
    pub fn new(
        project: Project,
        data_dir: &Path,
        sink: Arc<dyn EventSink>,
        settings_file: PathBuf,
        session: Weak<Session>,
        backends: Backends,
    ) -> Self {
        let Backends {
            terminal: terminal_backend,
            run: run_backend,
            debug: debug_connector,
        } = backends;
        let hash = ferrule_vfs::path_hash(project.root());
        let project_file = data_dir.join("projects").join(format!("{hash}.json"));
        let stored = std::fs::read(&project_file)
            .ok()
            .and_then(|b| serde_json::from_slice::<ProjectFile>(&b).ok())
            .unwrap_or_default();
        let project = Arc::new(project);
        let terminals = crate::terminal::project_terminals(terminal_backend, sink.clone());
        let runs = crate::runs::Runs::new(
            run_backend,
            sink.clone(),
            project.root().to_path_buf(),
            data_dir.join("runs").join(&hash),
        );
        let debug = crate::debug::Debugs::new(sink.clone(), debug_connector);
        let git = Arc::new(crate::git::GitState::new(sink.clone()));
        let lsp = Arc::new(LanguageService::new(
            ferrule_lang_js::typescript_server(),
            project.clone(),
            sink,
            settings_file,
            session,
        ));
        Self {
            project,
            lsp,
            index: RwLock::new(None),
            watcher: Mutex::new(None),
            cancel: CancellationToken::new(),
            cache_file: data_dir.join("index").join(format!("{hash}.json")),
            project_file,
            recent: Mutex::new(stored.recent),
            terminals,
            git,
            runs,
            debug,
            trusted: AtomicBool::new(stored.trusted),
            index_task: Mutex::new(None),
            git_start: Mutex::new(None),
        }
    }

    /// Ends everything this project started and waits until it has
    /// stopped (each kind of work at most a few seconds): terminals and
    /// their processes, the language server and the processes it started,
    /// Git discovery and refresh (which run `git` in the project folder),
    /// indexing and the file watcher (including its handles on Windows).
    /// Returns what was still running when the waits ran out.
    pub async fn shut_down(self: &Arc<Self>) -> Vec<String> {
        let mut still_running = Vec::new();
        self.cancel.cancel();
        // Debug sessions first: their `Ended` events go out while the
        // runtime is alive, and their tasks are gone before the runs close.
        self.debug.shut_down().await;
        let me = self.clone();
        match tokio::task::spawn_blocking(move || me.terminals.close_all_and_wait(CLOSE_WAIT)).await
        {
            Ok(left) => still_running.extend(
                left.iter()
                    .map(|(id, pids)| terminal_still_running(*id, pids)),
            ),
            Err(_) => still_running.push("terminals (closing failed)".to_owned()),
        }
        let me = self.clone();
        match tokio::task::spawn_blocking(move || me.runs.terminals.close_all_and_wait(CLOSE_WAIT))
            .await
        {
            Ok(left) => still_running.extend(
                left.iter()
                    .map(|(id, pids)| format!("run {}", terminal_still_running(*id, pids))),
            ),
            Err(_) => still_running.push("runs (closing failed)".to_owned()),
        }
        // Waits for the server and the processes it started.
        if tokio::time::timeout(CLOSE_WAIT, self.lsp.stop())
            .await
            .is_err()
        {
            still_running.push("the TypeScript language server".to_owned());
        }
        if !finished(&self.git_start).await {
            still_running.push("Git repository discovery".to_owned());
        }
        // The index build creates the watcher when it ends: wait for it
        // before stopping the watcher.
        if !finished(&self.index_task).await {
            still_running.push("indexing".to_owned());
        }
        // Taken out of the mutex first: the watcher's batch thread locks it,
        // and Watcher::stop waits for that thread.
        let watcher = self.watcher.lock().ok().and_then(|mut w| w.take());
        if let Some(watcher) = watcher {
            let stopped = tokio::task::spawn_blocking(move || watcher.stop(CLOSE_WAIT)).await;
            if !matches!(stopped, Ok(true)) {
                still_running.push("the file watcher".to_owned());
            }
        }
        let git = self.git.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || git.stop(CLOSE_WAIT)).await,
            Ok(true)
        ) {
            still_running.push("a Git status refresh".to_owned());
        }
        if still_running.is_empty() {
            tracing::info!(root = ?self.project.root(), "project closed");
        } else {
            tracing::warn!(?still_running, "project closed; some work had not stopped");
        }
        still_running
    }

    pub fn index_state(&self) -> IndexState {
        let idle = IndexState {
            indexing: false,
            files: 0,
            symbols: 0,
        };
        let Ok(guard) = self.index.read() else {
            return idle;
        };
        match guard.as_ref() {
            Some(index) => IndexState {
                indexing: false,
                files: to_u32(index.len()),
                symbols: to_u32(index.files().map(|f| f.symbols.len()).sum()),
            },
            None => IndexState {
                indexing: true,
                ..idle
            },
        }
    }

    /// Moves `path` to the front of the recent-files list and persists it.
    pub fn touch_recent(&self, path: &str) {
        let recent = match self.recent.lock() {
            Ok(mut recent) => {
                recent.retain(|p| p != path);
                recent.insert(0, path.to_owned());
                recent.truncate(MAX_RECENT);
                recent.clone()
            }
            Err(_) => return,
        };
        self.save_project_file(recent);
    }

    /// Whether the user trusted the project to run commands.
    pub fn trusted(&self) -> bool {
        self.trusted.load(Ordering::Acquire)
    }

    /// Trusts the project or revokes the trust (persisted). Blocking.
    pub fn set_trusted(&self, trusted: bool) {
        self.trusted.store(trusted, Ordering::Release);
        let recent = self.recent.lock().map(|r| r.clone()).unwrap_or_default();
        self.save_project_file(recent);
    }

    fn save_project_file(&self, recent: Vec<String>) {
        let file = ProjectFile {
            format: 1,
            recent,
            trusted: self.trusted(),
        };
        let result = serde_json::to_vec(&file)
            .map_err(std::io::Error::other)
            .and_then(|json| {
                if let Some(dir) = self.project_file.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                ferrule_vfs::write_atomic(&self.project_file, &json)
            });
        if let Err(err) = result {
            tracing::warn!(%err, "saving the project's data failed");
        }
    }

    fn save_cache(&self, index: &Index) {
        if let Err(err) = index.save_cache(&self.cache_file) {
            tracing::warn!(%err, "saving the index cache failed");
        }
    }
}

/// Builds the index (reusing the on-disk cache), then starts watching. Runs
/// on a blocking thread; stops early when the project is replaced.
pub(crate) fn index_project(
    runtime: &Arc<ProjectRuntime>,
    registry: &Arc<LanguageRegistry>,
    sink: &Arc<dyn EventSink>,
) {
    let root = runtime.project.root().to_path_buf();
    let cached = Index::load_cache(&runtime.cache_file, &root);
    let cancel = runtime.cancel.clone();
    let started = std::time::Instant::now();
    let Some((index, stats)) = Index::build(&root, registry, cached, &|| cancel.is_cancelled())
    else {
        return;
    };
    tracing::info!(
        files = stats.files,
        parsed = stats.parsed,
        reused = stats.reused,
        elapsed_ms = started.elapsed().as_millis(),
        "project indexed"
    );
    runtime.save_cache(&index);
    if runtime.cancel.is_cancelled() {
        return;
    }
    // Start watching while holding the index write lock, *before* the index
    // becomes visible: a change made right after "index ready" is then never
    // missed (its batch waits for this lock). Lock order: index, then watcher.
    let Ok(mut slot) = runtime.index.write() else {
        return;
    };
    let weak: Weak<ProjectRuntime> = Arc::downgrade(runtime);
    let watch_registry = registry.clone();
    let event_sink = sink.clone();
    let watcher = Watcher::new(WATCH_DEBOUNCE, move |paths| {
        if let Some(runtime) = weak.upgrade() {
            on_fs_change(&runtime, &watch_registry, event_sink.as_ref(), paths);
        }
    });
    match watcher {
        Ok(mut watcher) => {
            for dir in index.dirs() {
                if let Err(err) = watcher.watch_dir(dir) {
                    tracing::warn!(?dir, %err, "cannot watch directory");
                }
            }
            tracing::info!(dirs = watcher.watched_count(), "watching project");
            if let Ok(mut watcher_slot) = runtime.watcher.lock() {
                *watcher_slot = Some(watcher);
            }
        }
        Err(err) => tracing::warn!(%err, "file watching unavailable"),
    }
    *slot = Some(index);
    drop(slot);
    sink.emit(SessionEvent::IndexStatus(runtime.index_state()));
}

/// Updates the index and the watched directories for a batch of changed
/// paths, then tells the frontend.
fn on_fs_change(
    runtime: &ProjectRuntime,
    registry: &LanguageRegistry,
    sink: &dyn EventSink,
    paths: Vec<PathBuf>,
) {
    let project = &runtime.project;
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| p.starts_with(project.root()))
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(SAVE_TEMP_PREFIX))
        })
        .collect();
    let mut changes = Vec::new();
    {
        // Lock order: index, then watcher (the only place holding both).
        let Ok(mut guard) = runtime.index.write() else {
            return;
        };
        let Some(index) = guard.as_mut() else {
            return;
        };
        let relevant: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| !index.is_excluded(p))
            .collect();
        if relevant.is_empty() {
            return;
        }
        let new_dirs = index.update(registry, &relevant);
        if let Ok(mut slot) = runtime.watcher.lock()
            && let Some(watcher) = slot.as_mut()
        {
            for dir in &new_dirs {
                if let Err(err) = watcher.watch_dir(dir) {
                    tracing::warn!(?dir, %err, "cannot watch directory");
                }
            }
            for path in relevant.iter().filter(|p| !p.exists()) {
                watcher.forget_below(path);
            }
        }
        for path in &relevant {
            if let Some(rel) = project.relativize(path) {
                changes.push(FsChange {
                    path: rel,
                    kind: if path.exists() {
                        FsChangeKind::Changed
                    } else {
                        FsChangeKind::Removed
                    },
                });
            }
        }
        runtime.save_cache(index);
        let watched: Vec<(PathBuf, bool)> =
            relevant.iter().map(|p| (p.clone(), p.exists())).collect();
        runtime.lsp.files_changed(&watched);
    }
    sink.emit(SessionEvent::FsChanged(FsChangeBatch { changes }));
    sink.emit(SessionEvent::IndexStatus(runtime.index_state()));
    runtime.git.request_refresh();
}
