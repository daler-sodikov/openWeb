//! Test reports: the JSON of Vitest and Jest (one format) and of
//! Playwright, parsed into one tree of files, suites and tests.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::config::TestRef;
use crate::paths;

/// A test's (or a group's) outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    /// Passed.
    Passed,
    /// Failed (or its file failed to run).
    Failed,
    /// Skipped, pending or todo.
    Skipped,
}

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A test file.
    File,
    /// A `describe` block.
    Suite,
    /// A test.
    Test,
}

/// A stack frame of a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    /// The file, absolute, with `/` separators (see [`crate::paths`]).
    pub file: String,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
}

/// A failure message with the stack frames found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The message, without terminal colors.
    pub message: String,
    /// Frames, innermost first.
    pub frames: Vec<StackFrame>,
}

/// A file, suite or test.
#[derive(Debug, Clone, PartialEq)]
pub struct TestNode {
    /// What it is.
    pub kind: NodeKind,
    /// Its title (the file path for files).
    pub name: String,
    /// Titles from the outermost suite down to this node (empty for files).
    pub full_name: Vec<String>,
    /// Outcome (for groups: failed if anything failed, passed if anything
    /// passed, else skipped).
    pub status: TestStatus,
    /// Milliseconds, if reported.
    pub duration_ms: Option<f64>,
    /// The file it is in (absolute, with `/` separators).
    pub file: String,
    /// One-based line, if reported.
    pub line: Option<u32>,
    /// Failures (a test's assertion errors, or a file that failed to run).
    pub failures: Vec<Failure>,
    /// Suites and tests inside.
    pub children: Vec<TestNode>,
}

/// A parsed report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TestReport {
    /// One node per test file.
    pub files: Vec<TestNode>,
}

impl TestReport {
    /// Passed, failed and skipped test counts.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        fn walk(node: &TestNode, counts: &mut (usize, usize, usize)) {
            if node.kind == NodeKind::Test {
                match node.status {
                    TestStatus::Passed => counts.0 += 1,
                    TestStatus::Failed => counts.1 += 1,
                    TestStatus::Skipped => counts.2 += 1,
                }
            }
            for child in &node.children {
                walk(child, counts);
            }
        }
        for file in &self.files {
            walk(file, &mut counts);
        }
        counts
    }

    /// The failed tests, to run them again: file paths relative to `cwd`
    /// when they are inside it. A file that failed without any test result
    /// is run whole (its `name` is empty).
    pub fn failed_tests(&self, cwd: &Path) -> Vec<TestRef> {
        fn walk(node: &TestNode, cwd: &Path, out: &mut Vec<TestRef>) {
            if node.kind == NodeKind::Test && node.status == TestStatus::Failed {
                out.push(TestRef {
                    file: relative(&node.file, cwd),
                    name: node.full_name.clone(),
                    line: node.line,
                });
            }
            for child in &node.children {
                walk(child, cwd, out);
            }
        }
        let mut out = Vec::new();
        for file in &self.files {
            if file.status == TestStatus::Failed && file.children.is_empty() {
                out.push(TestRef {
                    file: relative(&file.file, cwd),
                    name: Vec::new(),
                    line: None,
                });
            }
            walk(file, cwd, &mut out);
        }
        out
    }
}

fn relative(file: &str, cwd: &Path) -> String {
    paths::relative_to(file, cwd).unwrap_or_else(|| paths::normalize(file))
}

static ANSI: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").ok());

// `at fn (path:line:col)`, `at path:line:col`; the path may be a
// `file://` URL, Unix (`/x`), Windows (`C:\x`, `C:/x`) or UNC (`\\s\x`).
static FRAME: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^\s*at (?:.*? \()?((?:file://)?/?(?:[A-Za-z]:)?[^():]+):(\d+):(\d+)\)?\s*$").ok()
});

fn strip_ansi(text: &str) -> String {
    match ANSI.as_ref() {
        Some(re) => re.replace_all(text, "").into_owned(),
        None => text.to_owned(),
    }
}

/// A failure from a message whose stack lines may be in it, or in
/// `stack` (Playwright reports both).
fn failure(message: &str, stack: Option<&str>) -> Failure {
    let message = strip_ansi(message);
    let source = stack.map_or_else(|| message.clone(), strip_ansi);
    let frames = source
        .lines()
        .filter_map(|line| {
            let caps = FRAME.as_ref()?.captures(line)?;
            Some(StackFrame {
                file: paths::normalize(caps.get(1)?.as_str()),
                line: caps.get(2)?.as_str().parse().ok()?,
                column: caps.get(3)?.as_str().parse().ok()?,
            })
        })
        .collect();
    Failure {
        message: message.trim_end().to_owned(),
        frames,
    }
}

fn group_status(children: &[TestNode]) -> TestStatus {
    if children.iter().any(|c| c.status == TestStatus::Failed) {
        TestStatus::Failed
    } else if children.iter().any(|c| c.status == TestStatus::Passed) {
        TestStatus::Passed
    } else {
        TestStatus::Skipped
    }
}

fn group_duration(children: &[TestNode]) -> Option<f64> {
    let durations: Vec<f64> = children.iter().filter_map(|c| c.duration_ms).collect();
    (!durations.is_empty()).then(|| durations.iter().sum())
}

/// Fills in the statuses and durations of groups from their children.
fn settle(node: &mut TestNode) {
    for child in &mut node.children {
        settle(child);
    }
    if node.kind != NodeKind::Test && !node.children.is_empty() {
        node.status = if node.failures.is_empty() {
            group_status(&node.children)
        } else {
            TestStatus::Failed
        };
        node.duration_ms = group_duration(&node.children);
    }
}

fn group(kind: NodeKind, name: &str, full_name: Vec<String>, file: &str) -> TestNode {
    TestNode {
        kind,
        name: name.to_owned(),
        full_name,
        status: TestStatus::Skipped,
        duration_ms: None,
        file: file.to_owned(),
        line: None,
        failures: Vec::new(),
        children: Vec::new(),
    }
}

/// The suite with `titles` below `node`, created as needed.
fn suite<'a>(node: &'a mut TestNode, titles: &[String], depth: usize) -> &'a mut TestNode {
    let Some(title) = titles.get(depth) else {
        return node;
    };
    let index = match node
        .children
        .iter()
        .position(|c| c.kind == NodeKind::Suite && &c.name == title)
    {
        Some(index) => index,
        None => {
            let file = node.file.clone();
            node.children.push(group(
                NodeKind::Suite,
                title,
                titles[..=depth].to_vec(),
                &file,
            ));
            node.children.len() - 1
        }
    };
    suite(&mut node.children[index], titles, depth + 1)
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Parses the JSON of Jest (`--json`) or Vitest (`--reporter=json`, the
/// same format).
pub fn parse_jest_report(json: &str) -> Result<TestReport, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let results = root
        .get("testResults")
        .and_then(Value::as_array)
        .ok_or("no testResults in the report")?;
    let mut files = Vec::new();
    for result in results {
        let path = &paths::normalize(str_of(result, "name"));
        let mut file = group(NodeKind::File, path, Vec::new(), path);
        let message = str_of(result, "message");
        let assertions = result
            .get("assertionResults")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !message.trim().is_empty() && assertions.is_empty() {
            file.failures.push(failure(message, None));
            file.status = TestStatus::Failed;
        }
        for assertion in assertions {
            let ancestors: Vec<String> = assertion
                .get("ancestorTitles")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let title = str_of(assertion, "title");
            let mut full_name = ancestors.clone();
            full_name.push(title.to_owned());
            let status = match str_of(assertion, "status") {
                "passed" => TestStatus::Passed,
                "failed" => TestStatus::Failed,
                _ => TestStatus::Skipped,
            };
            let failures = assertion
                .get("failureMessages")
                .and_then(Value::as_array)
                .map(|m| {
                    m.iter()
                        .filter_map(Value::as_str)
                        .map(|text| failure(text, None))
                        .collect()
                })
                .unwrap_or_default();
            let test = TestNode {
                kind: NodeKind::Test,
                name: title.to_owned(),
                full_name,
                status,
                duration_ms: assertion.get("duration").and_then(Value::as_f64),
                file: path.to_owned(),
                line: assertion
                    .get("location")
                    .and_then(|l| l.get("line"))
                    .and_then(Value::as_u64)
                    .and_then(|l| u32::try_from(l).ok()),
                failures,
                children: Vec::new(),
            };
            suite(&mut file, &ancestors, 0).children.push(test);
        }
        settle(&mut file);
        files.push(file);
    }
    Ok(TestReport { files })
}

/// Parses Playwright's JSON report (`--reporter=json`). File paths in it
/// are relative to `config.rootDir` (the test directory).
pub fn parse_playwright_report(json: &str) -> Result<TestReport, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    // Data from the tool's OS: joined as strings, never with the host's
    // Path (which would insert `\` on Windows into a Linux report).
    let root_dir = paths::normalize(
        root.get("config")
            .map(|c| str_of(c, "rootDir"))
            .unwrap_or_default(),
    );
    let absolute = |file: &str| {
        let file = paths::normalize(file);
        if root_dir.is_empty() || paths::is_absolute(&file) {
            file
        } else {
            paths::join(&root_dir, &file)
        }
    };
    let suites = root
        .get("suites")
        .and_then(Value::as_array)
        .ok_or("no suites in the report")?;
    // Top-level suites are files; several projects repeat a file.
    let mut files: BTreeMap<String, TestNode> = BTreeMap::new();
    for top in suites {
        let path = absolute(str_of(top, "file"));
        let file = files
            .entry(path.clone())
            .or_insert_with(|| group(NodeKind::File, &path, Vec::new(), &path));
        add_playwright_suite(file, top, &[], &path);
    }
    let mut files: Vec<TestNode> = files.into_values().collect();
    for file in &mut files {
        settle(file);
    }
    Ok(TestReport { files })
}

fn line_of(value: &Value) -> Option<u32> {
    value
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|l| u32::try_from(l).ok())
}

/// Adds the specs and nested suites of a Playwright suite under `parent`;
/// `titles` are the enclosing suite titles (not the file).
fn add_playwright_suite(parent: &mut TestNode, suite_json: &Value, titles: &[String], path: &str) {
    for spec in suite_json
        .get("specs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let title = str_of(spec, "title");
        let mut full_name = titles.to_vec();
        full_name.push(title.to_owned());
        let tests = spec
            .get("tests")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let statuses: Vec<&str> = tests.iter().map(|t| str_of(t, "status")).collect();
        let status = if statuses.contains(&"unexpected") {
            TestStatus::Failed
        } else if !statuses.is_empty() && statuses.iter().all(|s| *s == "skipped") {
            TestStatus::Skipped
        } else {
            TestStatus::Passed
        };
        let mut duration = None;
        let mut failures = Vec::new();
        for test in tests {
            let results = test
                .get("results")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let Some(last) = results.last() else {
                continue;
            };
            if let Some(ms) = last.get("duration").and_then(Value::as_f64) {
                duration = Some(duration.unwrap_or(0.0) + ms);
            }
            if str_of(test, "status") != "unexpected" {
                continue;
            }
            let project = str_of(test, "projectName");
            let prefix = if project.is_empty() {
                String::new()
            } else {
                format!("[{project}] ")
            };
            for error in last
                .get("errors")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let mut f = failure(
                    &format!("{prefix}{}", str_of(error, "message")),
                    error.get("stack").and_then(Value::as_str),
                );
                // The stack repeats the message; frames come from it.
                if f.frames.is_empty() {
                    f.frames = failure(str_of(error, "message"), None).frames;
                }
                failures.push(f);
            }
        }
        let parent_file = parent.file.clone();
        let node = TestNode {
            kind: NodeKind::Test,
            name: title.to_owned(),
            full_name,
            status,
            duration_ms: duration,
            file: parent_file,
            line: line_of(spec),
            failures,
            children: Vec::new(),
        };
        parent.children.push(node);
    }
    for child in suite_json
        .get("suites")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let title = str_of(child, "title");
        let mut names = titles.to_vec();
        names.push(title.to_owned());
        let index = match parent
            .children
            .iter()
            .position(|c| c.kind == NodeKind::Suite && c.name == title)
        {
            Some(index) => index,
            None => {
                let mut node = group(NodeKind::Suite, title, names.clone(), path);
                node.line = line_of(child);
                parent.children.push(node);
                parent.children.len() - 1
            }
        };
        add_playwright_suite(&mut parent.children[index], child, &names, path);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read_to_string(path)
            .unwrap()
            .replace("__ROOT__", "/work/p")
            .replace("__REPO__", "/work")
            .replace("__NODE__", "/usr/bin/node")
    }

    fn names(node: &TestNode) -> Vec<String> {
        node.children.iter().map(|c| c.name.clone()).collect()
    }

    #[test]
    fn vitest_report_recorded_from_the_real_tool() {
        let report = parse_jest_report(&fixture("vitest.json")).unwrap();
        assert_eq!(report.files.len(), 1);
        let file = &report.files[0];
        assert_eq!(file.name, "/work/p/src/math.test.ts");
        assert_eq!(file.status, TestStatus::Failed);
        assert_eq!(names(file), ["math"]);
        let math = &file.children[0];
        assert_eq!(
            names(math),
            ["adds", "divides", "is skipped", "tables"],
            "suites come after the tests that preceded them"
        );
        let divides = &math.children[1];
        assert_eq!(divides.status, TestStatus::Failed);
        assert_eq!(divides.full_name, ["math", "divides"]);
        assert!(
            divides.failures[0]
                .message
                .starts_with("AssertionError: expected 0.25 to be 0.5")
        );
        assert_eq!(
            divides.failures[0].frames[0],
            StackFrame {
                file: "/work/p/src/math.test.ts".into(),
                line: 12,
                column: 26
            }
        );
        assert_eq!(math.children[2].status, TestStatus::Skipped);
        let tables = &math.children[3];
        assert_eq!(tables.kind, NodeKind::Suite);
        assert_eq!(tables.status, TestStatus::Passed);
        assert_eq!(
            tables.children[1].full_name,
            ["math", "tables", "adds 2 and 3"]
        );
        assert_eq!(report.counts(), (3, 1, 1));
        assert_eq!(
            report.failed_tests(Path::new("/work/p")),
            [TestRef {
                file: "src/math.test.ts".into(),
                name: vec!["math".into(), "divides".into()],
                line: None
            }]
        );
    }

    #[test]
    fn jest_report_written_from_the_documented_format() {
        let report = parse_jest_report(&fixture("jest.handwritten.json")).unwrap();
        assert_eq!(report.files.len(), 2);
        let math = &report.files[0].children[0];
        assert_eq!(math.children[1].line, Some(10));
        let message = &math.children[1].failures[0].message;
        assert!(
            !message.contains('\u{1b}'),
            "colors are stripped: {message:?}"
        );
        assert!(message.contains("Received: 0.25"), "{message}");
        assert_eq!(math.children[1].failures[0].frames[0].line, 12);
        assert_eq!(math.children[2].status, TestStatus::Skipped, "pending");
        // A file that failed to run: no tests, the error on the file.
        let broken = &report.files[1];
        assert_eq!(broken.status, TestStatus::Failed);
        assert!(
            broken.failures[0]
                .message
                .contains("Test suite failed to run")
        );
        let failed = report.failed_tests(Path::new("/work/p"));
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0].file, "src/math.test.ts");
        assert_eq!(failed[1].file, "src/broken.test.ts");
        assert!(failed[1].name.is_empty(), "the whole file");
    }

    #[test]
    fn jest_report_recorded_from_the_real_tool() {
        let report = parse_jest_report(&fixture("jest.json")).unwrap();
        assert_eq!(report.files.len(), 1);
        let file = &report.files[0];
        assert_eq!(file.name, "/work/p/jest/math.test.cjs");
        assert_eq!(file.status, TestStatus::Failed);
        assert_eq!(names(file), ["math"]);
        let math = &file.children[0];
        assert_eq!(names(math), ["adds", "divides", "is skipped", "tables"]);
        let divides = &math.children[1];
        assert_eq!(divides.status, TestStatus::Failed);
        assert_eq!(divides.full_name, ["math", "divides"]);
        assert_eq!(divides.line, Some(8));
        let failure = &divides.failures[0];
        assert!(
            failure
                .message
                .starts_with("Error: expect(received).toBe(expected)"),
            "{:?}",
            failure.message
        );
        assert!(failure.message.contains("Received: 0.25"));
        // Jest's own frames (jest-circus) follow the test's; the first is the test's.
        assert_eq!(
            failure.frames[0],
            StackFrame {
                file: "/work/p/jest/math.test.cjs".into(),
                line: 10,
                column: 26
            }
        );
        assert_eq!(math.children[2].status, TestStatus::Skipped, "pending");
        let tables = &math.children[3];
        assert_eq!(tables.kind, NodeKind::Suite);
        assert_eq!(
            tables.children[1].full_name,
            ["math", "tables", "adds 2 and 3"]
        );
        assert_eq!(report.counts(), (3, 1, 1));
        assert_eq!(
            report.failed_tests(Path::new("/work/p")),
            [TestRef {
                file: "jest/math.test.cjs".into(),
                name: vec!["math".into(), "divides".into()],
                line: Some(8)
            }]
        );
    }

    #[test]
    fn playwright_report_recorded_from_the_real_tool() {
        let report = parse_playwright_report(&fixture("playwright.json")).unwrap();
        assert_eq!(report.files.len(), 1);
        let file = &report.files[0];
        assert_eq!(file.name, "/work/p/e2e/math.spec.ts", "relative to rootDir");
        let suite = &file.children[0];
        assert_eq!(suite.name, "e2e math");
        assert_eq!(suite.line, Some(3));
        assert_eq!(
            names(suite),
            ["adds without a browser", "fails without a browser"]
        );
        let failed = &suite.children[1];
        assert_eq!(failed.status, TestStatus::Failed);
        assert_eq!(failed.line, Some(8));
        assert_eq!(failed.full_name, ["e2e math", "fails without a browser"]);
        assert!(!failed.failures[0].message.contains('\u{1b}'));
        assert!(
            failed.failures[0].message.contains("toBe"),
            "{:?}",
            failed.failures[0]
        );
        assert_eq!(
            failed.failures[0].frames[0],
            StackFrame {
                file: "/work/p/e2e/math.spec.ts".into(),
                line: 10,
                column: 19
            }
        );
        assert_eq!(report.counts(), (1, 1, 0));
        assert_eq!(
            report.failed_tests(Path::new("/work/p")),
            [TestRef {
                file: "e2e/math.spec.ts".into(),
                name: vec!["e2e math".into(), "fails without a browser".into()],
                line: Some(8)
            }]
        );
    }

    /// Reports written on Windows parse the same on every host: native
    /// file names with drive letters, stack frames with `\`, `file://`
    /// URLs and UNC paths (the Linux CI would have caught the Windows
    /// separator bug with this).
    #[test]
    fn windows_reports_parse_on_every_os() {
        let jest = serde_json::json!({
            "testResults": [{
                "name": r"C:\work\p\src\a.test.ts",
                "message": "",
                "assertionResults": [{
                    "ancestorTitles": ["math"],
                    "title": "divides",
                    "status": "failed",
                    "failureMessages": [concat!(
                        "Error: expected 1\n",
                        r"    at Object.<anonymous> (C:\work\p\src\a.test.ts:12:26)", "\n",
                        "    at file:///C:/work/p/node_modules/vitest/dist/run.js:5:7\n",
                        r"    at \\server\share\lib\x.js:3:4",
                    )],
                }],
            }],
        });
        let report = parse_jest_report(&jest.to_string()).unwrap();
        let file = &report.files[0];
        assert_eq!(file.file, "C:/work/p/src/a.test.ts");
        let frames: Vec<(&str, u32, u32)> = file.children[0].children[0].failures[0]
            .frames
            .iter()
            .map(|f| (f.file.as_str(), f.line, f.column))
            .collect();
        assert_eq!(
            frames,
            [
                ("C:/work/p/src/a.test.ts", 12, 26),
                ("C:/work/p/node_modules/vitest/dist/run.js", 5, 7),
                ("//server/share/lib/x.js", 3, 4),
            ]
        );
        // The project folder as Windows prints it; the drive letter's case
        // may differ from the report's.
        let failed = report.failed_tests(Path::new(r"c:\work\p"));
        assert_eq!(failed[0].file, "src/a.test.ts");

        let playwright = serde_json::json!({
            "config": { "rootDir": "C:/work/p/e2e" },
            "suites": [{
                "title": r"sub\math.spec.ts",
                "file": "sub/math.spec.ts",
                "specs": [{
                    "title": "fails",
                    "line": 3,
                    "tests": [{ "results": [{
                        "status": "failed",
                        "errors": [{
                            "message": "Error: no",
                            "stack": r"Error: no
        at C:\work\p\e2e\sub\math.spec.ts:4:9",
                        }],
                    }], "status": "unexpected" }],
                }],
            }],
        });
        let report = parse_playwright_report(&playwright.to_string()).unwrap();
        assert_eq!(report.files[0].file, "C:/work/p/e2e/sub/math.spec.ts");
        let failed = report.failed_tests(Path::new(r"C:\work\p"));
        assert_eq!(failed[0].file, "e2e/sub/math.spec.ts");
    }

    #[test]
    fn bad_reports_are_errors() {
        assert!(parse_jest_report("{}").is_err());
        assert!(parse_playwright_report("[").is_err());
    }
}
