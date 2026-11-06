//! Kill-on-close Job Objects.

use std::os::windows::io::RawHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};

/// A Job Object that kills its processes when its handle closes.
pub(crate) struct Tree(HANDLE);

// SAFETY: a job handle is a kernel handle; it may be used and closed from
// any thread.
unsafe impl Send for Tree {}
// SAFETY: as above; every use is a kernel call on the handle.
unsafe impl Sync for Tree {}

impl Tree {
    pub(crate) fn job(process: RawHandle) -> Option<Self> {
        // SAFETY: creates a new unnamed job; null attributes and name are
        // allowed.
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
        // SAFETY: both handles are valid: the job above, and the process of
        // a child that is still alive.
        let assigned = unsafe { AssignProcessToJobObject(job.0, process.cast()) };
        (limited != 0 && assigned != 0).then_some(job)
    }

    pub(crate) fn kill(&self) {
        // SAFETY: the handle is a job owned by this value.
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }

    pub(crate) fn hang_up(&self, _grace: Duration) {
        self.kill();
    }

    pub(crate) fn is_alive(&self) -> bool {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let Ok(size) = u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()) else {
            return false;
        };
        // SAFETY: `info` is a valid, writable buffer of `size` bytes; the
        // returned-length pointer may be null.
        let ok = unsafe {
            QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                (&raw mut info).cast(),
                size,
                std::ptr::null_mut(),
            )
        };
        ok != 0 && info.ActiveProcesses > 0
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed once; closing
        // the last handle ends the job's processes (kill on close).
        unsafe {
            CloseHandle(self.0);
        }
    }
}
