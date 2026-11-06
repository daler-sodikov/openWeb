//! Child process trees that end together, so stopping something Ferrule
//! started (a terminal's shell, a language server, later a run
//! configuration) never leaves orphans holding files or folders open.
//!
//! - Unix: the members of a process group. The child must be started as a
//!   group leader (`CommandExt::process_group(0)`, or `setsid` as
//!   portable-pty does), so its pid is the group id; what it starts stays in
//!   the group unless it moves itself out.
//! - Windows: the processes of a kill-on-close Job Object. Processes the
//!   child starts after it was assigned join the job.

use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix::Tree;
#[cfg(windows)]
use windows::Tree;

/// How often [`ProcessTree::wait_gone`] looks again.
const POLL: Duration = Duration::from_millis(20);

/// A child process and everything it starts, ended as one unit. Dropping it
/// ends the processes that are still running.
pub struct ProcessTree(Tree);

impl std::fmt::Debug for ProcessTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProcessTree").finish_non_exhaustive()
    }
}

impl ProcessTree {
    /// The process group led by `pid` (Unix). `None` if `pid` is not a
    /// valid process id.
    #[cfg(unix)]
    pub fn group(pid: u32) -> Option<Self> {
        Tree::group(pid).map(Self)
    }

    /// A new kill-on-close Job Object holding `process` (Windows). `None`
    /// if the job cannot be created or the process cannot join it.
    #[cfg(windows)]
    pub fn job(process: std::os::windows::io::RawHandle) -> Option<Self> {
        Tree::job(process).map(Self)
    }

    /// Ends every process of the tree now (Unix `SIGKILL` to the group,
    /// Windows `TerminateJobObject`).
    pub fn kill(&self) {
        self.0.kill();
    }

    /// Asks the tree to end as a closed terminal does (Unix `SIGHUP`) and
    /// kills what is left after `grace`, without blocking. On Windows the
    /// same as [`ProcessTree::kill`].
    pub fn hang_up(&self, grace: Duration) {
        self.0.hang_up(grace);
    }

    /// Whether a process of the tree still runs.
    pub fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    /// Blocks until no process of the tree runs, at most `timeout`; returns
    /// whether the tree is gone.
    pub fn wait_gone(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !self.is_alive() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const LONG: Duration = Duration::from_secs(10);

    /// A shell that starts a grandchild and then waits: the tree to end.
    #[cfg(unix)]
    fn spawn_tree(script: &str) -> (std::process::Child, ProcessTree) {
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .process_group(0)
            .spawn()
            .unwrap();
        let tree = ProcessTree::group(child.id()).unwrap();
        (child, tree)
    }

    #[cfg(unix)]
    #[test]
    fn kill_ends_the_child_and_what_it_started() {
        let (mut child, tree) = spawn_tree("sleep 1000 & sleep 1000");
        assert!(tree.is_alive());
        tree.kill();
        let _ = child.wait();
        assert!(tree.wait_gone(LONG), "a process of the group survived");
    }

    #[cfg(unix)]
    #[test]
    fn hang_up_kills_what_ignores_it_after_the_grace_time() {
        // The grandchild ignores SIGHUP (like a program started with nohup).
        let (mut child, tree) = spawn_tree("trap '' HUP; sleep 1000 & wait");
        std::thread::sleep(Duration::from_millis(200));
        tree.hang_up(Duration::from_millis(500));
        std::thread::sleep(Duration::from_millis(100));
        assert!(tree.is_alive(), "nothing in the group ends on SIGHUP");
        // The shell is our child: reap it so it does not linger as a zombie
        // (which still counts as a group member).
        let _ = child.wait();
        assert!(tree.wait_gone(LONG), "SIGKILL after the grace time");
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_tree_ends_it() {
        let (mut child, tree) = spawn_tree("sleep 1000 & sleep 1000");
        let pid = child.id();
        drop(tree);
        let _ = child.wait();
        let check = ProcessTree::group(pid).unwrap();
        assert!(check.wait_gone(LONG));
    }

    #[cfg(windows)]
    #[test]
    fn kill_ends_the_child_and_what_it_started() {
        use std::os::windows::io::AsRawHandle;
        // cmd waits for ping, which it started after joining the job.
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 1000 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let tree = ProcessTree::job(child.as_raw_handle()).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(tree.is_alive());
        tree.kill();
        let _ = child.wait();
        assert!(tree.wait_gone(LONG), "a process of the job survived");
    }
}
