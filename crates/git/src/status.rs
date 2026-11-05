//! `git status --porcelain=v2 -z --branch --untracked-files=all
//! --ignored=matching`, parsed.

use crate::error::GitError;

/// A change in the index (against HEAD) or the worktree (against the index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Content or mode changed.
    Modified,
    /// New.
    Added,
    /// Removed.
    Deleted,
    /// Renamed (index only).
    Renamed,
    /// Copied (index only).
    Copied,
    /// File type changed (e.g. file to symlink).
    TypeChanged,
}

/// How a file is shown (tree colors, commit window).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    /// Merge conflict.
    Conflicted,
    /// Changed.
    Modified,
    /// New and tracked (staged).
    Added,
    /// Deleted.
    Deleted,
    /// Renamed or copied.
    Renamed,
    /// Not tracked.
    Untracked,
    /// Ignored by `.gitignore` (a directory entry covers everything below).
    Ignored,
}

/// One entry of the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Repository-relative path with `/` (ignored directories end without
    /// the slash).
    pub path: String,
    /// The path before a rename or copy.
    pub orig_path: Option<String>,
    /// Change staged in the index.
    pub index: Option<Change>,
    /// Change in the worktree, not staged.
    pub worktree: Option<Change>,
    /// The derived kind.
    pub kind: StatusKind,
    /// Whether this is an ignored directory (porcelain `dir/`).
    pub is_dir: bool,
}

/// The current branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchInfo {
    /// Branch name; `None` for a detached HEAD.
    pub head: Option<String>,
    /// Commit id of HEAD; `None` before the first commit.
    pub oid: Option<String>,
    /// Upstream (`origin/main`).
    pub upstream: Option<String>,
    /// Commits ahead of the upstream.
    pub ahead: u32,
    /// Commits behind the upstream.
    pub behind: u32,
}

/// The parsed status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// Branch information.
    pub branch: BranchInfo,
    /// Changed, untracked and ignored entries, in git's order.
    pub files: Vec<FileStatus>,
}

fn change(c: u8) -> Option<Change> {
    match c {
        b'M' => Some(Change::Modified),
        b'A' => Some(Change::Added),
        b'D' => Some(Change::Deleted),
        b'R' => Some(Change::Renamed),
        b'C' => Some(Change::Copied),
        b'T' => Some(Change::TypeChanged),
        _ => None,
    }
}

fn kind_of(index: Option<Change>, worktree: Option<Change>) -> StatusKind {
    match (index, worktree) {
        (_, Some(Change::Deleted)) | (Some(Change::Deleted), _) => StatusKind::Deleted,
        (Some(Change::Renamed | Change::Copied), _) => StatusKind::Renamed,
        (Some(Change::Added), _) => StatusKind::Added,
        _ => StatusKind::Modified,
    }
}

/// Parses NUL-separated porcelain v2 output.
pub fn parse_status(output: &[u8]) -> Result<RepoStatus, GitError> {
    let text = String::from_utf8_lossy(output);
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    let mut status = RepoStatus::default();
    let bad = |record: &str| GitError::Parse(format!("status record {record:?}"));
    while let Some(record) = records.next() {
        let (tag, rest) = record.split_once(' ').ok_or_else(|| bad(record))?;
        match tag {
            "#" => {
                let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
                let b = &mut status.branch;
                match key {
                    "branch.oid" => b.oid = (value != "(initial)").then(|| value.to_owned()),
                    "branch.head" => b.head = (value != "(detached)").then(|| value.to_owned()),
                    "branch.upstream" => b.upstream = Some(value.to_owned()),
                    "branch.ab" => {
                        for part in value.split(' ') {
                            if let Some(n) = part.strip_prefix('+') {
                                b.ahead = n.parse().unwrap_or(0);
                            } else if let Some(n) = part.strip_prefix('-') {
                                b.behind = n.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "1" | "2" => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
                let fields = if tag == "1" { 8 } else { 9 };
                let parts: Vec<&str> = rest.splitn(fields, ' ').collect();
                let (Some(xy), Some(path)) = (parts.first(), parts.get(fields - 1)) else {
                    return Err(bad(record));
                };
                let xy = xy.as_bytes();
                let index = change(xy.first().copied().unwrap_or(b'.'));
                let worktree = change(xy.get(1).copied().unwrap_or(b'.'));
                let orig_path = if tag == "2" {
                    Some(records.next().ok_or_else(|| bad(record))?.to_owned())
                } else {
                    None
                };
                status.files.push(FileStatus {
                    path: (*path).to_owned(),
                    orig_path,
                    index,
                    worktree,
                    kind: kind_of(index, worktree),
                    is_dir: false,
                });
            }
            "u" => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let path = rest.splitn(10, ' ').nth(9).ok_or_else(|| bad(record))?;
                status.files.push(FileStatus {
                    path: path.to_owned(),
                    orig_path: None,
                    index: None,
                    worktree: None,
                    kind: StatusKind::Conflicted,
                    is_dir: false,
                });
            }
            "?" | "!" => {
                let is_dir = rest.ends_with('/');
                status.files.push(FileStatus {
                    path: rest.trim_end_matches('/').to_owned(),
                    orig_path: None,
                    index: None,
                    worktree: None,
                    kind: if tag == "?" {
                        StatusKind::Untracked
                    } else {
                        StatusKind::Ignored
                    },
                    is_dir,
                });
            }
            _ => return Err(bad(record)),
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_record_type() {
        let out = [
            "# branch.oid 1234abcd",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 .M N... 100644 100644 100644 aaa bbb src/a b.ts",
            "1 A. N... 000000 100644 100644 000 ccc new.ts",
            "1 D. N... 100644 000000 000000 ddd 000 gone.ts",
            "2 R. N... 100644 100644 100644 eee eee R100 to.ts",
            "from.ts",
            "u UU N... 100644 100644 100644 100644 a b c both.ts",
            "? notes/todo.md",
            "! node_modules/",
            "! debug.log",
            "",
        ]
        .join("\0");
        let s = parse_status(out.as_bytes()).unwrap();
        assert_eq!(s.branch.head.as_deref(), Some("main"));
        assert_eq!(s.branch.oid.as_deref(), Some("1234abcd"));
        assert_eq!(s.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.branch.ahead, s.branch.behind), (2, 1));
        let summary: Vec<(&str, StatusKind)> =
            s.files.iter().map(|f| (f.path.as_str(), f.kind)).collect();
        assert_eq!(
            summary,
            [
                ("src/a b.ts", StatusKind::Modified),
                ("new.ts", StatusKind::Added),
                ("gone.ts", StatusKind::Deleted),
                ("to.ts", StatusKind::Renamed),
                ("both.ts", StatusKind::Conflicted),
                ("notes/todo.md", StatusKind::Untracked),
                ("node_modules", StatusKind::Ignored),
                ("debug.log", StatusKind::Ignored),
            ]
        );
        assert_eq!(s.files[0].index, None);
        assert_eq!(s.files[0].worktree, Some(Change::Modified));
        assert_eq!(s.files[3].orig_path.as_deref(), Some("from.ts"));
        assert!(s.files[6].is_dir);
    }

    #[test]
    fn initial_and_detached() {
        let s = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0").unwrap();
        assert_eq!(s.branch.oid, None);
        assert_eq!(s.branch.head, None);
        assert!(s.files.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_status(b"x y z\0").is_err());
    }
}
