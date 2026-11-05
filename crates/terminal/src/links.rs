//! File links in terminal output: `path:line[:col]`, `path(line,col)` (tsc)
//! and `file://` URLs. Columns are character indices in a row's text, which
//! has one character per cell.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// A file reference found in a line of output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMatch {
    /// First character (cell) of the link.
    pub start: usize,
    /// One past the last character.
    pub end: usize,
    /// The path as written (a `file://` URL is decoded to a path).
    pub path: String,
    /// One-based line, if given.
    pub line: Option<u32>,
    /// One-based column, if given.
    pub column: Option<u32>,
}

/// A link resolved to an existing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLink {
    /// Absolute path.
    pub path: PathBuf,
    /// One-based line, if given.
    pub line: Option<u32>,
    /// One-based column, if given.
    pub column: Option<u32>,
}

// A path: an optional `file://`, an optional drive letter, then path
// characters containing a slash, a backslash or a dot (so plain words are
// not links). Suffixes: `:12`, `:12:5`, `(12,5)` or `(12)`.
static LINK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<path>
            (?:file://)?
            (?:[A-Za-z]:)?
            [\w.~@+\-/\\%]*[/\\.][\w.~@+\-/\\%]*
        )
        (?:
            :(?P<l1>\d+)(?::(?P<c1>\d+))?
          | \((?P<l2>\d+)(?:,\s?(?P<c2>\d+))?\)
        )?",
    )
    .ok()
});

fn number(caps: &regex::Captures<'_>, a: &str, b: &str) -> Option<u32> {
    caps.name(a)
        .or_else(|| caps.name(b))
        .and_then(|m| m.as_str().parse().ok())
}

/// `%20` and friends in `file://` URLs.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every candidate link in `line`, left to right.
pub fn find_links(line: &str) -> Vec<LinkMatch> {
    let Some(re) = LINK.as_ref() else {
        return Vec::new();
    };
    let char_index = |byte: usize| line[..byte].chars().count();
    re.captures_iter(line)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            let raw = caps.name("path")?.as_str();
            let has_suffix = caps.name("l1").is_some() || caps.name("l2").is_some();
            // Trailing punctuation belongs to the sentence, not the path.
            let trimmed = if has_suffix {
                raw
            } else {
                raw.trim_end_matches(['.', ',', ';', ':'])
            };
            if trimmed.is_empty() || !trimmed.contains(['/', '\\', '.']) {
                return None;
            }
            let end_byte = if has_suffix {
                whole.end()
            } else {
                whole.start() + trimmed.len()
            };
            let path = match trimmed.strip_prefix("file://") {
                Some(url) => percent_decode(url),
                None => trimmed.to_owned(),
            };
            Some(LinkMatch {
                start: char_index(whole.start()),
                end: char_index(end_byte),
                path,
                line: number(&caps, "l1", "l2"),
                column: number(&caps, "c1", "c2"),
            })
        })
        .collect()
}

/// The link covering character `col` of `line`, if any.
pub fn link_at(line: &str, col: usize) -> Option<LinkMatch> {
    find_links(line)
        .into_iter()
        .find(|link| col >= link.start && col < link.end)
}

/// Resolves a link against `bases` (tried in order; absolute paths are
/// used as they are). Only existing files resolve.
pub fn resolve(link: &LinkMatch, bases: &[&Path]) -> Option<ResolvedLink> {
    let path = Path::new(&link.path);
    let candidates: Vec<PathBuf> = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        bases.iter().map(|base| base.join(path)).collect()
    };
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|found| ResolvedLink {
            path: found,
            line: link.line,
            column: link.column,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> LinkMatch {
        let links = find_links(line);
        assert_eq!(links.len(), 1, "{line:?}: {links:?}");
        links.into_iter().next().unwrap()
    }

    #[test]
    fn line_and_column_suffixes() {
        let link = one("error in src/a.ts:12:5 here");
        assert_eq!(
            (link.path.as_str(), link.line, link.column),
            ("src/a.ts", Some(12), Some(5))
        );
        assert_eq!((link.start, link.end), (9, 22));
        let link = one("./lib/x.rs:7");
        assert_eq!((link.line, link.column), (Some(7), None));
        let tsc = one("src/App.tsx(14,3): error TS2322");
        assert_eq!(
            (tsc.path.as_str(), tsc.line, tsc.column),
            ("src/App.tsx", Some(14), Some(3))
        );
    }

    #[test]
    fn absolute_windows_and_url_paths() {
        assert_eq!(one("at /home/me/p/a.js:3:1").path, "/home/me/p/a.js");
        let win = one(r"C:\work\app\main.ts:4:2 failed");
        assert_eq!(
            (win.path.as_str(), win.line, win.column),
            (r"C:\work\app\main.ts", Some(4), Some(2))
        );
        let url = one("see file:///tmp/my%20dir/a.ts:9");
        assert_eq!((url.path.as_str(), url.line), ("/tmp/my dir/a.ts", Some(9)));
    }

    #[test]
    fn columns_count_characters_and_plain_words_are_not_links() {
        let link = one("漢字 src/b.ts:2");
        assert_eq!(link.start, 3);
        assert!(find_links("hello world, nothing here").is_empty());
        assert_eq!(one("done: README.md.").path, "README.md");
        let at = link_at("x src/a.ts:1 y", 4).unwrap();
        assert_eq!(at.path, "src/a.ts");
        assert!(link_at("x src/a.ts:1 y", 0).is_none());
    }

    #[test]
    fn resolves_only_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.ts"), "x").unwrap();
        let link = one("src/a.ts:3:1");
        let other = tempfile::tempdir().unwrap();
        let resolved = resolve(&link, &[other.path(), dir.path()]).unwrap();
        assert_eq!(resolved.path, dir.path().join("src/a.ts"));
        assert_eq!((resolved.line, resolved.column), (Some(3), Some(1)));
        assert!(resolve(&one("src/missing.ts:1"), &[dir.path()]).is_none());
    }
}
