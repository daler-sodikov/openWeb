//! Git for the open project (`docs/ARCHITECTURE.md`, 4.9): the repository
//! containing it, its status (refreshed in the background and pushed as
//! `git-status-event`), gutter hunks, rollback, diffs, blame, history,
//! branches and commits. Every `git` call runs on a blocking thread.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use ferrule_git::{
    Change, DiffRow, FailureKind, Git, GitError, HunkKind, LineKind, Repo, RepoStatus, StatusKind,
};
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};

use crate::convert::to_u32;
use crate::events::{EventSink, SessionEvent};
use crate::session::{Session, blocking, lock};

/// Quiet time before a requested refresh runs.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(200);
/// A refresh runs at the latest this long after the first request, even
/// while requests keep coming.
const REFRESH_MAX_DELAY: Duration = Duration::from_secs(1);
/// Commits listed in a history.
const LOG_LIMIT: usize = 1000;

#[derive(Default)]
struct Inner {
    repo: Option<Repo>,
    status: ipc::GitStatus,
    raw: RepoStatus,
    /// Index text per repository path, for the current status generation.
    bases: HashMap<String, Option<Arc<str>>>,
}

/// The project's repository and its latest status.
pub(crate) struct GitState {
    sink: Arc<dyn EventSink>,
    inner: Mutex<Inner>,
    refresh: Mutex<Option<mpsc::Sender<()>>>,
    generation: AtomicU32,
}

fn git_error(err: GitError) -> IpcError {
    match err {
        GitError::NotInstalled { .. } => IpcError::new(IpcErrorKind::Io, err.to_string()),
        GitError::Parse(_) => IpcError::new(IpcErrorKind::Internal, err.to_string()),
        GitError::Failed { kind, message } => {
            let text = match kind {
                FailureKind::MissingIdentity => "Git does not know who you are. Set your name \
                     and email with `git config --global user.name \"Your Name\"` and \
                     `git config --global user.email you@example.com`."
                    .to_owned(),
                FailureKind::HookFailed => format!("A commit hook rejected the commit:\n{message}"),
                FailureKind::NothingToCommit => {
                    "Nothing to commit: select the files to include.".to_owned()
                }
                FailureKind::LocalChanges => format!(
                    "Checkout would overwrite local changes; commit or roll them back first.\n{message}"
                ),
                FailureKind::NotMerged | FailureKind::BranchExists | FailureKind::Other => message,
            };
            IpcError::new(IpcErrorKind::Conflict, text)
        }
    }
}

fn kind_to_ipc(kind: StatusKind) -> ipc::GitFileKind {
    match kind {
        StatusKind::Conflicted => ipc::GitFileKind::Conflicted,
        StatusKind::Modified => ipc::GitFileKind::Modified,
        StatusKind::Added => ipc::GitFileKind::Added,
        StatusKind::Deleted => ipc::GitFileKind::Deleted,
        StatusKind::Renamed => ipc::GitFileKind::Renamed,
        StatusKind::Untracked => ipc::GitFileKind::Untracked,
        StatusKind::Ignored => ipc::GitFileKind::Ignored,
    }
}

/// Repository path to project path (`None` outside the project).
fn to_project(prefix: &str, path: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_owned());
    }
    if path == prefix {
        return Some(String::new());
    }
    path.strip_prefix(prefix)?
        .strip_prefix('/')
        .map(str::to_owned)
}

fn to_repo(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_owned()
    } else {
        format!("{prefix}/{path}")
    }
}

fn status_to_ipc(raw: &RepoStatus, prefix: &str, generation: u32) -> ipc::GitStatus {
    let mut files = Vec::new();
    let mut changed_dirs = BTreeSet::new();
    let mut ignored_dirs = Vec::new();
    for f in &raw.files {
        let Some(path) = to_project(prefix, &f.path).filter(|p| !p.is_empty()) else {
            continue;
        };
        if f.kind == StatusKind::Ignored && f.is_dir {
            ignored_dirs.push(path);
            continue;
        }
        if f.kind != StatusKind::Ignored {
            let mut dir = path.as_str();
            while let Some((parent, _)) = dir.rsplit_once('/') {
                changed_dirs.insert(parent.to_owned());
                dir = parent;
            }
        }
        let conflicted = f.kind == StatusKind::Conflicted;
        files.push(ipc::GitFile {
            orig_path: f
                .orig_path
                .as_deref()
                .map(|o| to_project(prefix, o).unwrap_or_else(|| o.to_owned())),
            kind: kind_to_ipc(f.kind),
            staged: f.index.is_some() && !conflicted,
            unstaged: f.worktree.is_some() || conflicted || f.kind == StatusKind::Untracked,
            path,
        });
    }
    let b = &raw.branch;
    ipc::GitStatus {
        unavailable: None,
        branch: b.head.clone(),
        head: b.oid.as_ref().map(|o| o.chars().take(8).collect()),
        upstream: b.upstream.clone(),
        ahead: b.ahead,
        behind: b.behind,
        files,
        changed_dirs: changed_dirs.into_iter().collect(),
        ignored_dirs,
        generation,
    }
}

/// Runs a refresh after requests settle; ends with the state.
fn refresher(state: Weak<GitState>, rx: mpsc::Receiver<()>) {
    while rx.recv().is_ok() {
        let first = Instant::now();
        loop {
            match rx.recv_timeout(REFRESH_DEBOUNCE) {
                Ok(()) if first.elapsed() < REFRESH_MAX_DELAY => {}
                Ok(()) | Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let Some(state) = state.upgrade() else {
            return;
        };
        state.refresh_now();
    }
}

impl GitState {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            inner: Mutex::new(Inner {
                status: ipc::GitStatus {
                    unavailable: Some("Looking for a Git repository…".into()),
                    ..ipc::GitStatus::default()
                },
                ..Inner::default()
            }),
            refresh: Mutex::new(None),
            generation: AtomicU32::new(0),
        }
    }

    /// Finds the repository containing `dir` and starts refreshing.
    /// Blocking.
    pub fn start(self: &Arc<Self>, dir: &Path, program: PathBuf) {
        let unavailable = match Git::new(program).discover(dir) {
            Ok(Some(repo)) => {
                tracing::info!(root = ?repo.root(), "Git repository");
                if let Ok(mut inner) = self.inner.lock() {
                    inner.repo = Some(repo);
                }
                let (tx, rx) = mpsc::channel();
                let weak = Arc::downgrade(self);
                let spawned = std::thread::Builder::new()
                    .name("git-refresh".into())
                    .spawn(move || refresher(weak, rx));
                match spawned {
                    Ok(_) => {
                        if let Ok(mut slot) = self.refresh.lock() {
                            *slot = Some(tx);
                        }
                    }
                    Err(err) => tracing::warn!(%err, "no Git refresh thread"),
                }
                self.refresh_now();
                return;
            }
            Ok(None) => "The project is not in a Git repository.".to_owned(),
            Err(err) => format!("{err}. Install Git or set git.path in settings.json."),
        };
        let status = ipc::GitStatus {
            unavailable: Some(unavailable),
            ..ipc::GitStatus::default()
        };
        if let Ok(mut inner) = self.inner.lock() {
            inner.status = status.clone();
        }
        self.sink.emit(SessionEvent::GitStatus(status));
    }

    /// Asks for a refresh soon (debounced; never blocks).
    pub fn request_refresh(&self) {
        if let Ok(slot) = self.refresh.lock()
            && let Some(tx) = slot.as_ref()
        {
            let _ = tx.send(());
        }
    }

    fn repo(&self) -> Result<(Repo, String), IpcError> {
        let inner = lock(&self.inner)?;
        match &inner.repo {
            Some(repo) => Ok((repo.clone(), repo.prefix().to_owned())),
            None => Err(IpcError::new(
                IpcErrorKind::Conflict,
                inner
                    .status
                    .unavailable
                    .clone()
                    .unwrap_or_else(|| "No Git repository.".into()),
            )),
        }
    }

    /// Recomputes the status now and pushes it. Blocking. A run that
    /// started before a newer one finished is dropped.
    pub fn refresh_now(&self) {
        let Ok((repo, prefix)) = self.repo() else {
            return;
        };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let raw = match repo.status() {
            Ok(raw) => raw,
            Err(err) => {
                tracing::warn!(%err, "git status failed");
                return;
            }
        };
        let status = status_to_ipc(&raw, &prefix, generation);
        {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            if inner.status.generation > generation {
                return;
            }
            inner.raw = raw;
            inner.status = status.clone();
            inner.bases.clear();
        }
        self.sink.emit(SessionEvent::GitStatus(status));
    }

    pub fn status(&self) -> ipc::GitStatus {
        self.inner
            .lock()
            .map(|i| i.status.clone())
            .unwrap_or_default()
    }

    /// Index text of a repository path (cached per status generation).
    fn base(&self, repo: &Repo, path: &str) -> Result<Option<Arc<str>>, IpcError> {
        if let Some(found) = lock(&self.inner)?.bases.get(path) {
            return Ok(found.clone());
        }
        let text = repo
            .index_text(path)
            .map_err(git_error)?
            .map(|t| Arc::from(t.replace("\r\n", "\n")));
        lock(&self.inner)?
            .bases
            .insert(path.to_owned(), text.clone());
        Ok(text)
    }

    /// The HEAD-side path of a project file (its old name if renamed).
    fn head_path(&self, repo_path: &str) -> String {
        self.inner
            .lock()
            .ok()
            .and_then(|i| {
                i.raw
                    .files
                    .iter()
                    .find(|f| f.path == repo_path && f.index == Some(Change::Renamed))
                    .and_then(|f| f.orig_path.clone())
            })
            .unwrap_or_else(|| repo_path.to_owned())
    }
}

fn utf16(text: &str, byte: usize) -> u32 {
    to_u32(text[..byte].encode_utf16().count())
}

fn line_to_ipc(row: DiffRow) -> ipc::GitDiffLine {
    ipc::GitDiffLine {
        old_line: row.old_line,
        new_line: row.new_line,
        kind: match row.kind {
            LineKind::Context => ipc::GitLineKind::Context,
            LineKind::Added => ipc::GitLineKind::Added,
            LineKind::Deleted => ipc::GitLineKind::Deleted,
        },
        text: row.text,
        words: row.words.into_iter().flat_map(|(a, b)| [a, b]).collect(),
    }
}

/// Seconds since the epoch as sent over IPC (u32 lasts until 2106).
fn seconds(time: i64) -> u32 {
    u32::try_from(time.max(0)).unwrap_or(u32::MAX)
}

impl Session {
    fn git(&self) -> Result<Arc<GitState>, IpcError> {
        Ok(self.runtime()?.git.clone())
    }

    /// The `git` program: `git.path` from the settings, else `git`.
    pub(crate) fn git_program(&self) -> PathBuf {
        ferrule_settings::load_user_settings(&self.settings_file)
            .ok()
            .and_then(|s| s.git_path)
            .unwrap_or_else(|| PathBuf::from("git"))
    }

    /// The latest repository status.
    pub async fn git_status(&self) -> Result<ipc::GitStatus, IpcError> {
        Ok(self.git()?.status())
    }

    /// Asks for a status refresh (window focus); the result arrives as an
    /// event.
    pub async fn git_refresh(&self) -> Result<(), IpcError> {
        self.git()?.request_refresh();
        Ok(())
    }

    /// Text, version and project path of an open document.
    fn doc_text(&self, doc_id: DocId) -> Result<(String, u32, String), IpcError> {
        let doc = self.doc(doc_id)?;
        let state = doc.lock()?;
        Ok((state.doc.text(), state.doc.version(), doc.rel_path.clone()))
    }

    /// Lines of a document that differ from the index.
    pub async fn git_line_changes(&self, doc_id: DocId) -> Result<ipc::GitLineChanges, IpcError> {
        let git = self.git()?;
        let (text, version, path) = self.doc_text(doc_id)?;
        blocking(move || {
            let (repo, prefix) = git.repo()?;
            let Some(base) = git.base(&repo, &to_repo(&prefix, &path))? else {
                return Ok(ipc::GitLineChanges {
                    version,
                    tracked: false,
                    hunks: Vec::new(),
                });
            };
            let hunks = ferrule_git::diff_lines(&base, &text)
                .into_iter()
                .map(|h| ipc::GitHunk {
                    kind: match h.kind {
                        HunkKind::Added => ipc::GitHunkKind::Added,
                        HunkKind::Modified => ipc::GitHunkKind::Modified,
                        HunkKind::Deleted => ipc::GitHunkKind::Deleted,
                    },
                    line: to_u32(h.new_start),
                    lines: to_u32(h.new_lines),
                    old_text: ferrule_git::hunk_base_text(&base, &h),
                })
                .collect();
            Ok(ipc::GitLineChanges {
                version,
                tracked: true,
                hunks,
            })
        })
        .await
    }

    /// The edit that rolls the hunk at `line` (with `lines` lines) back to
    /// the index text; the view applies it as one undo step.
    pub async fn git_rollback_hunk(
        &self,
        doc_id: DocId,
        version: u32,
        line: u32,
        lines: u32,
    ) -> Result<ipc::DocumentReplace, IpcError> {
        let git = self.git()?;
        let (text, current, path) = self.doc_text(doc_id)?;
        if current != version {
            return Err(IpcError::new(
                IpcErrorKind::Conflict,
                "the document changed; try again",
            ));
        }
        blocking(move || {
            let (repo, prefix) = git.repo()?;
            let base = git
                .base(&repo, &to_repo(&prefix, &path))?
                .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "the file is not tracked"))?;
            let hunk = ferrule_git::diff_lines(&base, &text)
                .into_iter()
                .find(|h| to_u32(h.new_start) == line && to_u32(h.new_lines) == lines)
                .ok_or_else(|| IpcError::new(IpcErrorKind::NotFound, "no change there"))?;
            let (from, to, insert) = ferrule_git::rollback_hunk(&base, &text, &hunk)
                .ok_or_else(|| IpcError::new(IpcErrorKind::Internal, "hunk out of range"))?;
            Ok(ipc::DocumentReplace {
                doc_id,
                version,
                edits: vec![ipc::TextEdit {
                    from: utf16(&text, from),
                    to: utf16(&text, to),
                    insert,
                }],
            })
        })
        .await
    }

    /// A project file against HEAD (its open document's text if open).
    pub async fn git_diff(
        &self,
        path: String,
        mode: ipc::GitDiffMode,
    ) -> Result<ipc::GitDiff, IpcError> {
        let git = self.git()?;
        let project = self.runtime()?.project.clone();
        let open = self
            .open_docs()
            .into_iter()
            .find(|d| d.rel_path == path)
            .map(|d| d.lock().map(|s| (s.doc.text(), s.dirty())))
            .transpose()?;
        blocking(move || {
            let (repo, prefix) = git.repo()?;
            let repo_path = to_repo(&prefix, &path);
            let head_path = git.head_path(&repo_path);
            let old = repo
                .head_text(&head_path)
                .map_err(git_error)?
                .map(|t| t.replace("\r\n", "\n"));
            let (new, dirty) = match open {
                Some(found) => (Some(found.0), found.1),
                None => {
                    let file = project.root().join(&path);
                    let text = if file.is_file() {
                        Some(
                            ferrule_vfs::read_text(&file)
                                .map_err(|e| IpcError::new(IpcErrorKind::Io, e.to_string()))?
                                .text,
                        )
                    } else {
                        None
                    };
                    (text, false)
                }
            };
            let head = git.status().head.unwrap_or_default();
            let old_label = match (&old, head_path == repo_path) {
                (None, _) => "HEAD (new file)".to_owned(),
                (Some(_), true) => format!("HEAD ({head})"),
                (Some(_), false) => format!("HEAD ({head}, {head_path})"),
            };
            let new_label = match (&new, dirty) {
                (None, _) => "Working tree (deleted)".to_owned(),
                (Some(_), true) => "Working tree (unsaved)".to_owned(),
                (Some(_), false) => "Working tree".to_owned(),
            };
            let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
            let hunks = to_u32(ferrule_git::diff_lines(&old, &new).len());
            let (unified, split) = match mode {
                ipc::GitDiffMode::Unified => (
                    ferrule_git::unified_rows(&old, &new)
                        .into_iter()
                        .map(line_to_ipc)
                        .collect(),
                    Vec::new(),
                ),
                ipc::GitDiffMode::SideBySide => (
                    Vec::new(),
                    ferrule_git::split_rows(&old, &new)
                        .into_iter()
                        .map(|r| ipc::GitSplitRow {
                            left: r.left.map(line_to_ipc),
                            right: r.right.map(line_to_ipc),
                        })
                        .collect(),
                ),
            };
            Ok(ipc::GitDiff {
                path,
                old_label,
                new_label,
                unified,
                split,
                hunks,
            })
        })
        .await
    }

    /// Who last changed each line of a document (its current text).
    pub async fn git_blame(&self, doc_id: DocId) -> Result<ipc::GitBlame, IpcError> {
        let git = self.git()?;
        let (text, version, path) = self.doc_text(doc_id)?;
        blocking(move || {
            let (repo, prefix) = git.repo()?;
            let lines = repo
                .blame(&to_repo(&prefix, &path), &text)
                .map_err(git_error)?
                .into_iter()
                .map(|l| ipc::GitBlameLine {
                    commit: if l.committed {
                        l.commit.chars().take(8).collect()
                    } else {
                        String::new()
                    },
                    author: l.author,
                    email: l.email,
                    time: seconds(l.time),
                    summary: l.summary,
                    committed: l.committed,
                })
                .collect();
            Ok(ipc::GitBlame { version, lines })
        })
        .await
    }

    /// Commits of the current branch, or of one project file.
    pub async fn git_log(&self, path: Option<String>) -> Result<Vec<ipc::GitCommit>, IpcError> {
        let git = self.git()?;
        blocking(move || {
            let (repo, prefix) = git.repo()?;
            let path = path.map(|p| to_repo(&prefix, &p));
            Ok(repo
                .log(path.as_deref(), LOG_LIMIT)
                .map_err(git_error)?
                .into_iter()
                .map(|c| ipc::GitCommit {
                    id: c.id,
                    short_id: c.short_id,
                    author: c.author,
                    email: c.email,
                    time: seconds(c.time),
                    subject: c.subject,
                    parents: to_u32(c.parents.len()),
                })
                .collect())
        })
        .await
    }

    /// Local and remote branches.
    pub async fn git_branches(&self) -> Result<Vec<ipc::GitBranch>, IpcError> {
        let git = self.git()?;
        blocking(move || {
            let (repo, _) = git.repo()?;
            Ok(repo
                .branches()
                .map_err(git_error)?
                .into_iter()
                .map(|b| ipc::GitBranch {
                    name: b.name,
                    remote: b.remote,
                    current: b.current,
                    upstream: b.upstream,
                    subject: b.subject,
                })
                .collect())
        })
        .await
    }

    /// Runs a Git write, then refreshes the status before returning.
    async fn git_write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Repo) -> Result<T, GitError> + Send + 'static,
    ) -> Result<T, IpcError> {
        let git = self.git()?;
        blocking(move || {
            let (repo, _) = git.repo()?;
            let result = f(&repo).map_err(git_error);
            git.refresh_now();
            result
        })
        .await
    }

    /// Checks out a branch. Refused while a document has unsaved changes,
    /// and by git when local changes would be overwritten.
    pub async fn git_checkout(&self, name: String, remote: bool) -> Result<(), IpcError> {
        let dirty: Vec<String> = self
            .open_docs()
            .into_iter()
            .filter(|d| d.lock().is_ok_and(|s| s.dirty()))
            .map(|d| d.rel_path.clone())
            .collect();
        if !dirty.is_empty() {
            return Err(IpcError::new(
                IpcErrorKind::Conflict,
                format!(
                    "Save or roll back the unsaved changes first: {}.",
                    dirty.join(", ")
                ),
            ));
        }
        self.git_write(move |repo| repo.checkout(&name, remote))
            .await
    }

    /// Creates a branch at HEAD (and checks it out if asked).
    pub async fn git_create_branch(&self, name: String, checkout: bool) -> Result<(), IpcError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(IpcError::new(
                IpcErrorKind::InvalidArgument,
                "Enter a branch name.",
            ));
        }
        self.git_write(move |repo| repo.create_branch(&name, checkout))
            .await
    }

    /// Deletes a local branch; an unmerged one only with `force`.
    pub async fn git_delete_branch(
        &self,
        name: String,
        force: bool,
    ) -> Result<ipc::GitDeleteResult, IpcError> {
        self.git_write(move |repo| match repo.delete_branch(&name, force) {
            Ok(()) => Ok(ipc::GitDeleteResult::Deleted),
            Err(GitError::Failed {
                kind: FailureKind::NotMerged,
                ..
            }) => Ok(ipc::GitDeleteResult::NotMerged),
            Err(err) => Err(err),
        })
        .await
    }

    fn repo_paths(&self, paths: Vec<String>) -> Result<Vec<String>, IpcError> {
        let (_, prefix) = self.git()?.repo()?;
        Ok(paths.iter().map(|p| to_repo(&prefix, p)).collect())
    }

    /// Stages project files.
    pub async fn git_stage(&self, paths: Vec<String>) -> Result<(), IpcError> {
        let paths = self.repo_paths(paths)?;
        self.git_write(move |repo| {
            repo.stage(&paths.iter().map(String::as_str).collect::<Vec<_>>())
        })
        .await
    }

    /// Unstages project files.
    pub async fn git_unstage(&self, paths: Vec<String>) -> Result<(), IpcError> {
        let paths = self.repo_paths(paths)?;
        self.git_write(move |repo| {
            repo.unstage(&paths.iter().map(String::as_str).collect::<Vec<_>>())
        })
        .await
    }

    /// Saves every unsaved document, stages `paths` again (the files
    /// checked in the commit window, so their saved text is included),
    /// then commits the index. Returns the new commit's abbreviated id.
    pub async fn git_commit(
        &self,
        message: String,
        amend: bool,
        paths: Vec<String>,
    ) -> Result<String, IpcError> {
        if message.trim().is_empty() {
            return Err(IpcError::new(
                IpcErrorKind::InvalidArgument,
                "Enter a commit message.",
            ));
        }
        for doc in self.open_docs() {
            let dirty = doc.lock().is_ok_and(|s| s.dirty());
            if dirty && !doc.read_only {
                self.save_document(doc.id).await?;
            }
        }
        let paths = self.repo_paths(paths)?;
        self.git_write(move |repo| {
            if !paths.is_empty() {
                repo.stage(&paths.iter().map(String::as_str).collect::<Vec<_>>())?;
            }
            repo.commit(&message, amend)
        })
        .await
    }

    /// The last commit's message (for Amend), if there is a commit.
    pub async fn git_last_commit_message(&self) -> Result<Option<String>, IpcError> {
        let git = self.git()?;
        blocking(move || {
            let (repo, _) = git.repo()?;
            repo.last_message().map_err(git_error)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_paths_between_repository_and_project() {
        assert_eq!(to_project("", "a/b.ts").as_deref(), Some("a/b.ts"));
        assert_eq!(
            to_project("web", "web/src/a.ts").as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(to_project("web", "webby/a.ts"), None);
        assert_eq!(to_project("web", "other/a.ts"), None);
        assert_eq!(to_repo("web", "src/a.ts"), "web/src/a.ts");
        assert_eq!(to_repo("", "src/a.ts"), "src/a.ts");
    }
}
