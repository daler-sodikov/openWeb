//! Runs and tests of the open project (`docs/ARCHITECTURE.md`, 4.10): run
//! configurations, trust, starting and stopping runs on PTYs (the terminal
//! session from M4, with its process-tree cleanup), test reports and the
//! gutter's test blocks.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_lang_js::{TestBlockKind, test_blocks};
use ferrule_run_debug::{
    self as rd, NodeKind, RunConfig, RunKind, TestFramework, TestNode, TestReport, TestStatus,
};
use ferrule_terminal::{SpawnSpec, Terminal, TerminalEvent, TerminalOptions, Terminals};

use crate::events::{EventSink, SessionEvent};
use crate::session::{Session, blocking, lock};

/// A run in progress or finished.
struct Entry {
    info: ipc::RunInfo,
    /// The configuration in run-debug's model.
    config: RunConfig,
    cwd: PathBuf,
    started: Instant,
    /// The JSON report a test run writes.
    report: Option<(PathBuf, TestFramework)>,
    /// Test files the run covers (`None`: all), shown as running.
    running_files: Option<Option<Vec<PathBuf>>>,
    /// The parsed report once finished.
    tests: Option<TestReport>,
    /// The user stopped it.
    stopped: bool,
}

#[derive(Default)]
struct State {
    entries: HashMap<u32, Entry>,
    /// Exits reported before the run was registered (a very short program).
    early_exits: HashMap<u32, (Option<i32>, Instant)>,
    /// Last status of each test: project-relative file (`/`) → full name →
    /// status.
    results: HashMap<String, HashMap<Vec<String>, TestStatus>>,
}

/// The project's runs.
pub(crate) struct Runs {
    /// Their PTYs; ids from [`ipc::RUN_ID_BASE`].
    pub terminals: Terminals,
    state: Mutex<State>,
    sink: Arc<dyn EventSink>,
    root: PathBuf,
    /// Where test runs write their reports (Ferrule's data folder).
    reports: PathBuf,
    next_report: AtomicU64,
}

impl Runs {
    pub fn new(
        backend: Option<Arc<dyn ferrule_terminal::Backend>>,
        sink: Arc<dyn EventSink>,
        root: PathBuf,
        reports: PathBuf,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak: &Weak<Runs>| {
            let weak = weak.clone();
            let frames = sink.clone();
            let forward: ferrule_terminal::EventSink = Arc::new(move |event| match event {
                TerminalEvent::Frame(frame) => frames.emit(SessionEvent::TerminalFrame(
                    crate::terminal::frame_to_ipc(&frame),
                )),
                TerminalEvent::Exited { id, code } => {
                    if let Some(runs) = weak.upgrade() {
                        runs.exited(id, code, Instant::now());
                    }
                }
            });
            Self {
                terminals: Terminals::with_first_id(
                    backend,
                    TerminalOptions::default(),
                    forward,
                    ipc::RUN_ID_BASE,
                ),
                state: Mutex::new(State::default()),
                sink,
                root,
                reports,
                next_report: AtomicU64::new(1),
            }
        })
    }

    /// A run's process ended: read its report and tell the view.
    fn exited(&self, id: u32, code: Option<i32>, at: Instant) {
        let entry = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            let Some(entry) = state.entries.get_mut(&id) else {
                state.early_exits.insert(id, (code, at));
                return;
            };
            entry.running_files = None;
            (
                entry.report.take(),
                at.duration_since(entry.started),
                entry.stopped,
            )
        };
        let (report, duration, stopped) = entry;
        let (tests, report_error) = match report {
            None => (None, None),
            Some((path, framework)) => {
                let text = std::fs::read_to_string(&path);
                let _ = std::fs::remove_file(&path);
                match text {
                    Err(_) => (
                        None,
                        Some(format!(
                            "{} wrote no report (it may have failed before running tests)",
                            framework.name()
                        )),
                    ),
                    Ok(text) => {
                        let parsed = match framework {
                            TestFramework::Playwright => rd::parse_playwright_report(&text),
                            TestFramework::Vitest | TestFramework::Jest => {
                                rd::parse_jest_report(&text)
                            }
                        };
                        match parsed {
                            Ok(mut report) => {
                                fill_lines(&mut report);
                                (Some(report), None)
                            }
                            Err(err) => (
                                None,
                                Some(format!(
                                    "the {} report could not be read: {err}",
                                    framework.name()
                                )),
                            ),
                        }
                    }
                }
            }
        };
        let finished = ipc::RunFinished {
            id,
            exit_code: code,
            stopped,
            duration_ms: duration.as_secs_f64() * 1000.0,
            tests: tests.as_ref().map(|r| self.tree(r)),
            counts: tests.as_ref().map(|r| {
                let (p, f, s) = r.counts();
                (to_u32(p), to_u32(f), to_u32(s))
            }),
            report_error,
        };
        if let Ok(mut state) = self.state.lock() {
            if let Some(report) = &tests {
                record_results(&mut state.results, report, &self.root);
            }
            if let Some(entry) = state.entries.get_mut(&id) {
                entry.tests = tests;
            }
        }
        self.sink.emit(SessionEvent::RunFinished(finished));
    }

    /// Registers a started run; reports its exit at once if it already
    /// ended.
    fn register(&self, id: u32, entry: Entry) {
        let early = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.entries.insert(id, entry);
            state.early_exits.remove(&id)
        };
        if let Some((code, at)) = early {
            self.exited(id, code, at);
        }
    }

    /// A report path (data from the tool, `/` separators) for the view:
    /// project-relative when inside the project.
    fn location(&self, file: &str, line: u32, column: u32) -> ipc::TestLocation {
        let relative = rd::paths::relative_to(file, &self.root);
        ipc::TestLocation {
            in_project: relative.is_some(),
            path: relative.unwrap_or_else(|| rd::paths::normalize(file)),
            line,
            column,
        }
    }

    fn tree(&self, report: &TestReport) -> Vec<ipc::TestTreeNode> {
        report.files.iter().map(|f| self.node(f)).collect()
    }

    fn node(&self, node: &TestNode) -> ipc::TestTreeNode {
        let location = self.location(&node.file, node.line.unwrap_or(0), 0);
        ipc::TestTreeNode {
            kind: match node.kind {
                NodeKind::File => ipc::TestNodeKind::File,
                NodeKind::Suite => ipc::TestNodeKind::Suite,
                NodeKind::Test => ipc::TestNodeKind::Test,
            },
            name: if node.kind == NodeKind::File {
                location.path.clone()
            } else {
                node.name.clone()
            },
            full_name: node.full_name.clone(),
            status: status_to_ipc(node.status),
            duration_ms: node.duration_ms,
            location,
            failures: node
                .failures
                .iter()
                .map(|f| ipc::TestFailure {
                    message: f.message.clone(),
                    frames: f
                        .frames
                        .iter()
                        .map(|fr| self.location(&fr.file, fr.line, fr.column))
                        .collect(),
                })
                .collect(),
            children: node.children.iter().map(|c| self.node(c)).collect(),
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn status_to_ipc(status: TestStatus) -> ipc::TestStatus {
    match status {
        TestStatus::Passed => ipc::TestStatus::Passed,
        TestStatus::Failed => ipc::TestStatus::Failed,
        TestStatus::Skipped => ipc::TestStatus::Skipped,
    }
}

/// Reporters without test lines (Vitest): take them from the file's test
/// blocks, so a click on a test opens it.
fn fill_lines(report: &mut TestReport) {
    for file in &mut report.files {
        let Ok(text) = std::fs::read_to_string(&file.file) else {
            continue;
        };
        let lines: HashMap<Vec<String>, u32> = test_blocks(Path::new(&file.file), &text)
            .into_iter()
            .map(|b| (b.name, b.line + 1))
            .collect();
        fn walk(node: &mut TestNode, lines: &HashMap<Vec<String>, u32>) {
            if node.line.is_none() {
                node.line = lines.get(&node.full_name).copied();
            }
            for child in &mut node.children {
                walk(child, lines);
            }
        }
        walk(file, &lines);
    }
}

fn record_results(
    results: &mut HashMap<String, HashMap<Vec<String>, TestStatus>>,
    report: &TestReport,
    root: &Path,
) {
    fn walk(node: &TestNode, out: &mut HashMap<Vec<String>, TestStatus>) {
        if node.kind == NodeKind::Test {
            out.insert(node.full_name.clone(), node.status);
        }
        for child in &node.children {
            walk(child, out);
        }
    }
    for file in &report.files {
        // Files outside the project have no gutter.
        if let Some(rel) = rd::paths::relative_to(&file.file, root) {
            walk(file, results.entry(rel).or_default());
        }
    }
}

/// Whether a reported test belongs to a block: the same name, a test below
/// a suite, or a concrete case of a parameterized title (`adds %i`).
fn covers(block: &[String], test: &[String]) -> bool {
    if test.len() < block.len() {
        return false;
    }
    block.iter().zip(test).all(|(b, t)| {
        if b == t {
            return true;
        }
        let Some(stem) = b.split(['%', '$']).next() else {
            return false;
        };
        (b.contains('%') || b.contains('$')) && t.starts_with(stem)
    })
}

fn block_status(
    block: &[String],
    results: Option<&HashMap<Vec<String>, TestStatus>>,
) -> Option<ipc::TestStatus> {
    let statuses: Vec<TestStatus> = results?
        .iter()
        .filter(|(name, _)| covers(block, name))
        .map(|(_, status)| *status)
        .collect();
    if statuses.is_empty() {
        None
    } else if statuses.contains(&TestStatus::Failed) {
        Some(ipc::TestStatus::Failed)
    } else if statuses.contains(&TestStatus::Passed) {
        Some(ipc::TestStatus::Passed)
    } else {
        Some(ipc::TestStatus::Skipped)
    }
}

// Conversions between the IPC and run-debug models.

fn pm_from_ipc(pm: ipc::PackageManager) -> rd::PackageManager {
    match pm {
        ipc::PackageManager::Bun => rd::PackageManager::Bun,
        ipc::PackageManager::Pnpm => rd::PackageManager::Pnpm,
        ipc::PackageManager::Yarn => rd::PackageManager::Yarn,
        ipc::PackageManager::Npm => rd::PackageManager::Npm,
    }
}

fn pm_to_ipc(pm: rd::PackageManager) -> ipc::PackageManager {
    match pm {
        rd::PackageManager::Bun => ipc::PackageManager::Bun,
        rd::PackageManager::Pnpm => ipc::PackageManager::Pnpm,
        rd::PackageManager::Yarn => ipc::PackageManager::Yarn,
        rd::PackageManager::Npm => ipc::PackageManager::Npm,
    }
}

fn fw_from_ipc(fw: ipc::TestFramework) -> TestFramework {
    match fw {
        ipc::TestFramework::Vitest => TestFramework::Vitest,
        ipc::TestFramework::Jest => TestFramework::Jest,
        ipc::TestFramework::Playwright => TestFramework::Playwright,
    }
}

fn fw_to_ipc(fw: TestFramework) -> ipc::TestFramework {
    match fw {
        TestFramework::Vitest => ipc::TestFramework::Vitest,
        TestFramework::Jest => ipc::TestFramework::Jest,
        TestFramework::Playwright => ipc::TestFramework::Playwright,
    }
}

fn ref_from_ipc(t: ipc::TestRef) -> rd::TestRef {
    rd::TestRef {
        file: t.file,
        name: t.name,
        line: t.line,
    }
}

fn ref_to_ipc(t: rd::TestRef) -> ipc::TestRef {
    ipc::TestRef {
        file: t.file,
        name: t.name,
        line: t.line,
    }
}

fn config_from_ipc(c: ipc::RunConfig) -> RunConfig {
    RunConfig {
        id: c.id,
        name: c.name,
        kind: match c.kind {
            ipc::RunKind::Node { file } => RunKind::Node { file },
            ipc::RunKind::NpmScript { script } => RunKind::NpmScript { script },
            ipc::RunKind::Test { framework, scope } => RunKind::Test {
                framework: fw_from_ipc(framework),
                scope: rd::TestScope {
                    files: scope.files,
                    tests: scope.tests.into_iter().map(ref_from_ipc).collect(),
                },
            },
        },
        cwd: c.cwd,
        args: c.args,
        env: c.env.into_iter().collect::<BTreeMap<_, _>>(),
        package_manager: c.package_manager.map(pm_from_ipc),
    }
}

fn config_to_ipc(c: RunConfig) -> ipc::RunConfig {
    ipc::RunConfig {
        id: c.id,
        name: c.name,
        kind: match c.kind {
            RunKind::Node { file } => ipc::RunKind::Node { file },
            RunKind::NpmScript { script } => ipc::RunKind::NpmScript { script },
            RunKind::Test { framework, scope } => ipc::RunKind::Test {
                framework: fw_to_ipc(framework),
                scope: ipc::TestScope {
                    files: scope.files,
                    tests: scope.tests.into_iter().map(ref_to_ipc).collect(),
                },
            },
        },
        cwd: c.cwd,
        args: c.args,
        env: c.env.into_iter().collect(),
        package_manager: c.package_manager.map(pm_to_ipc),
    }
}

fn config_error(err: rd::ConfigError) -> IpcError {
    IpcError::new(IpcErrorKind::InvalidArgument, err.to_string())
}

/// Whether a file name looks like a test file.
fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.contains(".test.")
        || name.contains(".spec.")
        || path.components().any(|c| c.as_os_str() == "__tests__")
}

/// The framework that runs a test file: Playwright if it imports
/// `@playwright/test`, else the first of Vitest and Jest found in a
/// `package.json` from the file's folder up to the project root.
fn framework_for(root: &Path, file: &Path, text: &str) -> Option<TestFramework> {
    if text.contains("@playwright/test") {
        return Some(TestFramework::Playwright);
    }
    for dir in file.ancestors().skip(1) {
        if let Ok(json) = std::fs::read_to_string(dir.join("package.json")) {
            for config in rd::detect_configs(&json) {
                if let RunKind::Test { framework, .. } = config.kind
                    && framework != TestFramework::Playwright
                {
                    return Some(framework);
                }
            }
        }
        if dir == root {
            break;
        }
    }
    None
}

impl Session {
    fn project_runs(&self) -> Result<Arc<Runs>, IpcError> {
        Ok(self.runtime()?.runs.clone())
    }

    /// Stored and detected configurations, the package manager and trust.
    pub async fn run_configurations(&self) -> Result<ipc::RunConfigurations, IpcError> {
        let runtime = self.runtime()?;
        blocking(move || {
            let root = runtime.project.root().to_path_buf();
            let (stored, error) = match rd::load_configs(&root) {
                Ok(stored) => (stored, None),
                Err(err) => (Vec::new(), Some(err.to_string())),
            };
            let detected = std::fs::read_to_string(root.join("package.json"))
                .map(|json| rd::detect_configs(&json))
                .unwrap_or_default();
            Ok(ipc::RunConfigurations {
                stored: stored.into_iter().map(config_to_ipc).collect(),
                detected: detected.into_iter().map(config_to_ipc).collect(),
                error,
                package_manager: pm_to_ipc(rd::detect_package_manager(&[&root])),
                trusted: runtime.trusted(),
            })
        })
        .await
    }

    /// Stores a configuration (a detected one gets a new id); returns it.
    pub async fn save_run_configuration(
        &self,
        config: ipc::RunConfig,
    ) -> Result<ipc::RunConfig, IpcError> {
        let root = self.runtime()?.project.root().to_path_buf();
        blocking(move || {
            let mut stored = rd::load_configs(&root).map_err(config_error)?;
            let mut config = config_from_ipc(config);
            if config.name.trim().is_empty() {
                return Err(IpcError::new(
                    IpcErrorKind::InvalidArgument,
                    "Enter a name.",
                ));
            }
            if config.id.is_empty() || config.id.starts_with("detected:") {
                let next = stored
                    .iter()
                    .filter_map(|c| c.id.strip_prefix("cfg-")?.parse::<u32>().ok())
                    .max()
                    .unwrap_or(0)
                    + 1;
                config.id = format!("cfg-{next}");
            }
            match stored.iter_mut().find(|c| c.id == config.id) {
                Some(slot) => *slot = config.clone(),
                None => stored.push(config.clone()),
            }
            rd::save_configs(&root, &stored).map_err(config_error)?;
            Ok(config_to_ipc(config))
        })
        .await
    }

    /// Removes a stored configuration.
    pub async fn delete_run_configuration(&self, id: String) -> Result<(), IpcError> {
        let root = self.runtime()?.project.root().to_path_buf();
        blocking(move || {
            let mut stored = rd::load_configs(&root).map_err(config_error)?;
            stored.retain(|c| c.id != id);
            rd::save_configs(&root, &stored).map_err(config_error)
        })
        .await
    }

    /// Trusts the project: runs no longer ask first.
    pub async fn trust_project(&self) -> Result<(), IpcError> {
        let runtime = self.runtime()?;
        blocking(move || {
            runtime.set_trusted(true);
            Ok(())
        })
        .await
    }

    /// Forgets the project's trust: the next run shows its command and
    /// asks again. Runs already going are not stopped.
    pub async fn revoke_project_trust(&self) -> Result<(), IpcError> {
        let runtime = self.runtime()?;
        blocking(move || {
            runtime.set_trusted(false);
            Ok(())
        })
        .await
    }

    /// Starts a run, or says what would run and that the project must be
    /// trusted first. A stored configuration is read again from
    /// `.ferrule/run.json`, so what was shown is what runs.
    pub async fn start_run(
        &self,
        config: ipc::RunConfig,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartRun, IpcError> {
        let runtime = self.runtime()?;
        blocking(move || {
            let root = runtime.project.root().to_path_buf();
            let mut config = config_from_ipc(config);
            if !config.id.starts_with("detected:")
                && !config.id.starts_with("adhoc:")
                && let Ok(stored) = rd::load_configs(&root)
                && let Some(fresh) = stored.into_iter().find(|c| c.id == config.id)
            {
                config = fresh;
            }
            start(&runtime.runs, &root, config, runtime.trusted(), size)
        })
        .await
    }

    /// Stops a run's process tree (its output stays).
    pub async fn stop_run(&self, id: u32) -> Result<(), IpcError> {
        let terminal = self.run_terminal(id)?;
        if let Some(entry) = lock(&self.project_runs()?.state)?.entries.get_mut(&id) {
            entry.stopped = true;
        }
        terminal.stop();
        Ok(())
    }

    /// Closes a run's tab: ends it if running and forgets it.
    pub async fn close_run(&self, id: u32) -> Result<(), IpcError> {
        let runs = self.project_runs()?;
        blocking(move || {
            let _ = runs.terminals.close(id);
            if let Ok(mut state) = runs.state.lock() {
                state.entries.remove(&id);
            }
            Ok(())
        })
        .await
    }

    fn run_terminal(&self, id: u32) -> Result<Arc<Terminal>, IpcError> {
        self.project_runs()?
            .terminals
            .get(id)
            .map_err(|e| IpcError::new(IpcErrorKind::NotFound, e.to_string()))
    }

    fn run_config(&self, id: u32) -> Result<RunConfig, IpcError> {
        let runs = self.project_runs()?;
        let state = lock(&runs.state)?;
        Ok(known(&state, id)?.config.clone())
    }

    /// Runs a run's configuration again (in a new tab).
    pub async fn rerun(&self, id: u32, size: ipc::TerminalSize) -> Result<ipc::StartRun, IpcError> {
        let config = self.run_config(id)?;
        self.start_run(config_to_ipc(config), size).await
    }

    /// Runs the failed tests of a test run again.
    pub async fn rerun_failed(
        &self,
        id: u32,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartRun, IpcError> {
        let runs = self.project_runs()?;
        let failed = {
            let state = lock(&runs.state)?;
            let entry = known(&state, id)?;
            entry
                .tests
                .as_ref()
                .map(|t| t.failed_tests(&entry.cwd))
                .unwrap_or_default()
        };
        if failed.is_empty() {
            return Err(IpcError::new(IpcErrorKind::Conflict, "No failed tests."));
        }
        self.rerun_scope(id, failed, "failed", size).await
    }

    /// Runs some tests of a test run again: files (an empty name), suites
    /// or tests, with paths relative to the project.
    pub async fn rerun_tests(
        &self,
        id: u32,
        tests: Vec<ipc::TestTarget>,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartRun, IpcError> {
        let root = self.runtime()?.project.root().to_path_buf();
        let runs = self.project_runs()?;
        let cwd = known(&*lock(&runs.state)?, id)?.cwd.clone();
        // The run's folder relative to the project ("" for the root).
        let cwd_rel = if rd::paths::normalize(&cwd.to_string_lossy()).trim_end_matches('/')
            == rd::paths::normalize(&root.to_string_lossy()).trim_end_matches('/')
        {
            Some(String::new())
        } else {
            rd::paths::relative_to(&cwd.to_string_lossy(), &root)
        };
        let mut refs = Vec::new();
        for test in tests {
            // Project-relative, never absolute or `..` (as strings: the
            // view's paths use `/` whatever the OS).
            let file = rd::paths::safe_relative(&test.path)
                .zip(cwd_rel.as_deref())
                .and_then(|(rel, dir)| {
                    if dir.is_empty() {
                        Some(rel)
                    } else {
                        rel.strip_prefix(dir)?.strip_prefix('/').map(str::to_owned)
                    }
                })
                .ok_or_else(|| {
                    IpcError::new(
                        IpcErrorKind::InvalidArgument,
                        format!("{} is outside the run's folder", test.path),
                    )
                })?;
            refs.push(rd::TestRef {
                file,
                name: test.name,
                line: None,
            });
        }
        if refs.is_empty() {
            return Err(IpcError::new(IpcErrorKind::InvalidArgument, "No tests."));
        }
        let label = match refs.as_slice() {
            [one] => one.name.last().cloned().unwrap_or_else(|| one.file.clone()),
            _ => "selected".to_owned(),
        };
        self.rerun_scope(id, refs, &label, size).await
    }

    /// Runs a test run's configuration again, narrowed to `refs`
    /// (relative to its working directory).
    async fn rerun_scope(
        &self,
        id: u32,
        refs: Vec<rd::TestRef>,
        label: &str,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartRun, IpcError> {
        let config = self.run_config(id)?;
        let RunKind::Test { framework, .. } = config.kind else {
            return Err(IpcError::new(
                IpcErrorKind::InvalidArgument,
                "not a test run",
            ));
        };
        let base = match config.name.rfind(" (") {
            Some(i) if config.name.ends_with(')') => &config.name[..i],
            _ => config.name.as_str(),
        };
        let mut again = config.clone();
        again.id = format!("adhoc:rerun:{id}");
        again.name = format!("{base} ({label})");
        let (files, tests): (Vec<_>, Vec<_>) = refs.into_iter().partition(|t| t.name.is_empty());
        again.kind = RunKind::Test {
            framework,
            scope: rd::TestScope {
                files: files.into_iter().map(|t| t.file).collect(),
                tests,
            },
        };
        self.start_run(config_to_ipc(again), size).await
    }

    /// Runs the test or suite at zero-based `line` of a document, the whole
    /// test file if `line` is `None` or not in a block, or (not a test
    /// file) the file with Node.
    pub async fn run_at(
        &self,
        doc_id: DocId,
        line: Option<u32>,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartRun, IpcError> {
        let (text, _, rel) = self.doc_text(doc_id)?;
        let root = self.runtime()?.project.root().to_path_buf();
        let path = root.join(&rel);
        let blocks = test_blocks(&path, &text);
        let framework = framework_for(&root, &path, &text);
        let config = match framework {
            Some(framework) if is_test_file(&path) || !blocks.is_empty() => {
                // The innermost block containing the line.
                let block = line.and_then(|line| {
                    blocks
                        .iter()
                        .filter(|b| b.line <= line && line <= b.end_line)
                        .max_by_key(|b| b.line)
                        .cloned()
                });
                let (name, scope) = match &block {
                    Some(b) => (
                        b.name.last().cloned().unwrap_or_default(),
                        rd::TestScope {
                            files: Vec::new(),
                            tests: vec![rd::TestRef {
                                file: rel.clone(),
                                name: b.name.clone(),
                                line: Some(b.line + 1),
                            }],
                        },
                    ),
                    None => (
                        rel.clone(),
                        rd::TestScope {
                            files: vec![rel.clone()],
                            tests: Vec::new(),
                        },
                    ),
                };
                RunConfig {
                    id: format!("adhoc:test:{rel}:{}", line.unwrap_or(0)),
                    name,
                    kind: RunKind::Test { framework, scope },
                    cwd: String::new(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    package_manager: None,
                }
            }
            _ => RunConfig {
                id: format!("adhoc:node:{rel}"),
                name: rel.clone(),
                kind: RunKind::Node { file: rel.clone() },
                cwd: String::new(),
                args: Vec::new(),
                env: BTreeMap::new(),
                package_manager: None,
            },
        };
        self.start_run(config_to_ipc(config), size).await
    }

    /// The test blocks of a document with their last statuses (running
    /// while a run covers the file).
    pub async fn test_blocks(&self, doc_id: DocId) -> Result<ipc::TestBlocks, IpcError> {
        let (text, version, rel) = self.doc_text(doc_id)?;
        let runtime = self.runtime()?;
        blocking(move || {
            let root = runtime.project.root().to_path_buf();
            let path = root.join(&rel);
            // Asked after edits of every document: the cheap check first.
            let framework = (is_test_file(&path) || text.contains("@playwright/test"))
                .then(|| framework_for(&root, &path, &text))
                .flatten();
            if framework.is_none() {
                return Ok(ipc::TestBlocks {
                    version,
                    framework: None,
                    blocks: Vec::new(),
                });
            }
            let state = lock(&runtime.runs.state)?;
            let running = state.entries.values().any(|e| match &e.running_files {
                Some(None) => true,
                Some(Some(files)) => files.iter().any(|f| f == &path),
                None => false,
            });
            let results = state.results.get(&rd::paths::normalize(&rel));
            let blocks = test_blocks(&path, &text)
                .into_iter()
                .map(|b| ipc::TestBlock {
                    line: b.line,
                    kind: match b.kind {
                        TestBlockKind::Suite => ipc::TestBlockKind::Suite,
                        TestBlockKind::Test => ipc::TestBlockKind::Test,
                    },
                    status: if running {
                        Some(ipc::TestStatus::Running)
                    } else {
                        block_status(&b.name, results)
                    },
                    name: b.name,
                })
                .collect();
            Ok(ipc::TestBlocks {
                version,
                framework: framework.map(fw_to_ipc),
                blocks,
            })
        })
        .await
    }

    /// The project's runs, oldest first.
    pub async fn runs(&self) -> Result<Vec<ipc::RunInfo>, IpcError> {
        let runs = self.project_runs()?;
        let state = lock(&runs.state)?;
        let mut infos: Vec<ipc::RunInfo> = state.entries.values().map(|e| e.info.clone()).collect();
        infos.sort_by_key(|i| i.id);
        Ok(infos)
    }
}

fn known(state: &State, id: u32) -> Result<&Entry, IpcError> {
    state
        .entries
        .get(&id)
        .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, format!("run {id} is not known")))
}

/// Builds the command and, if trusted, starts it. Blocking.
fn start(
    runs: &Runs,
    root: &Path,
    config: RunConfig,
    trusted: bool,
    size: ipc::TerminalSize,
) -> Result<ipc::StartRun, IpcError> {
    let framework = match &config.kind {
        RunKind::Test { framework, .. } => Some(*framework),
        _ => None,
    };
    let report = framework.map(|_| {
        let n = runs.next_report.fetch_add(1, Ordering::Relaxed);
        runs.reports
            .join(format!("report-{}-{n}.json", std::process::id()))
    });
    let tools = rd::Tools {
        which: &ferrule_lsp_client::find_on_path,
        os: rd::Os::current(),
    };
    let command = rd::command_line(&config, root, report.as_deref(), &tools)
        .map_err(|err| IpcError::new(IpcErrorKind::InvalidArgument, err.to_string()))?;
    let shown = command.display();
    if !trusted {
        return Ok(ipc::StartRun::NeedsTrust {
            command: shown,
            cwd: command.cwd.to_string_lossy().into_owned(),
        });
    }
    if let Some(dir) = report.as_deref().and_then(Path::parent) {
        std::fs::create_dir_all(dir).map_err(|e| IpcError::new(IpcErrorKind::Io, e.to_string()))?;
    }
    let mut env = ferrule_terminal::terminal_env();
    env.extend(command.env.iter().cloned());
    let spec = SpawnSpec {
        program: command.program.clone(),
        args: command.args.clone(),
        cwd: command.cwd.clone(),
        env,
    };
    let running_files = match &config.kind {
        RunKind::Test { scope, .. } => {
            let mut files: HashSet<PathBuf> =
                scope.files.iter().map(|f| command.cwd.join(f)).collect();
            files.extend(scope.tests.iter().map(|t| command.cwd.join(&t.file)));
            Some(if files.is_empty() {
                None
            } else {
                Some(files.into_iter().collect())
            })
        }
        _ => None,
    };
    let started = Instant::now();
    let opened = runs
        .terminals
        .open(
            &spec,
            ferrule_terminal::TermSize::new(
                u16::try_from(size.cols).unwrap_or(u16::MAX),
                u16::try_from(size.rows).unwrap_or(u16::MAX),
            ),
        )
        .map_err(|e| IpcError::new(IpcErrorKind::Io, e.to_string()))?;
    tracing::info!(command = %shown, id = opened.id, "run started");
    let info = ipc::RunInfo {
        id: opened.id,
        name: config.name.clone(),
        command: shown,
        config: config_to_ipc(config.clone()),
        tests: framework.is_some(),
    };
    runs.register(
        opened.id,
        Entry {
            info: info.clone(),
            config,
            cwd: command.cwd,
            started,
            report: report.zip(framework),
            running_files,
            tests: None,
            stopped: false,
        },
    );
    Ok(ipc::StartRun::Started { run: info })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn blocks_cover_their_tests_and_parameterized_cases() {
        assert!(covers(&name(&["math", "adds"]), &name(&["math", "adds"])));
        assert!(covers(&name(&["math"]), &name(&["math", "adds"])));
        assert!(!covers(&name(&["math", "adds"]), &name(&["math"])));
        assert!(covers(
            &name(&["math", "tables", "adds %i and %i"]),
            &name(&["math", "tables", "adds 2 and 3"])
        ));
        assert!(!covers(&name(&["math", "x"]), &name(&["math", "y"])));
        let mut results = HashMap::new();
        results.insert(name(&["math", "adds"]), TestStatus::Passed);
        results.insert(name(&["math", "divides"]), TestStatus::Failed);
        assert_eq!(
            block_status(&name(&["math"]), Some(&results)),
            Some(ipc::TestStatus::Failed)
        );
        assert_eq!(
            block_status(&name(&["math", "adds"]), Some(&results)),
            Some(ipc::TestStatus::Passed)
        );
        assert_eq!(block_status(&name(&["other"]), Some(&results)), None);
    }

    #[test]
    fn test_files_by_name() {
        assert!(is_test_file(Path::new("src/a.test.ts")));
        assert!(is_test_file(Path::new("e2e/a.spec.tsx")));
        assert!(is_test_file(Path::new("src/__tests__/a.ts")));
        assert!(!is_test_file(Path::new("src/main.ts")));
    }
}
