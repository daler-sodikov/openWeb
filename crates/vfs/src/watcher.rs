//! Debounced file watching.
//!
//! Directories are watched one by one (non-recursively), so callers decide
//! which subtrees to skip (e.g. `node_modules`) and never exhaust OS watch
//! limits on large ignored folders. Raw events are collected until the file
//! system has been quiet for the debounce delay, then delivered as one
//! deduplicated batch of paths. Callers re-check each path (exists or not)
//! instead of trusting event kinds, which differ between platforms.
//!
//! [`Watcher::stop`] returns only once the watcher holds nothing open: on
//! Windows notify closes its directory handles on its own thread after the
//! watcher is dropped, and a folder with an open handle cannot always be
//! deleted or renamed.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

#[cfg(windows)]
type Inner = notify::ReadDirectoryChangesWatcher;
#[cfg(not(windows))]
type Inner = notify::RecommendedWatcher;

/// Watches directories and reports changed paths in batches.
pub struct Watcher {
    /// `None` once stopped.
    inner: Option<Inner>,
    watched: HashSet<PathBuf>,
    /// The thread that collects batches and calls `on_change`.
    batches: Option<JoinHandle<()>>,
    /// Windows: notify reports each closed directory handle here, and the
    /// channel disconnects once its thread has closed all of them and ended.
    #[cfg(windows)]
    meta: mpsc::Receiver<notify::windows::MetaEvent>,
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
        let handler = move |result: notify::Result<notify::Event>| match result {
            Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                for path in event.paths {
                    // The receiver only disappears during shutdown.
                    let _ = tx.send(path);
                }
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(%err, "file watcher error"),
        };
        // As notify's own Windows constructor, but keeping the channel that
        // reports closed handles (Watcher::stop waits for it).
        #[cfg(windows)]
        let (inner, meta) = {
            let (meta_tx, meta) = mpsc::channel();
            let handler = std::sync::Arc::new(std::sync::Mutex::new(handler));
            (
                notify::ReadDirectoryChangesWatcher::create(handler, meta_tx)?,
                meta,
            )
        };
        #[cfg(not(windows))]
        let inner = notify::recommended_watcher(handler)?;
        let batches = std::thread::Builder::new()
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
            inner: Some(inner),
            watched: HashSet::new(),
            batches: Some(batches),
            #[cfg(windows)]
            meta,
        })
    }

    /// Stops watching and waits, at most `timeout`, until nothing of the
    /// watcher is left: notify has closed every directory handle (Windows)
    /// and the batch thread has finished (so no `on_change` runs after
    /// this). Returns whether that happened in time.
    pub fn stop(mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        drop(self.inner.take());
        #[cfg(windows)]
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.meta.recv_timeout(left) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
            }
        }
        // The batch thread ends once notify has dropped the event handler
        // (and with it the sender of the batch channel).
        if let Some(thread) = self.batches.take() {
            while !thread.is_finished() {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = thread.join();
        }
        true
    }

    /// Watches one directory (not its subdirectories). Already watched
    /// directories are ignored.
    pub fn watch_dir(&mut self, dir: &Path) -> notify::Result<()> {
        if self.watched.contains(dir) {
            return Ok(());
        }
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        inner.watch(dir, RecursiveMode::NonRecursive)?;
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
            if let Some(inner) = self.inner.as_mut() {
                let _ = inner.unwatch(&d);
            }
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

    #[test]
    fn stop_waits_until_the_watcher_holds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let watched = dir.path().join("watched");
        fs::create_dir_all(watched.join("sub")).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = Watcher::new(Duration::from_millis(50), move |batch| {
            let _ = tx.send(batch);
        })
        .unwrap();
        watcher.watch_dir(&watched).unwrap();
        watcher.watch_dir(&watched.join("sub")).unwrap();
        assert!(watcher.stop(Duration::from_secs(10)));
        // No more batches, and the folders can go (on Windows only once
        // notify has closed its directory handles).
        fs::write(watched.join("late.txt"), "x").unwrap();
        fs::remove_dir_all(&watched).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
