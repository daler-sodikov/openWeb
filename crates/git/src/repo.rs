//! Running `git`: discovery and every operation Ferrule uses. Only local
//! subcommands; prompts are disabled and no credentials are read.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::blame::{BlameLine, parse_blame};
use crate::branches::{BRANCH_FORMAT, Branch, parse_branches};
use crate::error::{FailureKind, GitError, classify};
use crate::log::{Commit, LOG_FORMAT, parse_log};
use crate::status::{RepoStatus, parse_status};

/// The `git` executable.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
}

/// Output of a finished command.
struct Output {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl Git {
    /// Uses `program` (a path, or `git` to search `PATH`).
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    fn command(&self, dir: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(dir)
            .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
            .args(args)
            // Stable messages to recognize failures; no prompts, ever.
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_PAGER", "cat")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console window flashes for each call.
            cmd.creation_flags(0x0800_0000);
        }
        cmd
    }

    fn output(&self, mut cmd: Command, stdin: Option<&[u8]>) -> Result<Output, GitError> {
        let not_installed = |err: std::io::Error| GitError::NotInstalled {
            program: self.program.display().to_string(),
            message: err.to_string(),
        };
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().map_err(not_installed)?;
        if let Some(input) = stdin
            && let Some(mut pipe) = child.stdin.take()
        {
            // A thread, so a large input cannot deadlock against the
            // output pipes.
            let input = input.to_vec();
            std::thread::spawn(move || {
                let _ = pipe.write_all(&input);
            });
        }
        let out = child.wait_with_output().map_err(not_installed)?;
        Ok(Output {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// The git version (`git version 2.45.0`), to check it runs.
    pub fn version(&self) -> Result<String, GitError> {
        let cmd = self.command(Path::new("."), &["--version"]);
        let out = self.output(cmd, None)?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// The repository containing `dir`, if any.
    pub fn discover(&self, dir: &Path) -> Result<Option<Repo>, GitError> {
        let cmd = self.command(dir, &["rev-parse", "--show-toplevel", "--show-prefix"]);
        let out = self.output(cmd, None)?;
        if !out.ok {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.lines();
        let root = lines
            .next()
            .ok_or_else(|| GitError::Parse("rev-parse printed nothing".into()))?;
        let prefix = lines.next().unwrap_or("").trim_end_matches('/').to_owned();
        Ok(Some(Repo {
            git: self.clone(),
            root: PathBuf::from(root),
            prefix,
        }))
    }
}

/// A repository (its worktree root).
#[derive(Debug, Clone)]
pub struct Repo {
    git: Git,
    root: PathBuf,
    prefix: String,
}

fn failed(out: &Output) -> GitError {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if out.stderr.trim().is_empty() {
        stdout.trim().to_owned()
    } else {
        out.stderr.trim().to_owned()
    };
    GitError::Failed {
        kind: classify(&format!("{}\n{stdout}", out.stderr)),
        message: text,
    }
}

impl Repo {
    /// Worktree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory `discover` was called with, relative to the root
    /// (`""` at the root, else `a/b`).
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    fn run_with(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        let out = self.git.output(self.git.command(&self.root, args), stdin)?;
        if out.ok {
            Ok(out.stdout)
        } else {
            Err(failed(&out))
        }
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        self.run_with(args, None)
    }

    fn text(&self, args: &[&str]) -> Result<String, GitError> {
        Ok(String::from_utf8_lossy(&self.run(args)?).into_owned())
    }

    /// The status: branch, changed, untracked and ignored files.
    pub fn status(&self) -> Result<RepoStatus, GitError> {
        let mut cmd = self.git.command(
            &self.root,
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--ignored=matching",
            ],
        );
        // A read: never take the index lock (an editor polls often).
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
        let out = self.git.output(cmd, None)?;
        if !out.ok {
            return Err(failed(&out));
        }
        parse_status(&out.stdout)
    }

    /// The index version of a file (`None` if not in the index).
    pub fn index_text(&self, path: &str) -> Result<Option<String>, GitError> {
        self.show(&format!(":{path}"))
    }

    /// The HEAD version of a file (`None` if not in HEAD).
    pub fn head_text(&self, path: &str) -> Result<Option<String>, GitError> {
        self.show(&format!("HEAD:{path}"))
    }

    fn show(&self, spec: &str) -> Result<Option<String>, GitError> {
        match self.run(&["show", spec]) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(GitError::Failed { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Blame of `path` with `contents` as its current text (unsaved edits
    /// included), one entry per line.
    pub fn blame(&self, path: &str, contents: &str) -> Result<Vec<BlameLine>, GitError> {
        let out = self.run_with(
            &["blame", "--porcelain", "--contents", "-", "--", path],
            Some(contents.as_bytes()),
        )?;
        parse_blame(&String::from_utf8_lossy(&out))
    }

    /// Up to `limit` commits of the current branch, or of `path` (following
    /// renames).
    pub fn log(&self, path: Option<&str>, limit: usize) -> Result<Vec<Commit>, GitError> {
        let limit = format!("-n{limit}");
        let mut args = vec!["log", LOG_FORMAT, limit.as_str()];
        if let Some(path) = path {
            args.extend(["--follow", "--", path]);
        }
        match self.text(&args) {
            Ok(out) => Ok(parse_log(&out)),
            // No commits yet.
            Err(GitError::Failed { message, .. })
                if message.contains("does not have any commits") =>
            {
                Ok(Vec::new())
            }
            Err(err) => Err(err),
        }
    }

    /// Local and remote branches.
    pub fn branches(&self) -> Result<Vec<Branch>, GitError> {
        let out = self.text(&["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"])?;
        Ok(parse_branches(&out))
    }

    /// Checks out a local branch, or a remote one as a new tracking branch.
    /// Git refuses when local changes would be overwritten.
    pub fn checkout(&self, name: &str, remote: bool) -> Result<(), GitError> {
        if remote {
            self.run(&["switch", "--track", name])?;
        } else {
            self.run(&["switch", name])?;
        }
        Ok(())
    }

    /// Creates a branch at HEAD and optionally checks it out.
    pub fn create_branch(&self, name: &str, checkout: bool) -> Result<(), GitError> {
        if checkout {
            self.run(&["switch", "-c", name])?;
        } else {
            self.run(&["branch", "--", name])?;
        }
        Ok(())
    }

    /// Deletes a local branch; refuses an unmerged one unless `force`.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<(), GitError> {
        self.run(&["branch", if force { "-D" } else { "-d" }, "--", name])?;
        Ok(())
    }

    /// Stages files (new, changed or deleted).
    pub fn stage(&self, paths: &[&str]) -> Result<(), GitError> {
        let mut args = vec!["add", "--all", "--"];
        args.extend(paths);
        self.run(&args)?;
        Ok(())
    }

    /// Unstages files (back to HEAD in the index; before the first commit,
    /// removes them from the index).
    pub fn unstage(&self, paths: &[&str]) -> Result<(), GitError> {
        let has_head = self.run(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
        let mut args = if has_head {
            vec!["restore", "--staged", "--"]
        } else {
            vec!["rm", "--cached", "-r", "-q", "--"]
        };
        args.extend(paths);
        self.run(&args)?;
        Ok(())
    }

    /// Commits the index. Hooks run; a failing hook is reported as
    /// [`FailureKind::HookFailed`].
    pub fn commit(&self, message: &str, amend: bool) -> Result<String, GitError> {
        let mut args = vec!["commit", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        match self.run_with(&args, Some(message.as_bytes())) {
            Ok(_) => Ok(self
                .text(&["rev-parse", "--short", "HEAD"])?
                .trim()
                .to_owned()),
            Err(GitError::Failed {
                kind: FailureKind::Other,
                message,
            }) if self.has_commit_hooks() => Err(GitError::Failed {
                kind: FailureKind::HookFailed,
                message,
            }),
            Err(err) => Err(err),
        }
    }

    fn has_commit_hooks(&self) -> bool {
        let Ok(dir) = self.text(&["rev-parse", "--git-path", "hooks"]) else {
            return false;
        };
        let dir = self.root.join(dir.trim());
        ["pre-commit", "commit-msg", "prepare-commit-msg"]
            .iter()
            .any(|hook| dir.join(hook).is_file())
    }

    /// The message of the last commit (for Amend).
    pub fn last_message(&self) -> Result<Option<String>, GitError> {
        match self.text(&["log", "-1", "--format=%B"]) {
            Ok(text) => Ok(Some(text.trim_end().to_owned())),
            Err(GitError::Failed { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }
}
