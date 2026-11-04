//! Crash-recovery files for unsaved documents.
//!
//! One JSON file per document in the recovery directory, named after a stable
//! 64-bit FNV-1a hash of the document's absolute path:
//! `{ "format": 1, "path": "...", "savedAtMs": 1762243200000, "text": "..." }`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::text_file::write_atomic;

const FORMAT: u32 = 1;

/// A recovered unsaved document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    /// File format version.
    pub format: u32,
    /// Absolute path of the document.
    pub path: PathBuf,
    /// When the entry was written (Unix epoch milliseconds).
    pub saved_at_ms: u64,
    /// Unsaved text (`\n` line endings).
    pub text: String,
}

/// Directory of recovery files.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    dir: PathBuf,
}

impl RecoveryStore {
    /// Uses `dir` for recovery files (created on first save).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Writes (atomically) the unsaved text of the document at `path`.
    pub fn save(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let entry = RecoveryEntry {
            format: FORMAT,
            path: path.to_path_buf(),
            saved_at_ms: now_ms(),
            text: text.to_owned(),
        };
        let json = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        write_atomic(&self.file_for(path), &json)
    }

    /// Loads the entry for `path`, if any. Unreadable or foreign entries are
    /// ignored (and logged) rather than failing the open.
    pub fn load(&self, path: &Path) -> Option<RecoveryEntry> {
        let file = self.file_for(path);
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
            Err(err) => {
                tracing::warn!(?file, %err, "cannot read recovery file");
                return None;
            }
        };
        match serde_json::from_slice::<RecoveryEntry>(&bytes) {
            Ok(entry) if entry.format == FORMAT && entry.path == path => Some(entry),
            Ok(_) => None,
            Err(err) => {
                tracing::warn!(?file, %err, "corrupt recovery file ignored");
                None
            }
        }
    }

    /// Deletes the entry for `path`, if any.
    pub fn remove(&self, path: &Path) -> io::Result<()> {
        match fs::remove_file(self.file_for(path)) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn file_for(&self, path: &Path) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.json",
            fnv1a64(path.to_string_lossy().as_bytes())
        ))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(dir.path().join("recovery"));
        let doc = Path::new("/project/src/a.ts");
        assert_eq!(store.load(doc), None);
        store.save(doc, "unsaved ✓").unwrap();
        let entry = store.load(doc).unwrap();
        assert_eq!(entry.text, "unsaved ✓");
        assert_eq!(entry.path, doc);
        assert!(entry.saved_at_ms > 0);
        // A different document does not see it.
        assert_eq!(store.load(Path::new("/project/src/b.ts")), None);
        store.remove(doc).unwrap();
        assert_eq!(store.load(doc), None);
        store.remove(doc).unwrap();
    }

    #[test]
    fn corrupt_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = RecoveryStore::new(dir.path());
        let doc = Path::new("/p/x.ts");
        store.save(doc, "x").unwrap();
        fs::write(store.file_for(doc), "{not json").unwrap();
        assert_eq!(store.load(doc), None);
    }

    #[test]
    fn hash_is_stable() {
        // Recovery file names must not change between releases.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
