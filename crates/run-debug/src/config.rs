//! Run configurations and their file in the project, `.ferrule/run.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version of the `.ferrule/run.json` format.
const FORMAT_VERSION: u32 = 1;

/// A package manager, for running `package.json` scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageManager {
    /// bun.
    Bun,
    /// pnpm.
    Pnpm,
    /// Yarn.
    Yarn,
    /// npm.
    Npm,
}

impl PackageManager {
    /// The executable's name.
    pub fn program(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Npm => "npm",
        }
    }
}

/// A test framework with a JSON reporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestFramework {
    /// Vitest.
    Vitest,
    /// Jest.
    Jest,
    /// Playwright Test.
    Playwright,
}

impl TestFramework {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Vitest => "Vitest",
            Self::Jest => "Jest",
            Self::Playwright => "Playwright",
        }
    }

    /// The npm package whose `bin` runs the tests, and the bin's name.
    pub fn package(self) -> (&'static str, &'static str) {
        match self {
            Self::Vitest => ("vitest", "vitest"),
            Self::Jest => ("jest", "jest"),
            Self::Playwright => ("@playwright/test", "playwright"),
        }
    }
}

/// One test (or suite) to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRef {
    /// Its file, relative to the working directory.
    pub file: String,
    /// Its full name: titles from the outermost suite (as reporters and
    /// the gutter list them).
    pub name: Vec<String>,
    /// Its one-based line (Playwright selects tests by line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Which tests a test configuration runs: everything if both lists are
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestScope {
    /// Whole test files, relative to the working directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Single tests or suites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestRef>,
}

/// What a configuration runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum RunKind {
    /// `node <file>`.
    Node {
        /// The file, relative to the working directory.
        file: String,
    },
    /// A `package.json` script through the package manager.
    NpmScript {
        /// The script's name.
        script: String,
    },
    /// Tests with a JSON report.
    Test {
        /// The framework.
        framework: TestFramework,
        /// Which tests.
        #[serde(default)]
        scope: TestScope,
    },
}

/// A run configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    /// Stable id (`detected:...` for detected ones until saved).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What runs.
    pub kind: RunKind,
    /// Working directory relative to the project root (`""` = the root).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// Extra arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Package manager for npm scripts (detected if `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<PackageManager>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    configurations: Vec<RunConfig>,
}

/// Errors reading or writing `.ferrule/run.json`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read or written.
    #[error("{path}: {message}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What failed.
        message: String,
    },
    /// The file is not a configuration file Ferrule understands.
    #[error("{path} is not a valid run configuration file: {message}")]
    Invalid {
        /// The file.
        path: PathBuf,
        /// Why.
        message: String,
    },
}

fn file_path(root: &Path) -> PathBuf {
    root.join(".ferrule").join("run.json")
}

/// The configurations stored in the project (none if the file is missing).
pub fn load_configs(root: &Path) -> Result<Vec<RunConfig>, ConfigError> {
    let path = file_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(ConfigError::Io {
                path,
                message: err.to_string(),
            });
        }
    };
    let file: File = serde_json::from_str(&text).map_err(|err| ConfigError::Invalid {
        path: path.clone(),
        message: err.to_string(),
    })?;
    if file.version != FORMAT_VERSION {
        return Err(ConfigError::Invalid {
            path,
            message: format!(
                "version {} (this Ferrule reads version {FORMAT_VERSION})",
                file.version
            ),
        });
    }
    Ok(file.configurations)
}

/// Writes the configurations, sorted by name then id, so the file diffs
/// well. Creates `.ferrule/` if needed; the write is atomic.
pub fn save_configs(root: &Path, configs: &[RunConfig]) -> Result<(), ConfigError> {
    let path = file_path(root);
    let io = |err: std::io::Error| ConfigError::Io {
        path: path.clone(),
        message: err.to_string(),
    };
    let mut configurations = configs.to_vec();
    configurations.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let file = File {
        version: FORMAT_VERSION,
        configurations,
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(|err| ConfigError::Invalid {
        path: path.clone(),
        message: err.to_string(),
    })?;
    text.push('\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    // Atomic: a temporary file next to it, then a rename.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(io)?;
    std::fs::rename(&temp, &path).map_err(io)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn config(id: &str, name: &str, kind: RunKind) -> RunConfig {
        RunConfig {
            id: id.into(),
            name: name.into(),
            kind,
            cwd: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            package_manager: None,
        }
    }

    #[test]
    fn round_trips_sorted_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_configs(dir.path()).unwrap().is_empty());
        let mut server = config(
            "c2",
            "Start server",
            RunKind::NpmScript {
                script: "start".into(),
            },
        );
        server.env.insert("PORT".into(), "4000".into());
        server.args = vec!["--verbose".into()];
        server.package_manager = Some(PackageManager::Pnpm);
        let tests = config(
            "c1",
            "All tests",
            RunKind::Test {
                framework: TestFramework::Vitest,
                scope: TestScope::default(),
            },
        );
        save_configs(dir.path(), &[server.clone(), tests.clone()]).unwrap();
        let text = std::fs::read_to_string(dir.path().join(".ferrule/run.json")).unwrap();
        assert!(text.starts_with("{\n  \"version\": 1,"), "{text}");
        assert!(text.find("All tests").unwrap() < text.find("Start server").unwrap());
        assert!(text.contains("\"type\": \"npmScript\""), "{text}");
        assert!(!text.contains("\"cwd\""), "empty fields are left out");
        assert_eq!(load_configs(dir.path()).unwrap(), [tests, server]);
    }

    #[test]
    fn rejects_other_versions_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ferrule")).unwrap();
        std::fs::write(
            dir.path().join(".ferrule/run.json"),
            r#"{"version": 9, "configurations": []}"#,
        )
        .unwrap();
        let err = load_configs(dir.path()).unwrap_err().to_string();
        assert!(err.contains("version 9"), "{err}");
        std::fs::write(dir.path().join(".ferrule/run.json"), "{ nope").unwrap();
        assert!(matches!(
            load_configs(dir.path()),
            Err(ConfigError::Invalid { .. })
        ));
    }
}
