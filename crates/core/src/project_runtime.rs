//! Per-project background state: the index, its cache, the file watcher and
//! the recent-files list.

use std::path::{Path, PathBuf};
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
}

impl Drop for ProjectRuntime {
    fn drop(&mut self) {
        // Stops a still-running index build for a project that was replaced.
        self.cancel.cancel();
        self.terminals.close_all();
    }
}

impl ProjectRuntime {
    pub fn new(
        project: Project,
        data_dir: &Path,
        sink: Arc<dyn EventSink>,
        settings_file: PathBuf,
        session: Weak<Session>,
        terminal_backend: Option<Arc<dyn ferrule_terminal::Backend>>,
    ) -> Self {
        let hash = ferrule_vfs::path_hash(project.root());
        let project_file = data_dir.join("projects").join(format!("{hash}.json"));
        let recent = std::fs::read(&project_file)
            .ok()
            .and_then(|b| serde_json::from_slice::<ProjectFile>(&b).ok())
            .map(|f| f.recent)
            .unwrap_or_default();
        let project = Arc::new(project);
        let terminals = crate::terminal::project_terminals(terminal_backend, sink.clone());
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
            recent: Mutex::new(recent),
            terminals,
        }
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
        let file = ProjectFile { format: 1, recent };
        let result = serde_json::to_vec(&file)
            .map_err(std::io::Error::other)
            .and_then(|json| {
                if let Some(dir) = self.project_file.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                ferrule_vfs::write_atomic(&self.project_file, &json)
            });
        if let Err(err) = result {
            tracing::warn!(%err, "saving recent files failed");
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
}
