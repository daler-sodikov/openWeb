//! [`Process`] on top of portable-pty, with cleanup of the whole process
//! tree: the process group on Unix (the shell is a session leader, so its
//! group id is its pid) and a kill-on-close Job Object on Windows.

use std::sync::{Mutex, MutexGuard, PoisonError};

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
    /// The shell's pid, which is also its process group id (Unix cleanup).
    #[cfg(unix)]
    pid: Option<u32>,
    /// The Job Object holding the shell and its children (Windows cleanup).
    #[cfg(windows)]
    job: Option<job::Job>,
}

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
    #[cfg(windows)]
    let job = child.as_raw_handle().and_then(job::Job::for_process);
    let killer = child.clone_killer();
    #[cfg(unix)]
    let pid = child.process_id();
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
                #[cfg(unix)]
                pid,
                #[cfg(windows)]
                job,
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
        #[cfg(unix)]
        if let Some(pid) = inner.pid {
            unix::hang_up_group(pid);
        }
        // Closing the job handle ends every process in the job.
        #[cfg(windows)]
        drop(inner.job.take());
        if let Err(err) = inner.killer.kill() {
            tracing::debug!(%err, "terminal shell already gone");
        }
        inner.master = None;
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

#[cfg(unix)]
mod unix {
    use std::time::Duration;

    /// SIGHUP to the shell's process group now, SIGKILL to whatever is left
    /// of it two seconds later.
    pub(super) fn hang_up_group(pid: u32) {
        let Ok(group) = libc::pid_t::try_from(pid) else {
            return;
        };
        // SAFETY: killpg only sends a signal; `group` is the shell's process
        // group id (portable-pty makes the shell a session leader with
        // setsid, so its pid is its group id). No memory is involved.
        unsafe {
            libc::killpg(group, libc::SIGHUP);
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            // SAFETY: as above. Checks first (signal 0) that the group still
            // exists; a group id is not reused while the group has members.
            unsafe {
                if libc::killpg(group, 0) == 0 {
                    libc::killpg(group, libc::SIGKILL);
                }
            }
        });
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::RawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// A Job Object that kills its processes when its handle closes.
    pub(super) struct Job(HANDLE);

    // SAFETY: a job handle is a kernel handle; it may be used and closed
    // from any thread.
    unsafe impl Send for Job {}
    // SAFETY: as above; the handle is only read.
    unsafe impl Sync for Job {}

    impl Job {
        /// Creates a kill-on-close job and puts `process` in it.
        pub(super) fn for_process(process: RawHandle) -> Option<Self> {
            // SAFETY: creates a new unnamed job; null attributes and name
            // are allowed.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Self(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).ok()?;
            // SAFETY: `info` lives across the call and `size` is its size.
            let limited = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size,
                )
            };
            // SAFETY: both handles are valid: the job above, the process from
            // the child that is still alive.
            let assigned = unsafe { AssignProcessToJobObject(job.0, process.cast()) };
            (limited != 0 && assigned != 0).then_some(job)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this value and closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
