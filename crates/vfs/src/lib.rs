//! File system access for documents: decoding, atomic saves and crash
//! recovery files. Functions here are synchronous; async callers run them on
//! `spawn_blocking` (see `docs/ARCHITECTURE.md`, section 6).
//!
//! Text files are UTF-8 (with or without BOM); other encodings are M2b
//! (`docs/ROADMAP.md`).

mod recovery;
mod text_file;
mod watcher;

pub use recovery::{RecoveryEntry, RecoveryStore, path_hash};
pub use text_file::{MAX_FILE_SIZE, TextFile, VfsError, decode, encode, read_text, write_atomic};
pub use watcher::Watcher;
