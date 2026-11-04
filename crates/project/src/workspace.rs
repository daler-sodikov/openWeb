//! JavaScript/TypeScript workspace layout: `tsconfig.json`/`jsconfig.json`
//! files, npm/yarn/Bun (`package.json` `workspaces`) or pnpm
//! (`pnpm-workspace.yaml`) packages, and the package manager.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::Project;

/// Directories never searched for configs or packages.
const SKIPPED_DIRS: &[&str] = &["node_modules", ".git", "dist", "build", "out", "coverage"];
/// How deep the search goes below the root.
const MAX_DEPTH: usize = 6;
/// At most this many config files and packages are reported.
const MAX_RESULTS: usize = 500;

/// Package manager of a project, from its lock file or `packageManager`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    /// npm (`package-lock.json`).
    Npm,
    /// Yarn (`yarn.lock`).
    Yarn,
    /// pnpm (`pnpm-lock.yaml`).
    Pnpm,
    /// Bun (`bun.lock`, `bun.lockb`).
    Bun,
}

impl PackageManager {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Yarn => "Yarn",
            Self::Pnpm => "pnpm",
            Self::Bun => "Bun",
        }
    }
}

/// What [`Project::detect_workspace`] found. Paths are project-relative
/// with `/` separators, sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    /// `tsconfig*.json` and `jsconfig.json` files.
    pub config_files: Vec<String>,
    /// Workspace package directories (containing a `package.json`).
    pub packages: Vec<String>,
    /// The package manager, if known.
    pub package_manager: Option<PackageManager>,
}

impl Project {
    /// Detects the JS/TS workspace layout. Reads a few small files and walks
    /// at most [`MAX_DEPTH`] levels, skipping ignored directories.
    pub fn detect_workspace(&self) -> Workspace {
        let root = self.root();
        let root_package = std::fs::read_to_string(root.join("package.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let mut patterns = root_package
            .as_ref()
            .map(package_json_workspaces)
            .unwrap_or_default();
        if let Ok(text) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) {
            patterns.extend(pnpm_workspace_packages(&text));
        }
        let packages = PackagePatterns::new(&patterns);

        let mut workspace = Workspace {
            package_manager: package_manager(root, root_package.as_ref()),
            ..Workspace::default()
        };
        let walker = ignore::WalkBuilder::new(root)
            .max_depth(Some(MAX_DEPTH))
            .hidden(true)
            .filter_entry(|entry| {
                !(entry.file_type().is_some_and(|t| t.is_dir())
                    && SKIPPED_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()))
            })
            .build();
        for entry in walker.flatten() {
            if workspace.config_files.len() >= MAX_RESULTS
                || workspace.packages.len() >= MAX_RESULTS
            {
                break;
            }
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            let Some(rel) = self.relativize(entry.path()) else {
                continue;
            };
            if is_config_file(&name) {
                workspace.config_files.push(rel.clone());
            }
            if name == "package.json"
                && let Some((dir, _)) = rel.rsplit_once('/')
                && packages.matches(dir)
            {
                workspace.packages.push(dir.to_owned());
            }
        }
        workspace.config_files.sort();
        workspace.packages.sort();
        workspace
    }
}

fn is_config_file(name: &str) -> bool {
    name == "jsconfig.json"
        || (name.starts_with("tsconfig")
            && name.ends_with(".json")
            && !name.contains(".tsbuildinfo"))
}

/// `workspaces` as an array, or as `{ "packages": [...] }` (Yarn classic).
fn package_json_workspaces(package: &serde_json::Value) -> Vec<String> {
    let list = match package.get("workspaces") {
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(serde_json::Value::Object(obj)) => obj
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// The `packages:` list of `pnpm-workspace.yaml`. Only the block-list form
/// (`- 'glob'` lines) is read; flow lists and anchors are not supported.
fn pnpm_workspace_packages(text: &str) -> Vec<String> {
    let mut in_packages = false;
    let mut result = Vec::new();
    for line in text.lines() {
        let content = line.split(" #").next().unwrap_or(line).trim_end();
        if content.trim().is_empty() || content.trim_start().starts_with('#') {
            continue;
        }
        let indented = content.starts_with([' ', '\t']);
        if !indented {
            in_packages = content.trim() == "packages:";
            continue;
        }
        if in_packages && let Some(item) = content.trim().strip_prefix('-') {
            let item = item.trim().trim_matches(['"', '\'']);
            if !item.is_empty() {
                result.push(item.to_owned());
            }
        }
    }
    result
}

fn package_manager(root: &Path, package: Option<&serde_json::Value>) -> Option<PackageManager> {
    let by_lock = [
        ("bun.lock", PackageManager::Bun),
        ("bun.lockb", PackageManager::Bun),
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("package-lock.json", PackageManager::Npm),
    ];
    if let Some((_, manager)) = by_lock.iter().find(|(file, _)| root.join(file).is_file()) {
        return Some(*manager);
    }
    let declared = package?.get("packageManager")?.as_str()?;
    let name = declared.split('@').next()?;
    match name {
        "npm" => Some(PackageManager::Npm),
        "yarn" => Some(PackageManager::Yarn),
        "pnpm" => Some(PackageManager::Pnpm),
        "bun" => Some(PackageManager::Bun),
        _ => None,
    }
}

/// Include and exclude (`!pattern`) globs over package directories.
struct PackagePatterns {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

impl PackagePatterns {
    fn new(patterns: &[String]) -> Self {
        let mut include = GlobSetBuilder::new();
        let mut exclude = GlobSetBuilder::new();
        let (mut any_include, mut any_exclude) = (false, false);
        for pattern in patterns {
            let (negated, pattern) = match pattern.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, pattern.as_str()),
            };
            let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
            let Ok(glob) = Glob::new(pattern) else {
                tracing::warn!(pattern, "ignoring an invalid workspace pattern");
                continue;
            };
            if negated {
                exclude.add(glob);
                any_exclude = true;
            } else {
                include.add(glob);
                any_include = true;
            }
        }
        Self {
            include: any_include.then(|| include.build().ok()).flatten(),
            exclude: any_exclude.then(|| exclude.build().ok()).flatten(),
        }
    }

    fn matches(&self, dir: &str) -> bool {
        self.include.as_ref().is_some_and(|set| set.is_match(dir))
            && !self.exclude.as_ref().is_some_and(|set| set.is_match(dir))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn finds_configs_npm_workspaces_and_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{ "workspaces": ["packages/*", "!packages/skip"] }"#,
        );
        write(root, "package-lock.json", "{}");
        write(root, "tsconfig.json", "{}");
        write(root, "packages/ui/package.json", "{}");
        write(root, "packages/ui/tsconfig.build.json", "{}");
        write(root, "packages/skip/package.json", "{}");
        write(root, "apps/web/package.json", "{}");
        write(root, "apps/web/jsconfig.json", "{}");
        write(root, "node_modules/lib/package.json", "{}");
        write(root, "node_modules/lib/tsconfig.json", "{}");
        let ws = Project::open(root).unwrap().detect_workspace();
        assert_eq!(
            ws.config_files,
            [
                "apps/web/jsconfig.json",
                "packages/ui/tsconfig.build.json",
                "tsconfig.json"
            ]
        );
        assert_eq!(ws.packages, ["packages/ui"]);
        assert_eq!(ws.package_manager, Some(PackageManager::Npm));
    }

    #[test]
    fn reads_yarn_object_form_and_pnpm_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{ "workspaces": { "packages": ["libs/*"] }, "packageManager": "pnpm@9.1.0" }"#,
        );
        write(
            root,
            "pnpm-workspace.yaml",
            "# monorepo\npackages:\n  - 'apps/*'   # apps\n  - \"tools/cli\"\ncatalog:\n  - x\n",
        );
        write(root, "libs/a/package.json", "{}");
        write(root, "apps/b/package.json", "{}");
        write(root, "tools/cli/package.json", "{}");
        write(root, "other/c/package.json", "{}");
        let ws = Project::open(root).unwrap().detect_workspace();
        assert_eq!(ws.packages, ["apps/b", "libs/a", "tools/cli"]);
        assert_eq!(ws.package_manager, Some(PackageManager::Pnpm));
        assert!(ws.config_files.is_empty());
    }

    #[test]
    fn empty_project() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Project::open(dir.path()).unwrap().detect_workspace();
        assert_eq!(ws, Workspace::default());
    }
}
