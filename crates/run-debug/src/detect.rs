//! Configurations and the package manager, from `package.json` and the
//! lockfile.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::config::{PackageManager, RunConfig, RunKind, TestFramework, TestScope};

/// Lockfiles, in the order they are looked for.
const LOCKFILES: &[(&str, PackageManager)] = &[
    ("bun.lock", PackageManager::Bun),
    ("bun.lockb", PackageManager::Bun),
    ("pnpm-lock.yaml", PackageManager::Pnpm),
    ("yarn.lock", PackageManager::Yarn),
    ("package-lock.json", PackageManager::Npm),
];

/// The package manager of the project in `dirs` (the working directory,
/// then its parents up to the project root): `packageManager` in the first
/// `package.json` that has it, else the first lockfile found, else npm.
pub fn detect_package_manager(dirs: &[&Path]) -> PackageManager {
    for dir in dirs {
        let field = std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|json| json.get("packageManager")?.as_str().map(str::to_owned));
        if let Some(manager) = field.as_deref().and_then(from_field) {
            return manager;
        }
        if let Some((_, manager)) = LOCKFILES.iter().find(|(name, _)| dir.join(name).is_file()) {
            return *manager;
        }
    }
    PackageManager::Npm
}

/// `"pnpm@9.1.0"` (Corepack's `packageManager` field).
fn from_field(field: &str) -> Option<PackageManager> {
    match field.split('@').next()?.trim() {
        "bun" => Some(PackageManager::Bun),
        "pnpm" => Some(PackageManager::Pnpm),
        "yarn" => Some(PackageManager::Yarn),
        "npm" => Some(PackageManager::Npm),
        _ => None,
    }
}

fn depends_on(json: &Value, package: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .iter()
        .any(|key| json.get(key).and_then(|deps| deps.get(package)).is_some())
}

fn detected(id: String, name: String, kind: RunKind) -> RunConfig {
    RunConfig {
        id,
        name,
        kind,
        cwd: String::new(),
        args: Vec::new(),
        env: BTreeMap::new(),
        package_manager: None,
    }
}

/// One configuration per script of `package_json`, and one per test
/// framework in its dependencies. Ids start with `detected:`.
pub fn detect_configs(package_json: &str) -> Vec<RunConfig> {
    let Ok(json) = serde_json::from_str::<Value>(package_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(scripts) = json.get("scripts").and_then(Value::as_object) {
        for script in scripts.keys() {
            out.push(detected(
                format!("detected:script:{script}"),
                script.clone(),
                RunKind::NpmScript {
                    script: script.clone(),
                },
            ));
        }
    }
    for framework in [
        TestFramework::Vitest,
        TestFramework::Jest,
        TestFramework::Playwright,
    ] {
        let (package, _) = framework.package();
        if depends_on(&json, package)
            || (package == "@playwright/test" && depends_on(&json, "playwright"))
        {
            out.push(detected(
                format!("detected:test:{}", framework.name().to_lowercase()),
                format!("{} (all tests)", framework.name()),
                RunKind::Test {
                    framework,
                    scope: TestScope::default(),
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn scripts_and_frameworks_become_configurations() {
        let configs = detect_configs(
            r#"{
              "scripts": { "dev": "vite", "test": "vitest run" },
              "devDependencies": { "vitest": "^3", "@playwright/test": "^1.50" }
            }"#,
        );
        let ids: Vec<&str> = configs.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "detected:script:dev",
                "detected:script:test",
                "detected:test:vitest",
                "detected:test:playwright"
            ]
        );
        assert_eq!(configs[2].name, "Vitest (all tests)");
        assert!(detect_configs("not json").is_empty());
    }

    #[test]
    fn package_manager_from_field_then_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(detect_package_manager(&[&app, root]), PackageManager::Npm);
        std::fs::write(root.join("yarn.lock"), "").unwrap();
        assert_eq!(detect_package_manager(&[&app, root]), PackageManager::Yarn);
        std::fs::write(app.join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager(&[&app, root]), PackageManager::Bun);
        std::fs::write(
            app.join("package.json"),
            r#"{ "packageManager": "pnpm@9.12.0" }"#,
        )
        .unwrap();
        assert_eq!(detect_package_manager(&[&app, root]), PackageManager::Pnpm);
    }
}
