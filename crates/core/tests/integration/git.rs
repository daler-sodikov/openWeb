//! Git through the session on real repositories in temp directories:
//! status events and refresh, project in a subfolder, gutter hunks and
//! rollback, diff, blame, commit (saving first), checkout guard, branches.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrule_core::{EventSink, Session, SessionConfig, SessionEvent};
use ferrule_ipc::{
    ChangeTransaction, EditKind, EditorSelection, GitDeleteResult, GitDiffMode, GitFileKind,
    GitHunkKind, GitLineKind, GitStatus, IpcErrorKind, SelectionRange, TextEdit,
};

#[derive(Default)]
struct Events(Mutex<Vec<SessionEvent>>);

impl EventSink for Events {
    fn emit(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn git_events(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, SessionEvent::GitStatus(_)))
            .count()
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `repo/` with one commit: `web/src/a.ts`, `web/b.ts`, `docs/x.md`.
fn make_repo(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Test User"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.hooksPath", ".git/hooks"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    write(&root, "web/src/a.ts", "one\ntwo\nthree\n");
    write(&root, "web/b.ts", "b\n");
    write(&root, "docs/x.md", "x\n");
    write(&root, ".gitignore", "node_modules/\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Initial commit"]);
    root
}

async fn open(dir: &Path, project: &Path) -> (Arc<Session>, Arc<Events>) {
    let events = Arc::new(Events::default());
    let s = Session::with_config(SessionConfig::new(dir.join("data")), events.clone()).unwrap();
    s.open_project(project.to_string_lossy().into_owned())
        .await
        .unwrap();
    (s, events)
}

async fn wait_status(s: &Session, what: &str, pred: impl Fn(&GitStatus) -> bool) -> GitStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status = s.git_status().await.unwrap();
        if pred(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "no status {what}: {status:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn kind_of(status: &GitStatus, path: &str) -> Option<GitFileKind> {
    status.files.iter().find(|f| f.path == path).map(|f| f.kind)
}

fn caret(pos: u32) -> EditorSelection {
    EditorSelection {
        ranges: vec![SelectionRange {
            anchor: pos,
            head: pos,
        }],
        main: 0,
    }
}

fn typing(at: u32, text: &str) -> ChangeTransaction {
    let len = u32::try_from(text.encode_utf16().count()).unwrap();
    ChangeTransaction {
        edits: vec![TextEdit {
            from: at,
            to: at,
            insert: text.into(),
        }],
        kind: EditKind::Typing,
        selection_before: caret(at),
        selection_after: caret(at + len),
    }
}

fn apply(text: &str, edits: &[TextEdit]) -> String {
    // Single-edit replaces in these tests; offsets are ASCII.
    let mut out = text.to_owned();
    for e in edits.iter().rev() {
        out.replace_range(e.from as usize..e.to as usize, &e.insert);
    }
    out
}

#[tokio::test]
async fn status_follows_the_project_in_a_subfolder() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path());
    write(&repo, "web/src/a.ts", "one\nTWO\nthree\n");
    write(&repo, "web/new.ts", "n\n");
    write(&repo, "docs/x.md", "changed outside the project\n");
    write(&repo, "web/node_modules/p/i.js", "");
    let (s, events) = open(dir.path(), &repo.join("web")).await;
    let status = wait_status(&s, "with the change", |st| !st.files.is_empty()).await;
    assert_eq!(status.unavailable, None);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(kind_of(&status, "src/a.ts"), Some(GitFileKind::Modified));
    assert_eq!(kind_of(&status, "new.ts"), Some(GitFileKind::Untracked));
    assert!(
        status.files.iter().all(|f| !f.path.contains("x.md")),
        "files outside the project are not listed"
    );
    assert_eq!(status.changed_dirs, ["src"]);
    assert_eq!(status.ignored_dirs, ["node_modules"]);
    assert!(events.git_events() >= 1);

    // A change on disk is picked up by the watcher.
    tokio::time::sleep(Duration::from_millis(300)).await;
    write(&repo, "web/b.ts", "b changed\n");
    wait_status(&s, "with b.ts modified", |st| {
        kind_of(st, "b.ts") == Some(GitFileKind::Modified)
    })
    .await;
}

#[tokio::test]
async fn outside_a_repository_git_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let (s, _) = open(dir.path(), &plain).await;
    let status = wait_status(&s, "unavailable", |st| {
        st.unavailable
            .as_deref()
            .is_some_and(|u| u.contains("not in a Git repository"))
    })
    .await;
    assert!(status.files.is_empty());
    let err = s.git_branches().await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
}

#[tokio::test]
async fn gutter_hunks_follow_unsaved_edits_and_roll_back() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path());
    let (s, _) = open(dir.path(), &repo).await;
    wait_status(&s, "loaded", |st| st.generation > 0).await;
    let doc = s.open_document("web/src/a.ts".into()).await.unwrap();
    let none = s.git_line_changes(doc.doc_id).await.unwrap();
    assert!(none.tracked);
    assert!(none.hunks.is_empty());

    // Insert a line at the top (unsaved).
    s.apply_changes(doc.doc_id, 0, vec![typing(0, "zero\n")])
        .await
        .unwrap();
    let changes = s.git_line_changes(doc.doc_id).await.unwrap();
    assert_eq!(changes.version, 1);
    assert_eq!(changes.hunks.len(), 1);
    let h = &changes.hunks[0];
    assert_eq!((h.kind, h.line, h.lines), (GitHunkKind::Added, 0, 1));

    let replace = s
        .git_rollback_hunk(doc.doc_id, 1, h.line, h.lines)
        .await
        .unwrap();
    assert_eq!(replace.version, 1);
    assert_eq!(
        apply("zero\none\ntwo\nthree\n", &replace.edits),
        "one\ntwo\nthree\n"
    );
    let stale = s.git_rollback_hunk(doc.doc_id, 0, 0, 1).await.unwrap_err();
    assert_eq!(stale.kind, IpcErrorKind::Conflict);

    // Untracked files have no markers.
    write(&repo, "fresh.ts", "f\n");
    let fresh = s.open_document("fresh.ts".into()).await.unwrap();
    assert!(!s.git_line_changes(fresh.doc_id).await.unwrap().tracked);
}

#[tokio::test]
async fn diff_and_blame() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path());
    write(&repo, "web/src/a.ts", "one\nTWO\nthree\nfour\n");
    let (s, _) = open(dir.path(), &repo).await;
    wait_status(&s, "loaded", |st| st.generation > 0).await;

    let unified = s
        .git_diff("web/src/a.ts".into(), GitDiffMode::Unified)
        .await
        .unwrap();
    assert!(unified.old_label.starts_with("HEAD ("));
    assert_eq!(unified.new_label, "Working tree");
    assert_eq!(unified.hunks, 2);
    let kinds: Vec<(GitLineKind, &str)> = unified
        .unified
        .iter()
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (GitLineKind::Context, "one"),
            (GitLineKind::Deleted, "two"),
            (GitLineKind::Added, "TWO"),
            (GitLineKind::Context, "three"),
            (GitLineKind::Added, "four"),
        ]
    );
    assert_eq!(unified.unified[2].words, [0, 3]);
    let split = s
        .git_diff("web/src/a.ts".into(), GitDiffMode::SideBySide)
        .await
        .unwrap();
    assert_eq!(split.split.len(), 4);
    assert!(split.unified.is_empty());

    let new_file = s
        .git_diff("web/b.ts".into(), GitDiffMode::Unified)
        .await
        .unwrap();
    assert_eq!(new_file.hunks, 0);

    let doc = s.open_document("web/src/a.ts".into()).await.unwrap();
    let blame = s.git_blame(doc.doc_id).await.unwrap();
    assert_eq!(blame.lines.len(), 4);
    assert!(blame.lines[0].committed);
    assert_eq!(blame.lines[0].author, "Test User");
    assert_eq!(blame.lines[0].summary, "Initial commit");
    assert!(!blame.lines[1].committed);
    assert!(blame.lines[0].time > 1_600_000_000);
}

#[tokio::test]
async fn commit_saves_documents_and_explains_failures() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path());
    let (s, _) = open(dir.path(), &repo).await;
    wait_status(&s, "loaded", |st| st.generation > 0).await;

    let err = s.git_commit("  ".into(), false, vec![]).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
    let err = s
        .git_commit("Nothing".into(), false, vec![])
        .await
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
    assert!(err.message.contains("Nothing to commit"), "{}", err.message);

    // A file checked (staged) before an unsaved edit: the edit is saved
    // and included.
    let doc = s.open_document("web/b.ts".into()).await.unwrap();
    s.git_stage(vec!["web/b.ts".into()]).await.unwrap();
    s.apply_changes(doc.doc_id, 0, vec![typing(0, "edited ")])
        .await
        .unwrap();
    let id = s
        .git_commit("Edit b".into(), false, vec!["web/b.ts".into()])
        .await
        .unwrap();
    assert!(!id.is_empty());
    assert_eq!(
        fs::read_to_string(repo.join("web/b.ts")).unwrap(),
        "edited b\n"
    );
    let log = s.git_log(None).await.unwrap();
    assert_eq!(log[0].subject, "Edit b");
    assert_eq!(s.git_log(Some("docs/x.md".into())).await.unwrap().len(), 1);
    assert_eq!(
        s.git_last_commit_message().await.unwrap().as_deref(),
        Some("Edit b")
    );
    // The status is fresh when the command returns.
    let status = s.git_status().await.unwrap();
    assert_eq!(kind_of(&status, "web/b.ts"), None);

    // Missing identity is explained.
    write(&repo, "docs/x.md", "y\n");
    s.git_stage(vec!["docs/x.md".into()]).await.unwrap();
    git(&repo, &["config", "user.name", ""]);
    git(&repo, &["config", "user.useConfigOnly", "true"]);
    let err = s.git_commit("Who".into(), false, vec![]).await.unwrap_err();
    assert!(err.message.contains("user.name"), "{}", err.message);
    git(&repo, &["config", "user.name", "Test User"]);

    // Unstage puts the file back.
    s.git_unstage(vec!["docs/x.md".into()]).await.unwrap();
    let status = s.git_status().await.unwrap();
    let x = status.files.iter().find(|f| f.path == "docs/x.md").unwrap();
    assert!(!x.staged && x.unstaged);

    // Amend rewrites the last commit.
    s.git_commit("Edit b, amended".into(), true, vec![])
        .await
        .unwrap();
    let log = s.git_log(None).await.unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].subject, "Edit b, amended");
}

#[tokio::test]
async fn branches_and_the_checkout_guard() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path());
    let (s, _) = open(dir.path(), &repo).await;
    wait_status(&s, "loaded", |st| st.generation > 0).await;

    s.git_create_branch("feature".into(), true).await.unwrap();
    assert_eq!(
        s.git_status().await.unwrap().branch.as_deref(),
        Some("feature")
    );
    write(&repo, "web/b.ts", "feature\n");
    s.git_stage(vec!["web/b.ts".into()]).await.unwrap();
    s.git_commit("Feature work".into(), false, vec![])
        .await
        .unwrap();

    // Unsaved edits block a checkout.
    let doc = s.open_document("web/src/a.ts".into()).await.unwrap();
    s.apply_changes(doc.doc_id, 0, vec![typing(0, "x")])
        .await
        .unwrap();
    let err = s.git_checkout("main".into(), false).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::Conflict);
    assert!(err.message.contains("web/src/a.ts"), "{}", err.message);
    s.save_document(doc.doc_id).await.unwrap();

    // Git refuses when the change would be overwritten... a.ts is the same
    // on both branches, so this checkout carries the change over.
    s.git_checkout("main".into(), false).await.unwrap();
    assert_eq!(
        s.git_status().await.unwrap().branch.as_deref(),
        Some("main")
    );

    let branches = s.git_branches().await.unwrap();
    assert_eq!(branches.len(), 2);
    assert!(branches.iter().any(|b| b.current && b.name == "main"));

    assert_eq!(
        s.git_delete_branch("feature".into(), false).await.unwrap(),
        GitDeleteResult::NotMerged
    );
    assert_eq!(
        s.git_delete_branch("feature".into(), true).await.unwrap(),
        GitDeleteResult::Deleted
    );
    let err = s.git_create_branch(" ".into(), false).await.unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
}
