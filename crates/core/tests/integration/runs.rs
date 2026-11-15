//! Runs and the test runner. Most tests use the in-memory run backend from
//! `ferrule-run-debug` (recorded reports, no real tools); the real-process
//! tests start Node, Vitest and Playwright directly (never through a
//! shell) and skip with a message when they are not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    IpcErrorKind, PackageManager, RunConfig, RunFinished, RunInfo, RunKind, StartRun,
    TerminalFrame, TerminalSize, TestFramework, TestStatus, TestTarget, TestTreeNode,
};
use ferrule_run_debug::fake::FakeRunBackend;

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn finished(&self, id: u32) -> Option<RunFinished> {
        self.0.lock().unwrap().iter().find_map(|e| match e {
            SessionEvent::RunFinished(f) if f.id == id => Some(f.clone()),
            _ => None,
        })
    }

    /// Every row text the run's frames showed.
    fn output(&self, id: u32) -> String {
        let frames: Vec<TerminalFrame> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::TerminalFrame(f) if f.id == id => Some(f.clone()),
                _ => None,
            })
            .collect();
        let mut out = String::new();
        for frame in frames {
            for row in &frame.rows {
                out.extend(row.runs.iter().map(|r| r.text.as_str()));
                out.push('\n');
            }
        }
        out
    }
}

const SIZE: TerminalSize = TerminalSize {
    cols: 120,
    rows: 30,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "node_modules" || name == "test-results" {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(&name));
        } else {
            fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

fn node() -> Option<PathBuf> {
    let found = ferrule_lsp_client::find_on_path("node");
    if found.is_none() {
        eprintln!("skipped: node is not on PATH");
    }
    found
}

/// Packages whose `bin` entries the command lines need; the fake backend
/// never starts them.
fn fake_packages(root: &Path) {
    for (package, bin, script) in [
        ("vitest", "vitest", "vitest.mjs"),
        ("@playwright/test", "playwright", "cli.js"),
    ] {
        let dir = root.join("node_modules").join(package);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("package.json"),
            format!(r#"{{ "name": "{package}", "bin": {{ "{bin}": "{script}" }} }}"#),
        )
        .unwrap();
        fs::write(dir.join(script), "").unwrap();
    }
}

/// A copy of `fixtures/test-runners` at `<dir>/p`, opened with `backend`
/// running its runs.
async fn open(
    dir: &Path,
    backend: Arc<dyn ferrule_terminal::Backend>,
) -> (Arc<Session>, Arc<Events>, PathBuf) {
    let root = dir.join("p");
    copy_dir(&repo().join("fixtures/test-runners"), &root);
    let events = Arc::new(Events::default());
    let mut config = SessionConfig::new(dir.join("data"));
    config.run_backend = Some(backend);
    let s = Session::with_config(config, events.clone()).unwrap();
    s.open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    (s, events, root)
}

fn started(start: StartRun) -> RunInfo {
    match start {
        StartRun::Started { run } => run,
        other => panic!("expected a started run, got {other:?}"),
    }
}

async fn wait_finished(events: &Events, id: u32, timeout: Duration) -> RunFinished {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(finished) = events.finished(id) {
            return finished;
        }
        assert!(
            Instant::now() < deadline,
            "run {id} did not finish; output:\n{}",
            events.output(id)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_output(events: &Events, id: u32, needle: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !events.output(id).contains(needle) {
        assert!(
            Instant::now() < deadline,
            "run {id} never printed {needle:?}; output:\n{}",
            events.output(id)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn find<'a>(nodes: &'a [TestTreeNode], full_name: &[&str]) -> Option<&'a TestTreeNode> {
    nodes.iter().find_map(|n| {
        if n.full_name == full_name {
            Some(n)
        } else {
            find(&n.children, full_name)
        }
    })
}

fn detected(s: &[RunConfig], id: &str) -> RunConfig {
    s.iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no {id} in {s:?}"))
        .clone()
}

#[tokio::test]
async fn nothing_runs_before_the_project_is_trusted() {
    let Some(_) = node() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (s, events, root) = open(dir.path(), Arc::new(FakeRunBackend)).await;
    fake_packages(&root);

    let configs = s.run_configurations().await.unwrap();
    assert!(!configs.trusted);
    assert!(configs.stored.is_empty());
    assert_eq!(configs.package_manager, PackageManager::Npm);
    let names: Vec<&str> = configs.detected.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"start"), "{names:?}");
    assert!(names.contains(&"test"), "{names:?}");
    let vitest = detected(&configs.detected, "detected:test:vitest");
    detected(&configs.detected, "detected:test:playwright");

    match s.start_run(vitest.clone(), SIZE).await.unwrap() {
        StartRun::NeedsTrust { command, cwd } => {
            assert!(command.contains("vitest.mjs"), "{command}");
            assert!(command.contains("--reporter=json"), "{command}");
            assert_eq!(
                fs::canonicalize(cwd).unwrap(),
                fs::canonicalize(&root).unwrap()
            );
        }
        other => panic!("ran without trust: {other:?}"),
    }
    assert!(s.runs().await.unwrap().is_empty(), "nothing may start");

    s.trust_project().await.unwrap();
    assert!(s.run_configurations().await.unwrap().trusted);
    let run = started(s.start_run(vitest, SIZE).await.unwrap());
    let finished = wait_finished(&events, run.id, Duration::from_secs(10)).await;
    assert_eq!(finished.exit_code, Some(1));
    assert_eq!(finished.counts, Some((3, 1, 1)));
    assert!(
        !root.join(".ferrule").exists(),
        "trust is kept outside the project"
    );

    // Trust is remembered for the project.
    let again = Session::with_config(
        SessionConfig::new(dir.path().join("data")),
        Arc::new(Events::default()),
    )
    .unwrap();
    again
        .open_project(root.to_string_lossy().into_owned())
        .await
        .unwrap();
    assert!(again.run_configurations().await.unwrap().trusted);
}

#[tokio::test]
async fn configurations_are_stored_in_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, root) = open(dir.path(), Arc::new(FakeRunBackend)).await;
    let configs = s.run_configurations().await.unwrap();
    let start = detected(&configs.detected, "detected:script:start");

    let first = s.save_run_configuration(start.clone()).await.unwrap();
    assert_eq!(first.id, "cfg-1");
    let file = fs::read_to_string(root.join(".ferrule/run.json")).unwrap();
    assert!(file.contains("\"start\""), "{file}");

    let mut second = start.clone();
    second.id = String::new();
    second.name = "Start with args".into();
    second.args = vec!["--port".into(), "3000".into()];
    second.env = vec![("DEBUG".into(), "1".into())];
    let second = s.save_run_configuration(second).await.unwrap();
    assert_eq!(second.id, "cfg-2");

    let mut renamed = first.clone();
    renamed.name = "Start app".into();
    s.save_run_configuration(renamed).await.unwrap();
    let stored = s.run_configurations().await.unwrap().stored;
    assert_eq!(stored.len(), 2);
    let names: Vec<&str> = stored.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"Start app"), "{names:?}");
    let with_args = stored.iter().find(|c| c.id == "cfg-2").unwrap();
    assert_eq!(with_args.args, ["--port", "3000"]);
    assert_eq!(with_args.env, [("DEBUG".to_owned(), "1".to_owned())]);

    s.delete_run_configuration("cfg-2".into()).await.unwrap();
    assert_eq!(s.run_configurations().await.unwrap().stored.len(), 1);

    let mut nameless = start;
    nameless.name = "  ".into();
    let err = s.save_run_configuration(nameless).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);

    fs::write(root.join(".ferrule/run.json"), "{ not json").unwrap();
    let broken = s.run_configurations().await.unwrap();
    assert!(broken.error.is_some());
    assert!(broken.stored.is_empty());
    assert!(!broken.detected.is_empty(), "detection still works");
}

#[tokio::test]
async fn test_results_fill_the_tree_and_the_gutter() {
    let Some(_) = node() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (s, events, root) = open(dir.path(), Arc::new(FakeRunBackend)).await;
    fake_packages(&root);
    s.trust_project().await.unwrap();
    let doc = s
        .open_document("src/math.test.ts".into())
        .await
        .unwrap()
        .doc_id;

    let blocks = s.test_blocks(doc).await.unwrap();
    assert_eq!(blocks.framework, Some(TestFramework::Vitest));
    let status_of = |blocks: &ferrule_ipc::TestBlocks, name: &[&str]| {
        blocks
            .blocks
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("no block {name:?}"))
            .status
    };
    assert_eq!(status_of(&blocks, &["math", "divides"]), None);

    let run = started(s.run_at(doc, None, SIZE).await.unwrap());
    assert!(run.tests);
    match &run.config.kind {
        RunKind::Test { scope, .. } => assert_eq!(scope.files, ["src/math.test.ts"]),
        other => panic!("{other:?}"),
    }
    let finished = wait_finished(&events, run.id, Duration::from_secs(10)).await;
    assert_eq!(finished.exit_code, Some(1));
    assert_eq!(finished.counts, Some((3, 1, 1)));
    assert_eq!(finished.report_error, None);
    let tree = finished.tests.unwrap();
    assert_eq!(tree[0].name, "src/math.test.ts");
    let divides = find(&tree, &["math", "divides"]).unwrap();
    assert_eq!(divides.status, TestStatus::Failed);
    assert!(divides.location.in_project);
    assert_eq!(divides.location.line, 10, "line from the test blocks");
    let frame = &divides.failures[0].frames[0];
    assert_eq!(frame.path, "src/math.test.ts");
    assert!(frame.in_project);
    assert_eq!((frame.line, frame.column), (12, 26));
    assert!(divides.failures[0].message.contains("expected 0.25"));

    let blocks = s.test_blocks(doc).await.unwrap();
    assert_eq!(
        status_of(&blocks, &["math", "divides"]),
        Some(TestStatus::Failed)
    );
    assert_eq!(
        status_of(&blocks, &["math", "adds"]),
        Some(TestStatus::Passed)
    );
    assert_eq!(status_of(&blocks, &["math"]), Some(TestStatus::Failed));
    assert_eq!(
        status_of(&blocks, &["math", "is skipped"]),
        Some(TestStatus::Skipped)
    );
    assert_eq!(
        status_of(&blocks, &["math", "tables", "adds %i and %i"]),
        Some(TestStatus::Passed)
    );

    // The report was read and removed.
    let reports: Vec<_> = fs::read_dir(dir.path().join("data/runs"))
        .unwrap()
        .flat_map(|d| fs::read_dir(d.unwrap().path()).unwrap())
        .collect();
    assert!(reports.is_empty(), "{reports:?}");

    let failed = started(s.rerun_failed(run.id, SIZE).await.unwrap());
    assert_eq!(failed.name, "src/math.test.ts (failed)");
    match &failed.config.kind {
        RunKind::Test { scope, .. } => {
            assert!(scope.files.is_empty());
            let names: Vec<&Vec<String>> = scope.tests.iter().map(|t| &t.name).collect();
            assert_eq!(names, [&vec!["math".to_owned(), "divides".to_owned()]]);
        }
        other => panic!("{other:?}"),
    }
    assert!(failed.command.contains("-t"), "{}", failed.command);
    wait_finished(&events, failed.id, Duration::from_secs(10)).await;

    // From the test tree: one test, and a whole file.
    let target = |path: &str, name: &[&str]| TestTarget {
        path: path.into(),
        name: name.iter().map(|n| (*n).to_owned()).collect(),
    };
    let one = started(
        s.rerun_tests(
            run.id,
            vec![target("src/math.test.ts", &["math", "adds"])],
            SIZE,
        )
        .await
        .unwrap(),
    );
    assert_eq!(one.name, "src/math.test.ts (adds)");
    match &one.config.kind {
        RunKind::Test { scope, .. } => {
            assert_eq!(scope.tests[0].file, "src/math.test.ts");
            assert_eq!(scope.tests[0].name, ["math", "adds"]);
        }
        other => panic!("{other:?}"),
    }
    let file = started(
        s.rerun_tests(run.id, vec![target("src/math.test.ts", &[])], SIZE)
            .await
            .unwrap(),
    );
    match &file.config.kind {
        RunKind::Test { scope, .. } => assert_eq!(scope.files, ["src/math.test.ts"]),
        other => panic!("{other:?}"),
    }
    let err = s
        .rerun_tests(run.id, vec![target("../elsewhere.test.ts", &[])], SIZE)
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);

    // The "adds" test (zero-based line 5) alone.
    let one = started(s.run_at(doc, Some(5), SIZE).await.unwrap());
    match &one.config.kind {
        RunKind::Test { scope, .. } => {
            assert_eq!(scope.tests[0].name, ["math", "adds"]);
            assert_eq!(scope.tests[0].line, Some(6));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(one.name, "adds");
}

#[tokio::test]
async fn stopping_keeps_the_output_and_closing_the_project_ends_runs() {
    let Some(_) = node() else { return };
    if ferrule_lsp_client::find_on_path("npm").is_none() {
        eprintln!("skipped: npm is not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (s, events, root) = open(dir.path(), Arc::new(FakeRunBackend)).await;
    s.trust_project().await.unwrap();

    // A plain file runs with Node and prints a link.
    let main = s.open_document("src/main.js".into()).await.unwrap().doc_id;
    let script = started(s.run_at(main, None, SIZE).await.unwrap());
    assert!(!script.tests);
    assert_eq!(
        script.config.kind,
        RunKind::Node {
            file: "src/main.js".into()
        }
    );
    let finished = wait_finished(&events, script.id, Duration::from_secs(10)).await;
    assert_eq!(finished.exit_code, Some(0));
    assert!(finished.tests.is_none());
    wait_output(
        &events,
        script.id,
        "see src/math.ts:5:10",
        Duration::from_secs(5),
    )
    .await;

    let serve = RunConfig {
        id: "adhoc:dev".into(),
        name: "dev".into(),
        kind: RunKind::NpmScript {
            script: "dev".into(),
        },
        cwd: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        package_manager: Some(PackageManager::Npm),
    };
    let run = started(s.start_run(serve, SIZE).await.unwrap());
    wait_output(
        &events,
        run.id,
        "serving until stopped",
        Duration::from_secs(5),
    )
    .await;
    assert!(events.finished(run.id).is_none());

    s.stop_run(run.id).await.unwrap();
    let stopped = wait_finished(&events, run.id, Duration::from_secs(10)).await;
    assert_eq!(stopped.exit_code, None, "ended by a signal");
    assert!(
        s.runs().await.unwrap().iter().any(|r| r.id == run.id),
        "a stopped run keeps its tab"
    );

    let again = started(s.rerun(run.id, SIZE).await.unwrap());
    assert_ne!(again.id, run.id);
    assert_eq!(again.config.id, "adhoc:dev");
    wait_output(
        &events,
        again.id,
        "serving until stopped",
        Duration::from_secs(5),
    )
    .await;

    s.close_run(run.id).await.unwrap();
    assert!(!s.runs().await.unwrap().iter().any(|r| r.id == run.id));

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    assert_eq!(
        s.runs().await.unwrap_err().kind,
        IpcErrorKind::Conflict,
        "no project"
    );
    crate::cleanup::remove_project_dir(&root, &report.still_running);
}

/// Links the repository's `node_modules` (where Vitest and Playwright are
/// installed for the frontend) into the copied fixture.
fn link_node_modules(root: &Path) -> bool {
    let target = repo().join("node_modules");
    for package in ["vitest", "@playwright/test"] {
        if !target.join(package).join("package.json").is_file() {
            eprintln!("skipped: {package} is not installed (run `bun install`)");
            return false;
        }
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&target, root.join("node_modules"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&target, root.join("node_modules"));
    match linked {
        Ok(()) => true,
        Err(err) => {
            eprintln!("skipped: cannot link node_modules: {err}");
            false
        }
    }
}

fn native() -> Arc<dyn ferrule_terminal::Backend> {
    Arc::new(ferrule_terminal::native::NativeBackend)
}

#[tokio::test]
async fn real_node_vitest_and_playwright_runs() {
    let Some(_) = node() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (s, events, root) = open(dir.path(), native()).await;
    if !link_node_modules(&root) {
        return;
    }
    s.trust_project().await.unwrap();
    let configs = s.run_configurations().await.unwrap();
    let slow = Duration::from_secs(120);

    let main = s.open_document("src/main.js".into()).await.unwrap().doc_id;
    let script = started(s.run_at(main, None, SIZE).await.unwrap());
    let finished = wait_finished(&events, script.id, slow).await;
    assert_eq!(finished.exit_code, Some(0), "{}", events.output(script.id));
    wait_output(&events, script.id, "see src/math.ts:5:10", slow).await;
    assert!(events.output(script.id).contains("started"));

    let vitest = detected(&configs.detected, "detected:test:vitest");
    let run = started(s.start_run(vitest, SIZE).await.unwrap());
    let finished = wait_finished(&events, run.id, slow).await;
    let output = events.output(run.id);
    assert_eq!(finished.report_error, None, "{output}");
    assert_eq!(finished.exit_code, Some(1), "{output}");
    assert_eq!(finished.counts, Some((3, 1, 1)), "{output}");
    let tree = finished.tests.unwrap();
    let divides = find(&tree, &["math", "divides"]).unwrap();
    assert_eq!(divides.location.line, 10);
    let frame = &divides.failures[0].frames[0];
    assert_eq!((frame.path.as_str(), frame.line), ("src/math.test.ts", 12));

    let playwright = detected(&configs.detected, "detected:test:playwright");
    let run = started(s.start_run(playwright, SIZE).await.unwrap());
    let finished = wait_finished(&events, run.id, slow).await;
    let output = events.output(run.id);
    assert_eq!(finished.report_error, None, "{output}");
    assert_eq!(finished.exit_code, Some(1), "{output}");
    assert_eq!(finished.counts, Some((1, 1, 0)), "{output}");
    let fails = find(
        &finished.tests.unwrap(),
        &["e2e math", "fails without a browser"],
    )
    .cloned()
    .unwrap();
    assert_eq!(fails.location.path, "e2e/math.spec.ts");
    assert!(fails.location.line > 0);

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    crate::cleanup::remove_project_dir(&root, &report.still_running);
}

/// A run that starts a child of its own: closing the project ends both,
/// so the project folder can be deleted (on Windows a live child's working
/// directory would block that). Runs Node directly, no shell.
#[tokio::test]
async fn closing_the_project_ends_a_run_and_its_children() {
    let Some(_) = node() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (s, events, root) = open(dir.path(), native()).await;
    fs::write(
        root.join("src/spawner.js"),
        "import { spawn } from \"node:child_process\";\n\
         const child = spawn(process.execPath, [\"-e\", \"setInterval(() => {}, 1000)\"], \
         { stdio: \"ignore\" });\n\
         console.log(`spawned ${child.pid}`);\n\
         setInterval(() => {}, 1000);\n",
    )
    .unwrap();
    s.trust_project().await.unwrap();
    let config = RunConfig {
        id: "adhoc:spawner".into(),
        name: "spawner".into(),
        kind: RunKind::Node {
            file: "src/spawner.js".into(),
        },
        cwd: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        package_manager: None,
    };
    let run = started(s.start_run(config, SIZE).await.unwrap());
    wait_output(&events, run.id, "spawned", Duration::from_secs(30)).await;

    let report = s.close_project().await.unwrap();
    assert!(report.still_running.is_empty(), "{report:?}");
    crate::cleanup::remove_project_dir(&root, &report.still_running);
}
