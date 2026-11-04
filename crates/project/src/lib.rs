//! Project model: the opened folder, project-relative paths and lazy
//! directory listings.
//!
//! Paths crossing the IPC boundary are project-relative, use `/` as the
//! separator on every OS, and never contain `.`/`..` components. The empty
//! string is the project root.
//!
//! One root folder; `.git` is hidden, everything else is listed. Entries
//! excluded by `.gitignore` files (at any level, the deepest decides) or
//! inside `node_modules` are listed but marked as ignored.
//! [`Project::detect_workspace`] finds `tsconfig`/`jsconfig` files and
//! npm/yarn/pnpm/Bun workspace packages.

mod workspace;

pub use workspace::{PackageManager, Workspace};

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Directory names never shown in the tree.
const HIDDEN_DIRS: &[&str] = &[".git"];
/// Directory names always treated as ignored.
const IGNORED_DIRS: &[&str] = &["node_modules"];

/// Errors from project operations.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// Underlying IO error.
    #[error("{0}")]
    Io(#[from] io::Error),
    /// The path to open is not a directory.
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    /// A relative path was malformed or tried to leave the project root.
    #[error("invalid project path: {0:?}")]
    InvalidPath(String),
}

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A directory (symlinks to directories included).
    Directory,
    /// Anything else.
    File,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// File name.
    pub name: String,
    /// Project-relative path with `/` separators.
    pub path: String,
    /// Directory or file.
    pub kind: EntryKind,
    /// Excluded by `.gitignore` (or inside `node_modules`).
    pub ignored: bool,
}

/// An opened project folder.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    name: String,
}

impl Project {
    /// Opens the folder at `path` (canonicalized).
    pub fn open(path: &Path) -> Result<Self, ProjectError> {
        let root = dunce_canonicalize(path)?;
        if !root.is_dir() {
            return Err(ProjectError::NotADirectory(root));
        }
        let name = root.file_name().map_or_else(
            || root.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        Ok(Self { root, name })
    }

    /// Absolute root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Display name (the folder name).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Resolves a project-relative path to an absolute path, rejecting
    /// anything that could escape the root.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, ProjectError> {
        let mut path = self.root.clone();
        if rel.is_empty() {
            return Ok(path);
        }
        for part in rel.split('/') {
            let valid = !part.is_empty()
                && part != "."
                && part != ".."
                && !part.contains(['\\', ':', '\0'])
                && matches!(
                    Path::new(part).components().next(),
                    Some(Component::Normal(_))
                );
            if !valid {
                return Err(ProjectError::InvalidPath(rel.to_owned()));
            }
            path.push(part);
        }
        Ok(path)
    }

    /// Converts an absolute path inside the project to a project-relative
    /// path, if it is inside.
    pub fn relativize(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = rel
            .components()
            .map(|c| match c {
                Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        Some(parts.join("/"))
    }

    /// Lists a directory: directories first, then files, each sorted
    /// case-insensitively. Hidden directories (`.git`) are skipped.
    pub fn list_dir(&self, rel: &str) -> Result<Vec<Entry>, ProjectError> {
        let dir = self.resolve(rel)?;
        let ignores = IgnoreChain::new(&self.root, &dir);
        let dir_ignored = ignores.ancestors_ignored(&self.root, &dir);
        let mut entries = Vec::new();
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            // `metadata` follows symlinks, so a link to a folder lists as a folder.
            let is_dir = fs::metadata(item.path())
                .map(|m| m.is_dir())
                .unwrap_or(false);
            if is_dir && HIDDEN_DIRS.contains(&name.as_str()) {
                continue;
            }
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let ignored = dir_ignored
                || (is_dir && IGNORED_DIRS.contains(&name.as_str()))
                || ignores.is_ignored(&item.path(), is_dir);
            entries.push(Entry {
                name,
                path,
                kind: if is_dir {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                ignored,
            });
        }
        entries.sort_by(compare_entries);
        Ok(entries)
    }
}

/// The `.gitignore` files from the project root down to one directory.
struct IgnoreChain {
    /// (directory, its `.gitignore`), root first.
    levels: Vec<ignore::gitignore::Gitignore>,
}

impl IgnoreChain {
    fn new(root: &Path, dir: &Path) -> Self {
        let mut dirs: Vec<PathBuf> = dir
            .ancestors()
            .take_while(|d| d.starts_with(root))
            .map(Path::to_path_buf)
            .collect();
        dirs.reverse();
        let levels = dirs
            .into_iter()
            .filter_map(|d| {
                let file = d.join(".gitignore");
                if !file.is_file() {
                    return None;
                }
                let mut builder = ignore::gitignore::GitignoreBuilder::new(&d);
                // A bad line is skipped; the rest of the file still applies.
                let _ = builder.add(&file);
                builder.build().ok()
            })
            .collect();
        Self { levels }
    }

    /// Whether `path` is ignored; the deepest `.gitignore` with an opinion
    /// decides (so `!pattern` in a subdirectory re-includes).
    fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        for level in self.levels.iter().rev() {
            let m = level.matched(path, is_dir);
            if m.is_ignore() {
                return true;
            }
            if m.is_whitelist() {
                return false;
            }
        }
        false
    }

    /// Whether `dir` or any directory between the root and it is ignored.
    fn ancestors_ignored(&self, root: &Path, dir: &Path) -> bool {
        dir.ancestors()
            .take_while(|d| *d != root && d.starts_with(root))
            .any(|d| {
                d.file_name()
                    .is_some_and(|n| IGNORED_DIRS.contains(&n.to_string_lossy().as_ref()))
                    || self.is_ignored(d, true)
            })
    }
}

fn compare_entries(a: &Entry, b: &Entry) -> Ordering {
    let dir_first = |e: &Entry| e.kind != EntryKind::Directory;
    dir_first(a)
        .cmp(&dir_first(b))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// Canonicalizes without Windows `\\?\` verbatim prefixes where possible, so
/// paths stay readable and comparable.
fn dunce_canonicalize(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let s = canonical.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\")
            && !rest.starts_with("UNC\\")
        {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/components")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join("Docs")).unwrap();
        fs::write(root.join("b.ts"), "").unwrap();
        fs::write(root.join("A.ts"), "").unwrap();
        fs::write(root.join(".gitignore"), "").unwrap();
        fs::write(root.join("src/index.ts"), "").unwrap();
        let project = Project::open(root).unwrap();
        (dir, project)
    }

    #[test]
    fn lists_dirs_first_sorted_and_hides_git() {
        let (_dir, project) = sample();
        let names: Vec<_> = project
            .list_dir("")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Docs", "src", ".gitignore", "A.ts", "b.ts"]);
        let src = project.list_dir("src").unwrap();
        assert_eq!(src[0].path, "src/components");
        assert_eq!(src[0].kind, EntryKind::Directory);
        assert_eq!(src[1].path, "src/index.ts");
        assert_eq!(src[1].kind, EntryKind::File);
    }

    #[test]
    fn marks_ignored_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("node_modules/react")).unwrap();
        fs::create_dir_all(root.join("dist/assets")).unwrap();
        fs::create_dir_all(root.join("src/gen")).unwrap();
        fs::write(root.join(".gitignore"), "dist\n*.log\n").unwrap();
        fs::write(root.join("src/.gitignore"), "gen/\n!keep.log\n").unwrap();
        fs::write(root.join("debug.log"), "").unwrap();
        fs::write(root.join("src/keep.log"), "").unwrap();
        fs::write(root.join("src/other.log"), "").unwrap();
        fs::write(root.join("src/a.ts"), "").unwrap();
        let project = Project::open(root).unwrap();
        let flags = |rel: &str| -> Vec<(String, bool)> {
            project
                .list_dir(rel)
                .unwrap()
                .into_iter()
                .map(|e| (e.name, e.ignored))
                .collect()
        };
        let top = flags("");
        assert!(top.contains(&("dist".into(), true)));
        assert!(top.contains(&("node_modules".into(), true)));
        assert!(top.contains(&("debug.log".into(), true)));
        assert!(top.contains(&("src".into(), false)));
        let src = flags("src");
        assert!(src.contains(&("gen".into(), true)));
        assert!(
            src.contains(&("keep.log".into(), false)),
            "re-included by src/.gitignore"
        );
        assert!(src.contains(&("other.log".into(), true)));
        assert!(src.contains(&("a.ts".into(), false)));
        assert!(
            flags("dist").iter().all(|(_, ignored)| *ignored),
            "inside an ignored dir"
        );
        assert!(flags("node_modules").iter().all(|(_, ignored)| *ignored));
    }

    #[test]
    fn rejects_escaping_paths() {
        let (_dir, project) = sample();
        for bad in [
            "..",
            "src/../..",
            "/etc",
            "src//x",
            "./src",
            "a\\b",
            "C:",
            "src/",
        ] {
            assert!(
                matches!(project.resolve(bad), Err(ProjectError::InvalidPath(_))),
                "{bad} should be rejected"
            );
        }
        assert_eq!(
            project.resolve("src/index.ts").unwrap(),
            project.root().join("src").join("index.ts")
        );
    }

    #[test]
    fn relativize_round_trips() {
        let (_dir, project) = sample();
        let abs = project.resolve("src/components").unwrap();
        assert_eq!(project.relativize(&abs).as_deref(), Some("src/components"));
        assert_eq!(project.relativize(Path::new("/elsewhere")), None);
        assert_eq!(project.relativize(project.root()).as_deref(), Some(""));
    }

    #[test]
    fn open_rejects_files() {
        let (_dir, project) = sample();
        let file = project.resolve("b.ts").unwrap();
        assert!(matches!(
            Project::open(&file),
            Err(ProjectError::NotADirectory(_))
        ));
    }
}
