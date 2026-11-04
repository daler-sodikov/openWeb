//! `file:` URIs ↔ paths, tolerant of the forms servers send (Windows drive
//! letters in either case, `:` percent-encoded as `%3A`).

use std::path::{Path, PathBuf};

use lsp_types::Uri;

/// The `file:` URI of an absolute path.
pub fn path_to_uri(path: &Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    url.as_str().parse().ok()
}

/// The path of a `file:` URI; `None` for other schemes.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let text = uri.as_str();
    if !text.starts_with("file:") {
        return None;
    }
    let url = url::Url::parse(&decode_drive_colon(text)).ok()?;
    let path = url.to_file_path().ok()?;
    Some(normalize_drive(path))
}

/// `file:///c%3A/x` → `file:///c:/x` (VS Code style URIs).
fn decode_drive_colon(text: &str) -> String {
    let Some(rest) = text.strip_prefix("file:///") else {
        return text.to_owned();
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_alphabetic()
        && rest.is_char_boundary(4)
        && rest[1..4].eq_ignore_ascii_case("%3a")
    {
        format!("file:///{}:{}", &rest[..1], &rest[4..])
    } else {
        text.to_owned()
    }
}

/// Upper-cases a Windows drive letter so paths compare equal to ours.
fn normalize_drive(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        let text = path.to_string_lossy();
        let bytes = text.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_lowercase() {
            return PathBuf::from(format!("{}{}", text[..1].to_ascii_uppercase(), &text[1..]));
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_paths_with_spaces_and_unicode() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\Users\a b\プロジェクト\x.ts")
        } else {
            PathBuf::from("/home/a b/プロジェクト/x.ts")
        };
        let uri = path_to_uri(&path).unwrap();
        assert!(uri.as_str().starts_with("file:///"));
        assert!(uri.as_str().contains("a%20b"));
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn accepts_encoded_drive_colons() {
        assert_eq!(
            decode_drive_colon("file:///c%3A/x/y.ts"),
            "file:///c:/x/y.ts"
        );
        assert_eq!(decode_drive_colon("file:///C%3a/x"), "file:///C:/x");
        assert_eq!(decode_drive_colon("file:///home/x"), "file:///home/x");
    }

    #[cfg(windows)]
    #[test]
    fn normalizes_drive_letter_case() {
        let uri: Uri = "file:///c%3A/Users/x.ts".parse().unwrap();
        assert_eq!(uri_to_path(&uri), Some(PathBuf::from(r"C:\Users\x.ts")));
    }

    #[test]
    fn rejects_other_schemes() {
        let uri: Uri = "untitled:Untitled-1".parse().unwrap();
        assert_eq!(uri_to_path(&uri), None);
    }
}
