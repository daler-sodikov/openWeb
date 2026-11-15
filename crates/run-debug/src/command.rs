//! The exact command a configuration runs.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{PackageManager, RunConfig, RunKind, TestFramework, TestScope};
use crate::detect::detect_package_manager;

/// The operating system family (how programs are started).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux, macOS and other Unix systems.
    Unix,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this binary runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// What building a command needs from the environment.
pub struct Tools<'a> {
    /// Finds a program on `PATH` (on Windows also `PATHEXT` names).
    pub which: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// The OS.
    pub os: Os,
}

/// A program with its arguments, working directory and extra environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    /// The program.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// The working directory.
    pub cwd: PathBuf,
    /// Variables added to the environment.
    pub env: Vec<(String, String)>,
}

impl CommandLine {
    /// The command as a person would type it (for the trust prompt and the
    /// run tab), environment first.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = self
            .env
            .iter()
            .map(|(key, value)| format!("{key}={}", quote(value)))
            .collect();
        parts.push(quote(&self.program.to_string_lossy()));
        parts.extend(self.args.iter().map(|a| quote(a)));
        parts.join(" ")
    }
}

fn quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:=@%+,\\".contains(c));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// Why a configuration cannot run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// A program is not installed or not on `PATH`.
    #[error("{program} was not found on PATH")]
    ProgramNotFound {
        /// The program.
        program: String,
    },
    /// A test framework is not installed in `node_modules`.
    #[error(
        "{package} is not installed (no node_modules/{package} from {cwd} upwards); install the project's dependencies"
    )]
    PackageNotFound {
        /// The package.
        package: String,
        /// Where the search started.
        cwd: String,
    },
    /// The package has no runnable `bin` entry.
    #[error("{package} has no `{bin}` entry in its package.json `bin`")]
    NoBin {
        /// The package.
        package: String,
        /// The bin's name.
        bin: String,
    },
    /// An argument cmd.exe would interpret (Windows package managers run
    /// through cmd.exe).
    #[error("on Windows, `{argument}` cannot be passed through cmd.exe safely")]
    UnsafeArgument {
        /// The argument.
        argument: String,
    },
    /// The working directory is missing.
    #[error("the working directory {0} does not exist")]
    NoWorkingDirectory(String),
}

/// The command `config` runs in the project at `root`. Test runs write
/// their JSON report to `report`.
pub fn command_line(
    config: &RunConfig,
    root: &Path,
    report: Option<&Path>,
    tools: &Tools<'_>,
) -> Result<CommandLine, CommandError> {
    let cwd = if config.cwd.is_empty() {
        root.to_path_buf()
    } else {
        root.join(&config.cwd)
    };
    if !cwd.is_dir() {
        return Err(CommandError::NoWorkingDirectory(
            cwd.to_string_lossy().into_owned(),
        ));
    }
    let mut env: Vec<(String, String)> = config
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let node = || {
        (tools.which)("node").ok_or_else(|| CommandError::ProgramNotFound {
            program: "node".into(),
        })
    };
    let (program, args) = match &config.kind {
        RunKind::Node { file } => {
            let mut args = vec![file.clone()];
            args.extend(config.args.iter().cloned());
            (node()?, args)
        }
        RunKind::NpmScript { script } => {
            let manager = config
                .package_manager
                .unwrap_or_else(|| detect_package_manager(&[cwd.as_path(), root]));
            let mut args = vec!["run".to_owned(), script.clone()];
            if manager == PackageManager::Npm && !config.args.is_empty() {
                args.push("--".into());
            }
            args.extend(config.args.iter().cloned());
            package_manager_command(manager, args, tools)?
        }
        RunKind::Test { framework, scope } => {
            let (package, bin) = framework.package();
            let script = find_bin(&cwd, package, bin)?;
            let mut args = vec![script.to_string_lossy().into_owned()];
            args.extend(test_args(*framework, scope));
            args.extend(config.args.iter().cloned());
            if let Some(report) = report {
                let report = report.to_string_lossy().into_owned();
                match framework {
                    TestFramework::Vitest => args.extend([
                        "--reporter=default".to_owned(),
                        "--reporter=json".to_owned(),
                        format!("--outputFile.json={report}"),
                    ]),
                    TestFramework::Jest => args.extend([
                        "--json".to_owned(),
                        "--testLocationInResults".to_owned(),
                        format!("--outputFile={report}"),
                    ]),
                    TestFramework::Playwright => {
                        args.push("--reporter=list,json".to_owned());
                        env.push(("PLAYWRIGHT_JSON_OUTPUT_NAME".into(), report));
                    }
                }
            }
            (node()?, args)
        }
    };
    Ok(CommandLine {
        program,
        args,
        cwd,
        env,
    })
}

/// `<pm> <args>`; on Windows a `.cmd`/`.bat` shim (npm, pnpm and yarn
/// usually are) runs through `cmd.exe`, which interprets some characters,
/// so arguments containing them are refused rather than mangled.
fn package_manager_command(
    manager: PackageManager,
    args: Vec<String>,
    tools: &Tools<'_>,
) -> Result<(PathBuf, Vec<String>), CommandError> {
    let name = manager.program();
    let program = (tools.which)(name).ok_or_else(|| CommandError::ProgramNotFound {
        program: name.into(),
    })?;
    let shim = program
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if tools.os == Os::Windows && shim {
        if let Some(bad) = args
            .iter()
            .find(|a| a.is_empty() || a.chars().any(|c| "&|<>^%!\"() \t".contains(c)))
        {
            return Err(CommandError::UnsafeArgument {
                argument: bad.clone(),
            });
        }
        let mut cmd_args = vec!["/d".to_owned(), "/c".to_owned(), name.to_owned()];
        cmd_args.extend(args);
        return Ok((PathBuf::from("cmd.exe"), cmd_args));
    }
    Ok((program, args))
}

/// The script of `package`'s `bin` entry `bin`, found like Node finds
/// packages: `node_modules/<package>` in `start` or a parent directory.
fn find_bin(start: &Path, package: &str, bin: &str) -> Result<PathBuf, CommandError> {
    for dir in start.ancestors() {
        let package_dir = dir.join("node_modules").join(package);
        let Ok(text) = std::fs::read_to_string(package_dir.join("package.json")) else {
            continue;
        };
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let entry = match json.get("bin") {
            Some(Value::String(path)) => Some(path.as_str()),
            Some(Value::Object(map)) => map.get(bin).and_then(Value::as_str),
            _ => None,
        };
        return entry
            .map(|path| crate::paths::join_native(&package_dir, path))
            .ok_or_else(|| CommandError::NoBin {
                package: package.into(),
                bin: bin.into(),
            });
    }
    Err(CommandError::PackageNotFound {
        package: package.into(),
        cwd: start.to_string_lossy().into_owned(),
    })
}

/// How a framework joins the titles of a full test name when it matches
/// `-t` (verified against Vitest 5: `"math > adds"`; Jest: `"math adds"`).
fn name_separator(framework: TestFramework) -> &'static str {
    match framework {
        TestFramework::Vitest => " > ",
        TestFramework::Jest | TestFramework::Playwright => " ",
    }
}

/// A JavaScript regular expression for `-t` matching these tests or suites
/// (a suite's name also matches every test below it). Placeholders of
/// parameterized titles (`%s`, `$name`) match anything.
pub fn test_name_pattern(names: &[Vec<String>], separator: &str) -> String {
    let below = js_escape(separator);
    let alternatives: Vec<String> = names
        .iter()
        .map(|name| {
            let escaped = js_escape(&name.join(separator));
            let open = match placeholders() {
                Some(re) => re.replace_all(&escaped, ".*").into_owned(),
                None => escaped,
            };
            format!("{open}(?:{below}.*)?")
        })
        .collect();
    format!("^(?:{})$", alternatives.join("|"))
}

fn placeholders() -> Option<&'static regex::Regex> {
    static RE: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| {
        // `%s`, `%d`, ... (printf-style) and `\$name`, `\$\{expr\}` (as
        // escaped by js_escape).
        regex::Regex::new(r"%[sdifjoOpc#%]|\\\$[A-Za-z_][\w.]*|\\\$\\\{[^}]*\\\}").ok()
    });
    RE.as_ref()
}

fn js_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\^$.*+?()[]{}|/-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn test_args(framework: TestFramework, scope: &TestScope) -> Vec<String> {
    match framework {
        TestFramework::Vitest | TestFramework::Jest => {
            let mut args = Vec::new();
            if framework == TestFramework::Vitest {
                args.push("run".to_owned());
            } else if !scope.files.is_empty() || !scope.tests.is_empty() {
                // Exact paths, not path patterns.
                args.push("--runTestsByPath".to_owned());
            }
            let mut files: Vec<String> = scope.files.clone();
            for test in &scope.tests {
                if !files.contains(&test.file) {
                    files.push(test.file.clone());
                }
            }
            args.extend(files);
            if !scope.tests.is_empty() {
                let names: Vec<Vec<String>> = scope.tests.iter().map(|t| t.name.clone()).collect();
                args.push("-t".to_owned());
                args.push(test_name_pattern(&names, name_separator(framework)));
            }
            args
        }
        TestFramework::Playwright => {
            let mut args = vec!["test".to_owned()];
            args.extend(scope.files.iter().cloned());
            for test in &scope.tests {
                args.push(match test.line {
                    Some(line) => format!("{}:{line}", test.file),
                    None => test.file.clone(),
                });
            }
            args
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeMap;

    use super::*;
    use crate::config::TestRef;

    fn config(kind: RunKind) -> RunConfig {
        RunConfig {
            id: "c".into(),
            name: "c".into(),
            kind,
            cwd: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            package_manager: None,
        }
    }

    fn which_all(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/usr/bin/{name}")))
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        // Hoisted install one level up, as in a monorepo.
        write(
            "node_modules/vitest/package.json",
            r#"{ "bin": { "vitest": "./vitest.mjs" } }"#,
        );
        write(
            "node_modules/jest/package.json",
            r#"{ "bin": "./bin/jest.js" }"#,
        );
        write(
            "node_modules/@playwright/test/package.json",
            r#"{ "bin": { "playwright": "cli.js" } }"#,
        );
        write("app/package.json", "{}");
        dir
    }

    #[test]
    fn npm_scripts_use_the_detected_package_manager() {
        let dir = project();
        let tools = Tools {
            which: &which_all,
            os: Os::Unix,
        };
        let mut c = config(RunKind::NpmScript {
            script: "dev".into(),
        });
        c.args = vec!["--port".into(), "4000".into()];
        let line = command_line(&c, dir.path(), None, &tools).unwrap();
        assert_eq!(line.program, PathBuf::from("/usr/bin/npm"));
        assert_eq!(line.args, ["run", "dev", "--", "--port", "4000"]);
        std::fs::write(dir.path().join("bun.lock"), "").unwrap();
        let line = command_line(&c, dir.path(), None, &tools).unwrap();
        assert_eq!(line.program, PathBuf::from("/usr/bin/bun"));
        assert_eq!(line.args, ["run", "dev", "--port", "4000"]);
        c.package_manager = Some(PackageManager::Pnpm);
        c.env.insert("NODE_ENV".into(), "test".into());
        let line = command_line(&c, dir.path(), None, &tools).unwrap();
        assert_eq!(
            line.display(),
            "NODE_ENV=test /usr/bin/pnpm run dev --port 4000"
        );
    }

    #[test]
    fn windows_shims_run_through_cmd_and_refuse_unsafe_arguments() {
        let dir = project();
        let which = |name: &str| Some(PathBuf::from(format!(r"C:\nodejs\{name}.cmd")));
        let tools = Tools {
            which: &which,
            os: Os::Windows,
        };
        let mut c = config(RunKind::NpmScript {
            script: "build".into(),
        });
        let line = command_line(&c, dir.path(), None, &tools).unwrap();
        assert_eq!(line.program, PathBuf::from("cmd.exe"));
        assert_eq!(line.args, ["/d", "/c", "npm", "run", "build"]);
        c.args = vec!["a&b".into()];
        assert!(matches!(
            command_line(&c, dir.path(), None, &tools),
            Err(CommandError::UnsafeArgument { .. })
        ));
    }

    #[test]
    fn test_runs_use_node_with_the_framework_bin_and_a_json_report() {
        let dir = project();
        let tools = Tools {
            which: &which_all,
            os: Os::Unix,
        };
        let report = Path::new("/data/report.json");
        let mut c = config(RunKind::Test {
            framework: TestFramework::Vitest,
            scope: TestScope {
                files: Vec::new(),
                tests: vec![TestRef {
                    file: "src/math.test.ts".into(),
                    name: vec!["math".into(), "adds 1 + 2".into()],
                    line: Some(4),
                }],
            },
        });
        c.cwd = "app".into();
        let line = command_line(&c, dir.path(), Some(report), &tools).unwrap();
        assert_eq!(line.program, PathBuf::from("/usr/bin/node"));
        assert_eq!(line.cwd, dir.path().join("app"));
        // The bin entry is "./vitest.mjs": joined per component, no "./".
        assert_eq!(
            Path::new(&line.args[0]),
            dir.path()
                .join("node_modules")
                .join("vitest")
                .join("vitest.mjs")
        );
        assert_eq!(
            &line.args[1..],
            [
                "run",
                "src/math.test.ts",
                "-t",
                r"^(?:math > adds 1 \+ 2(?: > .*)?)$",
                "--reporter=default",
                "--reporter=json",
                "--outputFile.json=/data/report.json"
            ]
        );

        if let RunKind::Test { framework, .. } = &mut c.kind {
            *framework = TestFramework::Jest;
        }
        let line = command_line(&c, dir.path(), Some(report), &tools).unwrap();
        // Path::ends_with compares components, so this holds on every OS.
        assert!(Path::new(&line.args[0]).ends_with(Path::new("bin").join("jest.js")));
        assert_eq!(
            &line.args[1..4],
            ["--runTestsByPath", "src/math.test.ts", "-t"]
        );
        assert!(line.args.contains(&"--testLocationInResults".to_owned()));

        if let RunKind::Test { framework, .. } = &mut c.kind {
            *framework = TestFramework::Playwright;
        }
        let line = command_line(&c, dir.path(), Some(report), &tools).unwrap();
        assert!(Path::new(&line.args[0]).ends_with("cli.js"));
        assert_eq!(
            &line.args[1..],
            ["test", "src/math.test.ts:4", "--reporter=list,json"]
        );
        assert_eq!(
            line.env,
            [(
                "PLAYWRIGHT_JSON_OUTPUT_NAME".to_owned(),
                "/data/report.json".to_owned()
            )]
        );
    }

    #[test]
    fn missing_tools_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        let tools = Tools {
            which: &none,
            os: Os::Unix,
        };
        let vitest = config(RunKind::Test {
            framework: TestFramework::Vitest,
            scope: TestScope::default(),
        });
        let err = command_line(&vitest, dir.path(), None, &tools).unwrap_err();
        assert!(matches!(err, CommandError::PackageNotFound { .. }), "{err}");
        let node = config(RunKind::Node {
            file: "main.js".into(),
        });
        assert_eq!(
            command_line(&node, dir.path(), None, &tools).unwrap_err(),
            CommandError::ProgramNotFound {
                program: "node".into()
            }
        );
    }

    #[test]
    fn name_patterns_escape_and_keep_placeholders_open() {
        assert_eq!(
            test_name_pattern(&[vec!["a (b)".into(), "c.d".into()], vec!["x".into()]], " "),
            r"^(?:a \(b\) c\.d(?: .*)?|x(?: .*)?)$"
        );
        assert_eq!(
            test_name_pattern(&[vec!["math".into(), "adds %d and $b".into()]], " > "),
            r"^(?:math > adds .* and .*(?: > .*)?)$"
        );
    }
}
