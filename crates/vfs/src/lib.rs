//! File system access for documents: decoding, atomic saves and crash
//! recovery files. Functions here are synchronous; async callers run them on
//! `spawn_blocking` (see `docs/ARCHITECTURE.md`, section 6).
//!
//! M1 scope: UTF-8 text (with or without BOM). Other encodings and file
//! watching arrive in M2 (`docs/ROADMAP.md`).

mod recovery;
mod text_file;

pub use recovery::{RecoveryEntry, RecoveryStore};
pub use text_file::{MAX_FILE_SIZE, TextFile, VfsError, decode, encode, read_text, write_atomic};
