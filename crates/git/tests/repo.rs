//! `ferrule-git` against real repositories in temp directories (needs `git`
//! on PATH, as CI runners have).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

use ferrule_git::{FailureKind, Git, GitError, Repo, StatusKind, diff_lines, rollback_hunk};

fn sh(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A repository with one commit (`a.txt`, `src/b.ts`), independent of the
/// user's global config where it matters (identity, signing, hooks).
fn repo() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    sh(root, &["init", "-q", "-b", "main"]);
    sh(root, &["config", "user.name", "Test User"]);
    sh(root, &["config", "user.email", "test@example.com"]);
    sh(root, &["config", "commit.gpgsign", "false"]);
    sh(root, &["config", "core.hooksPath", ".git/hooks"]);
    sh(root, &["config", "core.autocrlf", "false"]);
    write(root, "a.txt", "one\ntwo\nthree\n");
    write(root, "src/b.ts", "export const b = 1;\n");
    write(root, ".gitignore", "node_modules/\n*.log\n");
    sh(root, &["add", "."]);
    sh(root, &["commit", "-q", "-m", "Initial commit"]);
    let repo = Git::new("git").discover(root).unwrap().unwrap();
    (dir, repo)
}

fn kinds(repo: &Repo) -> Vec<(String, StatusKind)> {
    let mut v: Vec<_> = repo
        .status()
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.kind))
        .collect();
    v.sort();
    v
}

#[test]
fn discovery_finds_the_root_and_the_prefix() {
    let (dir, _) = repo();
    let git = Git::new("git");
    assert!(git.version().unwrap().starts_with("git version"));
    let sub = git.discover(&dir.path().join("src")).unwrap().unwrap();
    assert_eq!(sub.prefix(), "src");
    let outside = tempfile::tempdir().unwrap();
    assert!(git.discover(outside.path()).unwrap().is_none());
    let missing = Git::new("ferrule-no-such-git").discover(dir.path());
    assert!(matches!(missing, Err(GitError::NotInstalled { .. })));
}

#[test]
fn status_reports_every_kind() {
    let (dir, repo) = repo();
    let root = dir.path();
    write(root, "a.txt", "one\nTWO\nthree\n");
    write(root, "new.ts", "x");
    sh(root, &["add", "new.ts"]);
    write(root, "untracked/c.md", "c");
    write(root, "node_modules/pkg/index.js", "");
    write(root, "debug.log", "");
    sh(root, &["mv", "src/b.ts", "src/renamed.ts"]);
    let status = repo.status().unwrap();
    assert_eq!(status.branch.head.as_deref(), Some("main"));
    assert!(status.branch.oid.is_some());
    assert_eq!(
        kinds(&repo),
        [
            ("a.txt".to_owned(), StatusKind::Modified),
            ("debug.log".to_owned(), StatusKind::Ignored),
            ("new.ts".to_owned(), StatusKind::Added),
            ("node_modules".to_owned(), StatusKind::Ignored),
            ("src/renamed.ts".to_owned(), StatusKind::Renamed),
            ("untracked/c.md".to_owned(), StatusKind::Untracked),
        ]
    );
    let renamed = status
        .files
        .iter()
        .find(|f| f.path == "src/renamed.ts")
        .unwrap();
    assert_eq!(renamed.orig_path.as_deref(), Some("src/b.ts"));

    fs::remove_file(root.join("a.txt")).unwrap();
    assert!(kinds(&repo).contains(&("a.txt".to_owned(), StatusKind::Deleted)));
}

#[test]
fn status_reports_conflicts() {
    let (dir, repo) = repo();
    let root = dir.path();
    sh(root, &["switch", "-q", "-c", "other"]);
    write(root, "a.txt", "one\nother\nthree\n");
    sh(root, &["commit", "-q", "-am", "other"]);
    sh(root, &["switch", "-q", "main"]);
    write(root, "a.txt", "one\nmain\nthree\n");
    sh(root, &["commit", "-q", "-am", "main"]);
    let merge = Command::new("git")
        .current_dir(root)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    assert_eq!(kinds(&repo), [("a.txt".to_owned(), StatusKind::Conflicted)]);
}

#[test]
fn stage_commit_amend_and_log() {
    let (dir, repo) = repo();
    let root = dir.path();
    write(root, "a.txt", "one\n2\nthree\n");
    write(root, "c.txt", "c\n");
    repo.stage(&["a.txt", "c.txt"]).unwrap();
    let staged = repo.status().unwrap();
    assert!(staged.files.iter().all(|f| f.index.is_some()));
    repo.unstage(&["c.txt"]).unwrap();
    assert!(kinds(&repo).contains(&("c.txt".to_owned(), StatusKind::Untracked)));

    let id = repo.commit("Change a\n\nWith a body.", false).unwrap();
    assert!(!id.is_empty());
    assert_eq!(
        repo.last_message().unwrap().as_deref(),
        Some("Change a\n\nWith a body.")
    );
    repo.commit("Change a, better", true).unwrap();
    let log = repo.log(None, 10).unwrap();
    assert_eq!(log.len(), 2, "amend replaces the commit");
    assert_eq!(log[0].subject, "Change a, better");
    assert_eq!(log[0].author, "Test User");
    let file_log = repo.log(Some("src/b.ts"), 10).unwrap();
    assert_eq!(file_log.len(), 1);
    assert_eq!(file_log[0].subject, "Initial commit");
}

#[test]
fn commit_failures_are_recognized() {
    let (dir, repo) = repo();
    let root = dir.path();
    let kind = |r: Result<String, GitError>| match r {
        Err(GitError::Failed { kind, .. }) => kind,
        other => panic!("expected a failure, got {other:?}"),
    };
    assert_eq!(
        kind(repo.commit("nothing", false)),
        FailureKind::NothingToCommit
    );

    write(root, "a.txt", "changed\n");
    repo.stage(&["a.txt"]).unwrap();
    sh(root, &["config", "user.name", ""]);
    sh(root, &["config", "user.useConfigOnly", "true"]);
    assert_eq!(
        kind(repo.commit("no identity", false)),
        FailureKind::MissingIdentity
    );
    sh(root, &["config", "user.name", "Test User"]);

    write(
        root,
        ".git/hooks/pre-commit",
        "#!/bin/sh\necho 'lint failed: a.txt' >&2\nexit 1\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            root.join(".git/hooks/pre-commit"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    match repo.commit("hooked", false) {
        Err(GitError::Failed { kind, message }) => {
            assert_eq!(kind, FailureKind::HookFailed);
            assert!(message.contains("lint failed"), "{message}");
        }
        other => panic!("expected a hook failure, got {other:?}"),
    }
}

#[test]
fn unstage_before_the_first_commit() {
    let dir = tempfile::tempdir().unwrap();
    sh(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), "first.txt", "x");
    let repo = Git::new("git").discover(dir.path()).unwrap().unwrap();
    assert_eq!(repo.status().unwrap().branch.oid, None);
    repo.stage(&["first.txt"]).unwrap();
    assert_eq!(kinds(&repo), [("first.txt".to_owned(), StatusKind::Added)]);
    repo.unstage(&["first.txt"]).unwrap();
    assert_eq!(
        kinds(&repo),
        [("first.txt".to_owned(), StatusKind::Untracked)]
    );
    assert!(repo.log(None, 5).unwrap().is_empty());
    assert_eq!(repo.last_message().unwrap(), None);
}

#[test]
fn branches_checkout_create_delete() {
    let (dir, repo) = repo();
    let root = dir.path();
    repo.create_branch("feature", true).unwrap();
    let branches = repo.branches().unwrap();
    let current: Vec<&str> = branches
        .iter()
        .filter(|b| b.current)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(current, ["feature"]);
    assert!(matches!(
        repo.create_branch("main", false),
        Err(GitError::Failed {
            kind: FailureKind::BranchExists,
            ..
        })
    ));

    // An unmerged commit on feature.
    write(root, "a.txt", "feature\n");
    sh(root, &["commit", "-q", "-am", "feature work"]);
    // Local change that checkout would overwrite.
    write(root, "a.txt", "dirty\n");
    assert!(matches!(
        repo.checkout("main", false),
        Err(GitError::Failed {
            kind: FailureKind::LocalChanges,
            ..
        })
    ));
    sh(root, &["checkout", "--", "a.txt"]);
    repo.checkout("main", false).unwrap();
    assert_eq!(repo.status().unwrap().branch.head.as_deref(), Some("main"));

    assert!(matches!(
        repo.delete_branch("feature", false),
        Err(GitError::Failed {
            kind: FailureKind::NotMerged,
            ..
        })
    ));
    repo.delete_branch("feature", true).unwrap();
    repo.create_branch("merged", false).unwrap();
    repo.delete_branch("merged", false).unwrap();
    assert_eq!(repo.branches().unwrap().len(), 1);
}

#[test]
fn checkout_of_a_remote_branch_tracks_it() {
    let (origin_dir, _) = repo();
    let clone = tempfile::tempdir().unwrap();
    sh(
        clone.path(),
        &["clone", "-q", &origin_dir.path().to_string_lossy(), "c"],
    );
    let root = clone.path().join("c");
    sh(&root, &["config", "user.name", "Test User"]);
    sh(&root, &["config", "user.email", "test@example.com"]);
    sh(origin_dir.path(), &["branch", "remote-only"]);
    sh(&root, &["fetch", "-q"]);
    let repo = Git::new("git").discover(&root).unwrap().unwrap();
    let branches = repo.branches().unwrap();
    assert!(
        branches
            .iter()
            .any(|b| b.remote && b.name == "origin/remote-only")
    );
    assert!(
        branches.iter().all(|b| b.name != "origin"),
        "no origin/HEAD"
    );
    repo.checkout("origin/remote-only", true).unwrap();
    let status = repo.status().unwrap();
    assert_eq!(status.branch.head.as_deref(), Some("remote-only"));
    assert_eq!(
        status.branch.upstream.as_deref(),
        Some("origin/remote-only")
    );
}

#[test]
fn blame_uses_the_current_text() {
    let (_dir, repo) = repo();
    let lines = repo.blame("a.txt", "one\nedited\nthree\nfour\n").unwrap();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].committed);
    assert_eq!(lines[0].author, "Test User");
    assert_eq!(lines[0].summary, "Initial commit");
    assert!(!lines[1].committed);
    assert!(lines[2].committed);
    assert!(!lines[3].committed);
}

#[test]
fn index_text_and_hunk_rollback() {
    let (dir, repo) = repo();
    let root = dir.path();
    let base = repo.index_text("a.txt").unwrap().unwrap();
    assert_eq!(base, "one\ntwo\nthree\n");
    assert_eq!(repo.index_text("missing.txt").unwrap(), None);
    assert_eq!(
        repo.head_text("src/b.ts").unwrap().as_deref(),
        Some("export const b = 1;\n")
    );
    // Staged changes move the index away from HEAD.
    write(root, "a.txt", "one\nstaged\nthree\n");
    repo.stage(&["a.txt"]).unwrap();
    assert_eq!(
        repo.index_text("a.txt").unwrap().as_deref(),
        Some("one\nstaged\nthree\n")
    );
    let editor = "zero\none\nstaged\nthree\n";
    let index = repo.index_text("a.txt").unwrap().unwrap();
    let hunks = diff_lines(&index, editor);
    assert_eq!(hunks.len(), 1);
    let (from, to, insert) = rollback_hunk(&index, editor, &hunks[0]).unwrap();
    let rolled = format!("{}{insert}{}", &editor[..from], &editor[to..]);
    assert_eq!(rolled, index);
}
