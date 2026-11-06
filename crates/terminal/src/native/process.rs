//! [`Process`] on top of portable-pty, with cleanup of the whole process
//! tree ([`ProcessTree`]): the process group on Unix (portable-pty makes the
//! shell a session leader with `setsid`, so its group id is its pid) and a
//! kill-on-close Job Object on Windows.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ferrule_process::ProcessTree;

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::backend::{Process, SpawnSpec, Spawned, TerminalError};
use crate::model::TermSize;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

struct Inner {
    /// Dropped on terminate: closes the PTY (ends ConPTY output).
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exit: Option<Option<i32>>,
    terminated: bool,
    /// The shell and everything it starts; `None` if it could not be set
    /// up (the shell itself is still killed).
    tree: Option<ProcessTree>,
}

/// After a hang-up, how long programs get to exit before they are killed.
const HANG_UP_GRACE: Duration = Duration::from_secs(2);

/// A shell on a PTY.
pub struct PtyProcess {
    inner: Mutex<Inner>,
}

/// Starts `spec` on a new PTY.
pub fn spawn(spec: &SpawnSpec, size: TermSize) -> Result<Spawned, TerminalError> {
    let program = spec.program.display().to_string();
    let fail = |message: String| TerminalError::Spawn {
        program: program.clone(),
        message,
    };
    let pair = native_pty_system()
        .openpty(pty_size(size))
        .map_err(|e| fail(e.to_string()))?;
    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    command.cwd(&spec.cwd);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| fail(e.to_string()))?;
    // The child has its own handle to the slave side.
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| fail(e.to_string()))?;
    let writer = pair.master.take_writer().map_err(|e| fail(e.to_string()))?;
    #[cfg(unix)]
    let tree = child.process_id().and_then(ProcessTree::group);
    #[cfg(windows)]
    let tree = child.as_raw_handle().and_then(ProcessTree::job);
    if tree.is_none() {
        tracing::warn!(%program, "cannot track the shell's processes; only the shell is ended on close");
    }
    let killer = child.clone_killer();
    Ok(Spawned {
        reader,
        writer,
        process: Box::new(PtyProcess {
            inner: Mutex::new(Inner {
                master: Some(pair.master),
                child,
                killer,
                exit: None,
                terminated: false,
                tree,
            }),
        }),
    })
}

impl Process for PtyProcess {
    fn resize(&self, size: TermSize) -> Result<(), TerminalError> {
        match lock(&self.inner).master.as_ref() {
            Some(master) => master
                .resize(pty_size(size))
                .map_err(|e| TerminalError::Io(e.to_string())),
            None => Ok(()),
        }
    }

    fn terminate(&self) {
        let mut inner = lock(&self.inner);
        if inner.terminated {
            return;
        }
        inner.terminated = true;
        // Unix: SIGHUP to the group, SIGKILL to what is left later.
        // Windows: ends every process in the job.
        if let Some(tree) = &inner.tree {
            tree.hang_up(HANG_UP_GRACE);
        }
        if let Err(err) = inner.killer.kill() {
            tracing::debug!(%err, "terminal shell already gone");
        }
        inner.master = None;
    }

    fn running(&self) -> bool {
        // Reaps the shell if it exited: a zombie still counts as a member
        // of its process group.
        let shell_running = self.exit_status().is_none();
        shell_running
            || lock(&self.inner)
                .tree
                .as_ref()
                .is_some_and(ProcessTree::is_alive)
    }

    fn exit_status(&self) -> Option<Option<i32>> {
        let mut inner = lock(&self.inner);
        if let Some(exit) = inner.exit {
            return Some(exit);
        }
        let exit = match inner.child.try_wait() {
            Ok(Some(status)) => Some(i32::try_from(status.exit_code()).ok()),
            Ok(None) => None,
            Err(_) => Some(None),
        };
        inner.exit = exit;
        exit
    }
}
