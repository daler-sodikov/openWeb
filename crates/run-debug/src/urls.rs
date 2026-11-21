//! `file://` URLs and paths, and how paths compare on each OS
//! (`docs/ARCHITECTURE.md`, 4.10.3). Scripts reach the debugger as URLs;
//! breakpoints are paths. Everything works on strings, so the Windows rules
//! can be tested on any OS. Paths come out with `/` separators.

use crate::paths;

/// The OS whose file system rules apply when comparing paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// Linux: names are exact.
    Linux,
    /// macOS: case-insensitive.
    Mac,
    /// Windows: case-insensitive, `\` is `/`.
    Windows,
}

impl Host {
    /// The OS this binary runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

/// The path of a `file://` URL, decoded, with `/` separators: `file:///a/b`
/// is `/a/b`, `file:///C:/x/a.js` is `C:/x/a.js`, `file://server/share/a`
/// is `//server/share/a`. `None` if it is not a `file:` URL. A query or
/// fragment is dropped.
pub fn file_url_to_path(url: &str) -> Option<String> {
    let scheme = url.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = url.get(7..)?;
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    Some(paths::normalize(&format!("file://{}", &rest[..end])))
}

/// The `file://` URL of an absolute path (any OS's), percent-encoded the way
/// Node's `pathToFileURL` does for the characters that matter (spaces, `%`,
/// `#`, `?`, non-ASCII). `None` for a relative path.
pub fn path_to_file_url(path: &str) -> Option<String> {
    let path = paths::normalize(path);
    if !paths::is_absolute(&path) {
        return None;
    }
    let encoded = encode(&path);
    Some(if path.starts_with("//") {
        format!("file:{encoded}") // UNC: file://server/share/a
    } else if path.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}") // drive letter: file:///C:/x
    })
}

fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Whether two files (each a path or a `file://` URL) are the same name on
/// `host`: exact on Linux, without case on macOS and Windows (where `\` is
/// `/` too, and a drive letter's case never matters).
pub fn same_file(a: &str, b: &str, host: Host) -> bool {
    let (a, b) = (comparable(a), comparable(b));
    match host {
        Host::Linux => a == b,
        Host::Mac | Host::Windows => a.to_lowercase() == b.to_lowercase(),
    }
}

fn comparable(text: &str) -> String {
    let path = file_url_to_path(text).unwrap_or_else(|| paths::normalize(text));
    path.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_urls_and_paths() {
        assert_eq!(
            file_url_to_path("file:///home/me/app.js").as_deref(),
            Some("/home/me/app.js")
        );
        assert_eq!(
            file_url_to_path("file:///home/my%20dir/%C3%A9.js").as_deref(),
            Some("/home/my dir/é.js")
        );
        assert_eq!(
            file_url_to_path("file:///a/b.js?x=1#top").as_deref(),
            Some("/a/b.js")
        );
        assert_eq!(
            file_url_to_path("FILE:///a/b.js").as_deref(),
            Some("/a/b.js")
        );
        assert_eq!(
            path_to_file_url("/home/my dir/é#1.js").as_deref(),
            Some("file:///home/my%20dir/%C3%A9%231.js")
        );
    }

    #[test]
    fn windows_urls_and_paths() {
        assert_eq!(
            file_url_to_path("file:///C:/Users/me/app.js").as_deref(),
            Some("C:/Users/me/app.js")
        );
        assert_eq!(
            file_url_to_path("file:///c:/My%20Dir/a.js").as_deref(),
            Some("c:/My Dir/a.js")
        );
        assert_eq!(
            file_url_to_path("file://server/share/a.js").as_deref(),
            Some("//server/share/a.js")
        );
        assert_eq!(
            path_to_file_url(r"C:\Users\me\my app.js").as_deref(),
            Some("file:///C:/Users/me/my%20app.js")
        );
        assert_eq!(
            path_to_file_url("//server/share/a.js").as_deref(),
            Some("file://server/share/a.js")
        );
    }

    #[test]
    fn not_file_urls_and_relative_paths_are_refused() {
        for url in ["http://h/a.js", "node:fs", "data:,x", "", "file:/a", "fi"] {
            assert_eq!(file_url_to_path(url), None, "{url}");
        }
        assert_eq!(path_to_file_url("src/a.js"), None);
        assert_eq!(path_to_file_url(""), None);
    }

    #[test]
    fn urls_round_trip() {
        for path in ["/a/b c/é.js", "C:/x/y#z.js", "//srv/share/a%.js", "/"] {
            let url = path_to_file_url(path).unwrap();
            assert_eq!(file_url_to_path(&url).as_deref(), Some(path), "{url}");
        }
    }

    #[test]
    fn comparison_follows_the_host() {
        // The same file, written by three tools.
        let (a, b, c) = (
            "C:/Users/Me/App.js",
            r"c:\users\me\app.js",
            "file:///C:/Users/Me/App.js",
        );
        assert!(same_file(a, b, Host::Windows));
        assert!(same_file(a, c, Host::Windows));
        assert!(same_file(b, c, Host::Windows));
        assert!(!same_file(a, b, Host::Linux));
        assert!(same_file(a, c, Host::Linux));
        assert!(same_file("/Users/me/A.ts", "/users/me/a.ts", Host::Mac));
        assert!(!same_file("/Users/me/A.ts", "/users/me/a.ts", Host::Linux));
        assert!(!same_file("/a/b.ts", "/a/c.ts", Host::Windows));
        assert!(same_file("/a/b/", "/a/b", Host::Linux));
    }

    #[test]
    fn the_current_host_is_one_of_the_three() {
        let host = Host::current();
        assert_eq!(host == Host::Windows, cfg!(windows));
        assert_eq!(host == Host::Mac, cfg!(target_os = "macos"));
    }
}
