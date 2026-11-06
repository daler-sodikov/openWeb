//! Process groups.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) struct Tree {
    group: libc::pid_t,
    /// Set once the group was seen empty: its id may be reused by an
    /// unrelated group later, so no more signals go to it.
    ended: AtomicBool,
}

fn signal(group: libc::pid_t, sig: libc::c_int) -> bool {
    // SAFETY: killpg only sends a signal (or with 0 checks that the group
    // exists); no memory is involved. `group` is positive, so it names a
    // group, never "every process".
    unsafe { libc::killpg(group, sig) == 0 }
}

impl Tree {
    pub(crate) fn group(pid: u32) -> Option<Self> {
        let group = libc::pid_t::try_from(pid).ok().filter(|g| *g > 0)?;
        Some(Self {
            group,
            ended: AtomicBool::new(false),
        })
    }

    pub(crate) fn kill(&self) {
        if self.is_alive() {
            signal(self.group, libc::SIGKILL);
        }
    }

    pub(crate) fn hang_up(&self, grace: Duration) {
        if !self.is_alive() {
            return;
        }
        signal(self.group, libc::SIGHUP);
        let group = self.group;
        let spawned = std::thread::Builder::new()
            .name("process-hang-up".into())
            .spawn(move || {
                std::thread::sleep(grace);
                // A group id is not reused while the group has members.
                if signal(group, 0) {
                    signal(group, libc::SIGKILL);
                }
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "no thread for the delayed kill; killing now");
            signal(self.group, libc::SIGKILL);
        }
    }

    pub(crate) fn is_alive(&self) -> bool {
        if self.ended.load(Ordering::Acquire) {
            return false;
        }
        let alive = signal(self.group, 0);
        if !alive {
            self.ended.store(true, Ordering::Release);
        }
        alive
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.kill();
    }
}
