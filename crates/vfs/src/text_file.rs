//! Reading and writing text files.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use ferrule_buffer::{LineEnding, normalize_line_endings};

/// Files larger than this are refused in M1 (large-file paging is M2).
pub const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Errors from file operations.
#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    /// Underlying IO error.
    #[error("{0}")]
    Io(#[from] io::Error),
    /// The file is larger than [`MAX_FILE_SIZE`].
    #[error("file is too large to open ({size} bytes, limit {MAX_FILE_SIZE})")]
    TooLarge {
        /// File size in bytes.
        size: u64,
    },
    /// The file is binary or not valid UTF-8.
    #[error("file is binary or not UTF-8 encoded")]
    Unsupported,
}

/// A decoded text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    /// Text with `\n` line endings.
    pub text: String,
    /// Original line ending style, restored on save.
    pub line_ending: LineEnding,
    /// Whether the file started with a UTF-8 BOM, restored on save.
    pub bom: bool,
}

/// Decodes file bytes. Rejects files containing NUL bytes (treated as binary)
/// and invalid UTF-8.
pub fn decode(bytes: &[u8]) -> Result<TextFile, VfsError> {
    let (bom, body) = match bytes.strip_prefix(UTF8_BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    if body.contains(&0) {
        return Err(VfsError::Unsupported);
    }
    let raw = std::str::from_utf8(body).map_err(|_| VfsError::Unsupported)?;
    let (text, line_ending) = normalize_line_endings(raw);
    Ok(TextFile {
        text,
        line_ending,
        bom,
    })
}

/// Encodes normalized text with the given line ending and BOM.
pub fn encode(text: &str, line_ending: LineEnding, bom: bool) -> Vec<u8> {
    let body = line_ending.apply(text);
    let mut out = Vec::with_capacity(body.len() + if bom { 3 } else { 0 });
    if bom {
        out.extend_from_slice(UTF8_BOM);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// Reads and decodes a text file.
pub fn read_text(path: &Path) -> Result<TextFile, VfsError> {
    let size = fs::metadata(path)?.len();
    if size > MAX_FILE_SIZE {
        return Err(VfsError::TooLarge { size });
    }
    decode(&fs::read(path)?)
}

/// Writes `bytes` to `path` atomically: a temporary file in the same
/// directory is written, flushed to disk and renamed over the target, so a
/// crash leaves either the old or the new content. Permissions of an
/// existing file are preserved.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".ferrule-save-")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    if let Ok(meta) = fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.persist(path).map_err(|err| err.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_bom_and_crlf_and_round_trips() {
        let bytes = b"\xEF\xBB\xBFa\r\nb\r\n";
        let file = decode(bytes).unwrap();
        assert_eq!(file.text, "a\nb\n");
        assert_eq!(file.line_ending, LineEnding::CrLf);
        assert!(file.bom);
        assert_eq!(encode(&file.text, file.line_ending, file.bom), bytes);
    }

    #[test]
    fn rejects_binary_and_invalid_utf8() {
        assert!(matches!(decode(b"ab\0cd"), Err(VfsError::Unsupported)));
        assert!(matches!(decode(b"\xff\xfe"), Err(VfsError::Unsupported)));
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.ts");
        fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("file.ts")]);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        write_atomic(&path, b"#!/bin/sh\necho hi\n").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn read_text_reports_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_text(&dir.path().join("nope")),
            Err(VfsError::Io(_))
        ));
    }
}
