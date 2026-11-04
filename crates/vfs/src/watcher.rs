//! Debounced file watching.
//!
//! Directories are watched one by one (non-recursively), so callers decide
//! which subtrees to skip (e.g. `node_modules`) and never exhaust OS watch
//! limits on large ignored folders. Raw events are collected until the file
//! system has been quiet for the debounce delay, then delivered as one
//! deduplicated batch of paths. Callers re-check each path (exists or not)
//! instead of trusting event kinds, which differ between platforms.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Watches directories and reports changed paths in batches.
pub struct Watcher {
    inner: RecommendedWatcher,
    watched: HashSet<PathBuf>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("watched", &self.watched.len())
            .finish_non_exhaustive()
    }
}

impl Watcher {
    /// Starts a watcher. `on_change` runs on a background thread with each
    /// batch of changed paths; the thread ends when the watcher is dropped.
    pub fn new(
        debounce: Duration,
        on_change: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let inner = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            match result {
                Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                    for path in event.paths {
                        // The receiver only disappears during shutdown.
                        let _ = tx.send(path);
                    }
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(%err, "file watcher error"),
            }
        })?;
        std::thread::Builder::new()
            .name("ferrule-watcher".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut batch = BTreeSet::from([first]);
                    loop {
                        match rx.recv_timeout(debounce) {
                            Ok(path) => {
                                batch.insert(path);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change(batch.into_iter().collect());
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Self {
            inner,
            watched: HashSet::new(),
        })
    }

    /// Watches one directory (not its subdirectories). Already watched
    /// directories are ignored.
    pub fn watch_dir(&mut self, dir: &Path) -> notify::Result<()> {
        if self.watched.contains(dir) {
            return Ok(());
        }
        self.inner.watch(dir, RecursiveMode::NonRecursive)?;
        self.watched.insert(dir.to_path_buf());
        Ok(())
    }

    /// Stops watching directories at or below `dir` (e.g. after removal).
    pub fn forget_below(&mut self, dir: &Path) {
        let gone: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|d| d.starts_with(dir))
            .cloned()
            .collect();
        for d in gone {
            // Removed directories are usually unwatched by the OS already.
            let _ = self.inner.unwatch(&d);
            self.watched.remove(&d);
        }
    }

    /// Number of watched directories.
    pub fn watched_count(&self) -> usize {
        self.watched.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reports_changes_in_watched_dirs_only() {
        let dir = tempfile::tempdir().unwrap();
        let watched = dir.path().join("watched");
        let other = dir.path().join("other");
        fs::create_dir_all(&watched).unwrap();
        fs::create_dir_all(&other).unwrap();

        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(Duration::from_millis(50), move |batch| {
            let _ = tx.send(batch);
        })
        .unwrap();
        watcher.watch_dir(&watched).unwrap();
        watcher.watch_dir(&watched).unwrap();
        assert_eq!(watcher.watched_count(), 1);

        fs::write(other.join("ignored.txt"), "x").unwrap();
        fs::write(watched.join("a.txt"), "1").unwrap();
        fs::write(watched.join("a.txt"), "2").unwrap();
        let batch = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no change reported");
        assert!(batch.iter().any(|p| p.ends_with("a.txt")), "{batch:?}");
        assert!(
            !batch.iter().any(|p| p.ends_with("ignored.txt")),
            "{batch:?}"
        );

        watcher.forget_below(&watched);
        assert_eq!(watcher.watched_count(), 0);
    }
}
