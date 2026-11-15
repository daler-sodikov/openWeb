//! Paths as data: what test tools write in their reports (and what the view
//! sends back) is a string from some OS, not a path of the host. Report
//! paths are kept with `/` separators and compared as strings; they become
//! native paths only where a file is opened.
//!
//! What the tools write (read in their sources, Vitest 5.1, Playwright
//! 1.64): Playwright's `config.rootDir` and spec `file` go through
//! `toPosixPath` (`C:/x/e2e` and `math.spec.ts` on Windows); Vitest's
//! file `name` is its module path (forward slashes); stack frames are
//! Node's native text (`C:\x\a.ts:1:2`) or `file://` URLs. Jest is not
//! installed here; its paths are native, which this handles too.

use std::path::{Component, Path, PathBuf};

/// A path with `/` separators: `\` becomes `/`, `file://` URLs are decoded
/// and Windows' `\\?\` prefixes are dropped.
pub fn normalize(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        return match decoded.strip_prefix('/') {
            // file:///C:/x -> C:/x; file:///home/x -> /home/x.
            Some(local) if has_drive(local) => local.replace('\\', "/"),
            Some(_) => decoded.replace('\\', "/"),
            // file://server/share/x -> //server/share/x (UNC).
            None => format!("//{}", decoded.replace('\\', "/")),
        };
    }
    let slashed = path.replace('\\', "/");
    if let Some(unc) = slashed.strip_prefix("//?/UNC/") {
        return format!("//{unc}");
    }
    match slashed.strip_prefix("//?/") {
        Some(verbatim) => verbatim.to_owned(),
        None => slashed,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| bytes.get(at).and_then(|&h| char::from(h).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `C:` at the start.
fn has_drive(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    )
}

/// A Windows path (drive letter or UNC): compared without case.
fn is_windows(path: &str) -> bool {
    has_drive(path) || path.starts_with("//")
}

/// Whether a normalized path is absolute on any OS: `/x`, `C:/x`,
/// `//server/share`.
pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || (has_drive(path) && path.get(2..3) == Some("/"))
}

/// `base` and a relative path, joined with `/`.
pub fn join(base: &str, relative: &str) -> String {
    let base = base.trim_end_matches('/');
    let relative = relative.trim_start_matches("./");
    if base.is_empty() {
        relative.to_owned()
    } else {
        format!("{base}/{relative}")
    }
}

/// `file` relative to the folder `dir`, with `/`, if it is inside it.
/// Windows paths compare without case (drive letters differ in case
/// between tools).
pub fn relative_to(file: &str, dir: &Path) -> Option<String> {
    let file = normalize(file);
    let dir = normalize(&dir.to_string_lossy());
    let dir = dir.trim_end_matches('/');
    let head = file.get(..dir.len())?;
    let same = if is_windows(dir) {
        head.eq_ignore_ascii_case(dir)
    } else {
        head == dir
    };
    if !same {
        return None;
    }
    file.get(dir.len()..)?
        .strip_prefix('/')
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned)
}

/// A path relative to a folder, as sent by the view: normalized, or `None`
/// if it is absolute (on any OS) or leaves the folder (`..`).
pub fn safe_relative(path: &str) -> Option<String> {
    let path = normalize(path);
    if path.is_empty() || is_absolute(&path) || has_drive(&path) {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() || parts.contains(&"..") {
        return None;
    }
    Some(parts.join("/"))
}

/// `base` joined with a package-relative path such as a `bin` entry
/// (`./cli.js`, `bin/jest.js`): one native component per segment, `.`
/// dropped.
pub fn join_native(base: &Path, relative: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => out.push(Component::ParentDir.as_os_str()),
            name => out.push(name),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_paths_from_any_os_use_slashes() {
        assert_eq!(normalize("/work/p/a.ts"), "/work/p/a.ts");
        assert_eq!(normalize(r"C:\work\p\a.ts"), "C:/work/p/a.ts");
        assert_eq!(normalize("C:/work/p/a.ts"), "C:/work/p/a.ts");
        assert_eq!(normalize(r"\\server\share\p\a.ts"), "//server/share/p/a.ts");
        assert_eq!(normalize(r"\\?\C:\work\p"), "C:/work/p");
        assert_eq!(normalize(r"\\?\UNC\server\share\p"), "//server/share/p");
        assert_eq!(normalize("file:///home/me/a%20b.ts"), "/home/me/a b.ts");
        assert_eq!(normalize("file:///C:/work/p/a.ts"), "C:/work/p/a.ts");
        assert_eq!(normalize("file://server/share/a.ts"), "//server/share/a.ts");
    }

    #[test]
    fn absolute_on_any_os() {
        for path in ["/x", "C:/x", "c:/x", "//server/share/x"] {
            assert!(is_absolute(path), "{path}");
        }
        for path in ["x/y", "./x", "C:x", ""] {
            assert!(!is_absolute(path), "{path}");
        }
        assert_eq!(join("C:/p/e2e/", "./math.spec.ts"), "C:/p/e2e/math.spec.ts");
        assert_eq!(join("/p/e2e", "sub/a.spec.ts"), "/p/e2e/sub/a.spec.ts");
    }

    #[test]
    fn relative_to_a_folder_on_any_os() {
        let unix = Path::new("/work/p");
        assert_eq!(
            relative_to("/work/p/src/a.ts", unix).as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            relative_to("/work/pp/a.ts", unix),
            None,
            "not a prefix match"
        );
        assert_eq!(
            relative_to("/WORK/p/a.ts", unix),
            None,
            "Unix paths keep case"
        );
        assert_eq!(relative_to("/work/p", unix), None, "the folder itself");
        // Windows folders, as the host would print them.
        let win = Path::new(r"C:\work\p");
        assert_eq!(
            relative_to(r"C:\work\p\src\a.ts", win).as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            relative_to("c:/work/p/src/a.ts", win).as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            relative_to("file:///C:/work/p/a.ts", win).as_deref(),
            Some("a.ts")
        );
        assert_eq!(relative_to(r"D:\work\p\a.ts", win), None);
        let verbatim = Path::new(r"\\?\C:\work\p");
        assert_eq!(
            relative_to(r"C:\work\p\a.ts", verbatim).as_deref(),
            Some("a.ts")
        );
        let unc = Path::new(r"\\server\share\p");
        assert_eq!(
            relative_to(r"\\SERVER\share\p\x\a.ts", unc).as_deref(),
            Some("x/a.ts")
        );
    }

    #[test]
    fn paths_from_the_view_stay_inside() {
        assert_eq!(
            safe_relative("src/a.test.ts").as_deref(),
            Some("src/a.test.ts")
        );
        assert_eq!(
            safe_relative(r"src\a.test.ts").as_deref(),
            Some("src/a.test.ts")
        );
        assert_eq!(safe_relative("./src//a.ts").as_deref(), Some("src/a.ts"));
        for bad in [
            "../x.ts",
            r"..\x.ts",
            "src/../../x.ts",
            "/etc/passwd",
            r"C:\x.ts",
            "C:x.ts",
            r"\\server\share\x.ts",
            "",
            ".",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn bin_entries_join_per_segment() {
        let base = Path::new("node_modules").join("vitest");
        assert_eq!(join_native(&base, "./vitest.mjs"), base.join("vitest.mjs"));
        assert_eq!(
            join_native(&base, "bin/jest.js"),
            base.join("bin").join("jest.js")
        );
        assert_eq!(
            join_native(&base, r".\bin\cli.js"),
            base.join("bin").join("cli.js")
        );
    }
}
