//! Project index: the list of project files and their symbols.
//!
//! - The walk honours `.gitignore`/`.ignore` files (also outside Git
//!   repositories) and always skips `.git` and `node_modules`.
//! - Symbols come from the languages' tags queries (TS/JS in M2).
//! - The index is cached on disk as JSON; files whose modification time and
//!   size are unchanged are not re-parsed.
//! - [`Index::update`] refreshes individual paths reported by the watcher.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ferrule_syntax::LanguageRegistry;
use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};

/// Directory names that are never indexed or watched.
pub const EXCLUDED_DIRS: &[&str] = &[".git", "node_modules"];

/// Files larger than this are listed but not parsed for symbols.
const MAX_SYMBOL_FILE_SIZE: u64 = 1024 * 1024;

const CACHE_FORMAT: u32 = 1;

/// A symbol definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    /// Name.
    pub name: String,
    /// Kind (`function`, `class`, `method`, `interface`, `type`, ...).
    pub kind: String,
    /// Zero-based line.
    pub line: u32,
    /// Zero-based UTF-16 column.
    pub column: u32,
}

/// One indexed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedFile {
    /// Project-relative path with `/` separators.
    pub path: String,
    /// Modification time (Unix epoch milliseconds).
    pub mtime_ms: u64,
    /// Size in bytes.
    pub size: u64,
    /// Symbols defined in the file.
    pub symbols: Vec<Symbol>,
}

/// What a build did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildStats {
    /// Files in the index.
    pub files: usize,
    /// Files parsed for symbols in this build.
    pub parsed: usize,
    /// Files whose cached entry was reused.
    pub reused: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    format: u32,
    root: PathBuf,
    files: Vec<IndexedFile>,
}

/// The index of one project root.
#[derive(Debug, Clone)]
pub struct Index {
    root: PathBuf,
    files: BTreeMap<String, IndexedFile>,
    dirs: Vec<PathBuf>,
    ignore: Gitignore,
}

fn is_excluded_name(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

fn rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn stat(meta: &fs::Metadata) -> (u64, u64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    (mtime, meta.len())
}

fn root_ignore(root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    for name in [".gitignore", ".ignore"] {
        let file = root.join(name);
        if file.is_file() {
            // A malformed line is reported and skipped; the rest still applies.
            if let Some(err) = builder.add(&file) {
                tracing::warn!(?file, %err, "ignore file has errors");
            }
        }
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn parse_symbols(registry: &LanguageRegistry, path: &Path, size: u64) -> Vec<Symbol> {
    if size > MAX_SYMBOL_FILE_SIZE {
        return Vec::new();
    }
    let Some(language) = registry.for_path(path).filter(|l| l.has_symbols()) else {
        return Vec::new();
    };
    let Ok(file) = ferrule_vfs::read_text(path) else {
        return Vec::new();
    };
    language
        .symbols(&file.text)
        .into_iter()
        .map(|s| Symbol {
            name: s.name,
            kind: s.kind,
            line: s.line,
            column: s.column,
        })
        .collect()
}

impl Index {
    /// Walks `root` and builds the index, reusing entries from `cache` whose
    /// mtime and size match. Returns `None` if `cancelled` returns true.
    pub fn build(
        root: &Path,
        registry: &LanguageRegistry,
        cache: Option<Index>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Option<(Self, BuildStats)> {
        let mut cached = cache.map(|c| c.files).unwrap_or_default();
        let mut files = BTreeMap::new();
        let mut dirs = vec![root.to_path_buf()];
        let mut to_parse: Vec<(String, PathBuf, u64, u64)> = Vec::new();
        let mut stats = BuildStats::default();

        let walker = WalkBuilder::new(root)
            .hidden(false)
            .require_git(false)
            .git_global(false)
            .parents(false)
            .filter_entry(|e| {
                !(e.depth() > 0 && is_excluded_name(&e.file_name().to_string_lossy()))
            })
            .build();
        for entry in walker {
            if cancelled() {
                return None;
            }
            let Ok(entry) = entry else { continue };
            if entry.depth() == 0 {
                continue;
            }
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                dirs.push(entry.into_path());
                continue;
            }
            let Some(rel) = rel_path(root, entry.path()) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            let (mtime, size) = stat(&meta);
            match cached.remove(&rel) {
                Some(old) if old.mtime_ms == mtime && old.size == size => {
                    stats.reused += 1;
                    files.insert(rel, old);
                }
                _ => to_parse.push((rel, entry.into_path(), mtime, size)),
            }
        }

        // Parse in parallel; tree-sitter parsing is CPU-bound.
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(8);
        let chunk = to_parse.len().div_ceil(threads).max(1);
        let parsed: Vec<IndexedFile> = std::thread::scope(|scope| {
            let handles: Vec<_> = to_parse
                .chunks(chunk)
                .map(|items| {
                    scope.spawn(move || {
                        let mut out = Vec::with_capacity(items.len());
                        for (rel, abs, mtime, size) in items {
                            if cancelled() {
                                break;
                            }
                            out.push(IndexedFile {
                                path: rel.clone(),
                                mtime_ms: *mtime,
                                size: *size,
                                symbols: parse_symbols(registry, abs, *size),
                            });
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        if cancelled() {
            return None;
        }
        stats.parsed = parsed.len();
        for file in parsed {
            files.insert(file.path.clone(), file);
        }
        stats.files = files.len();
        Some((
            Self {
                root: root.to_path_buf(),
                files,
                dirs,
                ignore: root_ignore(root),
            },
            stats,
        ))
    }

    /// Project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All indexed files, sorted by path.
    pub fn files(&self) -> impl Iterator<Item = &IndexedFile> {
        self.files.values()
    }

    /// Number of indexed files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Directories that were walked (for the watcher), root included.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Whether `path` (absolute) is excluded from the index.
    pub fn is_excluded(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return true;
        };
        if rel
            .components()
            .any(|c| is_excluded_name(&c.as_os_str().to_string_lossy()))
        {
            return true;
        }
        let is_dir = path.is_dir();
        self.ignore
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
    }

    /// Refreshes the given absolute paths (files or directories, existing or
    /// removed). Returns directories that are new and should be watched.
    pub fn update(&mut self, registry: &LanguageRegistry, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut new_dirs = Vec::new();
        for path in paths {
            if self.is_excluded(path) {
                continue;
            }
            let Some(rel) = rel_path(&self.root, path) else {
                continue;
            };
            match fs::metadata(path) {
                Ok(meta) if meta.is_dir() => {
                    if !self.dirs.contains(path) {
                        // A new (or moved-in) directory: index its contents.
                        if let Some((sub, _)) = Index::build(path, registry, None, &|| false) {
                            for dir in sub.dirs {
                                if !self.dirs.contains(&dir) {
                                    self.dirs.push(dir.clone());
                                    new_dirs.push(dir);
                                }
                            }
                            for (sub_rel, mut file) in sub.files {
                                file.path = format!("{rel}/{sub_rel}");
                                self.files.insert(file.path.clone(), file);
                            }
                        }
                    }
                }
                Ok(meta) => {
                    let (mtime, size) = stat(&meta);
                    let unchanged = self
                        .files
                        .get(&rel)
                        .is_some_and(|f| f.mtime_ms == mtime && f.size == size);
                    if !unchanged {
                        let symbols = parse_symbols(registry, path, size);
                        self.files.insert(
                            rel.clone(),
                            IndexedFile {
                                path: rel,
                                mtime_ms: mtime,
                                size,
                                symbols,
                            },
                        );
                    }
                }
                Err(_) => {
                    // Removed: drop the file, or everything under the directory.
                    let prefix = format!("{rel}/");
                    self.files
                        .retain(|p, _| p != &rel && !p.starts_with(&prefix));
                    self.dirs.retain(|d| !d.starts_with(path));
                }
            }
        }
        new_dirs
    }

    /// Loads a cached index for `root` from `file`, if valid.
    pub fn load_cache(file: &Path, root: &Path) -> Option<Self> {
        let bytes = fs::read(file).ok()?;
        let cache: CacheFile = serde_json::from_slice(&bytes).ok()?;
        if cache.format != CACHE_FORMAT || cache.root != root {
            return None;
        }
        Some(Self {
            root: root.to_path_buf(),
            files: cache
                .files
                .into_iter()
                .map(|f| (f.path.clone(), f))
                .collect(),
            dirs: Vec::new(),
            ignore: Gitignore::empty(),
        })
    }

    /// Writes the index to `file` (atomically).
    pub fn save_cache(&self, file: &Path) -> io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let cache = CacheFile {
            format: CACHE_FORMAT,
            root: self.root.clone(),
            files: self.files.values().cloned().collect(),
        };
        let json = serde_json::to_vec(&cache).map_err(io::Error::other)?;
        ferrule_vfs::write_atomic(file, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> LanguageRegistry {
        let mut r = LanguageRegistry::default();
        for def in ferrule_lang_js::languages() {
            r.register(def).unwrap();
        }
        r
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/components")).unwrap();
        fs::create_dir_all(root.join("node_modules/react")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "dist\n*.log\n").unwrap();
        fs::write(
            root.join("src/components/Counter.tsx"),
            "export function Counter() {}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/util.ts"),
            "export class Store {}\nexport type Id = string;\n",
        )
        .unwrap();
        fs::write(
            root.join("node_modules/react/index.js"),
            "function hidden() {}",
        )
        .unwrap();
        fs::write(root.join("dist/bundle.js"), "function built() {}").unwrap();
        fs::write(root.join("debug.log"), "x").unwrap();
        fs::write(root.join(".git/HEAD"), "ref").unwrap();
        fs::write(root.join("README.md"), "# hi").unwrap();
        dir
    }

    fn paths(index: &Index) -> Vec<&str> {
        index.files().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn walk_honours_gitignore_and_exclusions() {
        let dir = project();
        let (index, stats) = Index::build(dir.path(), &registry(), None, &|| false).unwrap();
        assert_eq!(
            paths(&index),
            vec![
                ".gitignore",
                "README.md",
                "src/components/Counter.tsx",
                "src/util.ts"
            ]
        );
        assert_eq!(stats.parsed, 4);
        let util = index.files().find(|f| f.path == "src/util.ts").unwrap();
        let names: Vec<_> = util
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert_eq!(names, vec![("Store", "class"), ("Id", "type")]);
        assert!(
            index
                .dirs()
                .iter()
                .all(|d| !d.ends_with("node_modules") && !d.ends_with("dist"))
        );
        assert!(index.is_excluded(&dir.path().join("dist/x.js")));
        assert!(index.is_excluded(&dir.path().join("node_modules/a/b.js")));
        assert!(index.is_excluded(&dir.path().join("trace.log")));
        assert!(!index.is_excluded(&dir.path().join("src/new.ts")));
    }

    #[test]
    fn cache_round_trip_reuses_unchanged_files() {
        let dir = project();
        let reg = registry();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache_file = cache_dir.path().join("index/cache.json");
        let (index, _) = Index::build(dir.path(), &reg, None, &|| false).unwrap();
        index.save_cache(&cache_file).unwrap();

        // Change one file (different size so the test does not depend on mtime resolution).
        fs::write(dir.path().join("src/util.ts"), "export class Store2 {}\n").unwrap();
        let cached = Index::load_cache(&cache_file, dir.path()).unwrap();
        let (index, stats) = Index::build(dir.path(), &reg, Some(cached), &|| false).unwrap();
        assert_eq!(stats.reused, 3);
        assert_eq!(stats.parsed, 1, "only the changed file is parsed again");
        let util = index.files().find(|f| f.path == "src/util.ts").unwrap();
        assert_eq!(util.symbols[0].name, "Store2");

        assert!(Index::load_cache(&cache_file, Path::new("/other/root")).is_none());
    }

    #[test]
    fn update_follows_creates_changes_and_removes() {
        let dir = project();
        let reg = registry();
        let (mut index, _) = Index::build(dir.path(), &reg, None, &|| false).unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("src/new/deep")).unwrap();
        fs::write(
            root.join("src/new/deep/a.ts"),
            "export function deepFn() {}\n",
        )
        .unwrap();
        fs::write(root.join("src/util.ts"), "export interface Changed {}\n").unwrap();
        fs::write(root.join("ignored.log"), "x").unwrap();
        let new_dirs = index.update(
            &reg,
            &[
                root.join("src/new"),
                root.join("src/util.ts"),
                root.join("ignored.log"),
            ],
        );
        assert_eq!(
            new_dirs.len(),
            2,
            "src/new and src/new/deep are new: {new_dirs:?}"
        );
        assert!(paths(&index).contains(&"src/new/deep/a.ts"));
        assert!(!paths(&index).contains(&"ignored.log"));
        let util = index.files().find(|f| f.path == "src/util.ts").unwrap();
        assert_eq!(util.symbols[0].name, "Changed");

        fs::remove_dir_all(root.join("src/new")).unwrap();
        fs::remove_file(root.join("README.md")).unwrap();
        index.update(&reg, &[root.join("src/new"), root.join("README.md")]);
        assert!(
            !paths(&index)
                .iter()
                .any(|p| p.starts_with("src/new") || *p == "README.md")
        );
    }

    #[test]
    fn cancellation_stops_the_build() {
        let dir = project();
        assert!(Index::build(dir.path(), &registry(), None, &|| true).is_none());
    }
}
