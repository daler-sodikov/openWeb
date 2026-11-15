//! Deleting a test project folder after `close_project`, and explaining why
//! when that fails.
//!
//! Windows refuses to delete a folder that a process uses as its working
//! directory or that has files open without delete sharing (os error 32).
//! Linux allows it, which would hide such a leak there; so on Linux the
//! same condition is checked directly first: no process may have its
//! working directory in the folder and this process may hold no open file
//! in it.
//!
//! A short retry with backoff (about 5 s) covers transient locks, such as
//! an antivirus or the search indexer scanning files right after a process
//! exits. The number of attempts is printed, so a persistent leak cannot
//! hide behind the retry.

#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the deletion is retried.
const RETRY_FOR: Duration = Duration::from_secs(5);

/// Deletes `dir` after a project in it was closed; `report` is what
/// `close_project` said was still running. Panics with diagnostics if the
/// folder is still in use.
pub(crate) fn remove_project_dir(dir: &Path, report: &[String]) {
    #[cfg(target_os = "linux")]
    {
        let holders = linux::holders(dir);
        assert!(
            holders.is_empty(),
            "still in use after close_project (report: {report:?}):\n{}",
            holders.join("\n")
        );
    }
    let started = Instant::now();
    let mut delay = Duration::from_millis(50);
    let mut attempts = 0;
    loop {
        attempts += 1;
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {
                eprintln!(
                    "deleted {} after {attempts} attempt(s) in {:?}",
                    dir.display(),
                    started.elapsed()
                );
                return;
            }
            Err(err) if started.elapsed() < RETRY_FOR => {
                eprintln!("attempt {attempts} to delete {}: {err}", dir.display());
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(800));
            }
            Err(err) => panic!(
                "cannot delete {} after {attempts} attempts over {:?}: {err}\n\
                 close_project reported still running: {report:?}\n{}",
                dir.display(),
                started.elapsed(),
                diagnostics(dir)
            ),
        }
    }
}

/// Files below `dir` (at most `limit`).
fn files_below(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if found.len() < limit {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(not(windows))]
fn diagnostics(dir: &Path) -> String {
    format!("files left: {:?}", files_below(dir, 20))
}

#[cfg(windows)]
fn diagnostics(dir: &Path) -> String {
    format!(
        "Jobs: this test's terminals use the in-memory backend (no Job Object); \
         processes of a terminal still running would be listed in the report above.\n\
         Processes started by this test or whose command line mentions the folder:\n{}\n\
         Restart Manager (processes holding files in the folder):\n{}",
        windows::processes(dir),
        windows::restart_manager(dir)
    )
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    /// Processes whose working directory is in `dir`, and files in `dir`
    /// this process holds open.
    pub(super) fn holders(dir: &Path) -> Vec<String> {
        let Ok(dir) = dir.canonicalize() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(pid) = name.to_str().filter(|n| n.parse::<u32>().is_ok()) else {
                    continue;
                };
                if let Ok(cwd) = std::fs::read_link(entry.path().join("cwd"))
                    && cwd.starts_with(&dir)
                {
                    let cmd = std::fs::read_to_string(entry.path().join("cmdline"))
                        .unwrap_or_default()
                        .replace('\0', " ");
                    found.push(format!(
                        "process {pid} has its working directory there: {cmd}"
                    ));
                }
            }
        }
        if let Ok(fds) = std::fs::read_dir("/proc/self/fd") {
            for fd in fds.flatten() {
                if let Ok(target) = std::fs::read_link(fd.path())
                    && target.starts_with(&dir)
                {
                    found.push(format!("this process holds {} open", target.display()));
                }
            }
        }
        found
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
        RmStartSession,
    };

    /// Processes started (directly or not) by this test process, and
    /// processes whose command line mentions `dir`, with parent and start
    /// time.
    pub(super) fn processes(dir: &Path) -> String {
        let script = r#"
$root = $env:FERRULE_DIAG_ROOT
$me = [int]$env:FERRULE_DIAG_PID
$all = @(Get-CimInstance Win32_Process)
$ours = @{}
$queue = New-Object System.Collections.Queue
$queue.Enqueue($me)
while ($queue.Count -gt 0) {
  $parent = $queue.Dequeue()
  foreach ($p in $all) {
    if ($p.ParentProcessId -eq $parent -and -not $ours.ContainsKey($p.ProcessId)) {
      $ours[$p.ProcessId] = $true
      $queue.Enqueue($p.ProcessId)
    }
  }
}
$all | Where-Object {
  $ours.ContainsKey($_.ProcessId) -or ($_.CommandLine -and $_.CommandLine.Contains($root))
} | ForEach-Object {
  "$($_.ProcessId) parent=$($_.ParentProcessId) started=$($_.CreationDate) $($_.Name): $($_.CommandLine)"
}
"#;
        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("FERRULE_DIAG_ROOT", dir)
            .env("FERRULE_DIAG_PID", std::process::id().to_string())
            .output()
        {
            Ok(out) => format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(err) => format!("(powershell failed: {err})"),
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// The Restart Manager's list of processes holding the files in `dir`
    /// (it reports files, not folders used as a working directory).
    pub(super) fn restart_manager(dir: &Path) -> String {
        let files: Vec<Vec<u16>> = super::files_below(dir, 500)
            .iter()
            .map(|p| wide(p))
            .collect();
        if files.is_empty() {
            return "no files left in the folder".to_owned();
        }
        let names: Vec<*const u16> = files.iter().map(|f| f.as_ptr()).collect();
        let Ok(count) = u32::try_from(names.len()) else {
            return "too many files".to_owned();
        };
        let mut session = 0_u32;
        let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: `session` and `key` are writable; the key buffer has the
        // documented size (CCH_RM_SESSION_KEY + 1).
        let started = unsafe { RmStartSession(&raw mut session, 0, key.as_mut_ptr()) };
        if started != 0 {
            return format!("RmStartSession failed: error {started}");
        }
        // SAFETY: `names` holds `count` pointers to NUL-terminated paths in
        // `files`, which outlives the call; no applications or services.
        let registered = unsafe {
            RmRegisterResources(
                session,
                count,
                names.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        let result = if registered == 0 {
            let mut infos = vec![RM_PROCESS_INFO::default(); 64];
            let mut needed = 0_u32;
            let mut returned = u32::try_from(infos.len()).unwrap_or(0);
            let mut reasons = 0_u32;
            // SAFETY: `infos` has room for `returned` entries; the other
            // pointers are writable u32s.
            let listed = unsafe {
                RmGetList(
                    session,
                    &raw mut needed,
                    &raw mut returned,
                    infos.as_mut_ptr(),
                    &raw mut reasons,
                )
            };
            if listed == 0 {
                let lines: Vec<String> = infos
                    .iter()
                    .take(returned as usize)
                    .map(|info| {
                        let name = String::from_utf16_lossy(&info.strAppName);
                        format!(
                            "{} {}",
                            info.Process.dwProcessId,
                            name.trim_end_matches('\0')
                        )
                    })
                    .collect();
                if lines.is_empty() {
                    "no process holds a file there".to_owned()
                } else {
                    lines.join("\n")
                }
            } else {
                format!("RmGetList failed: error {listed} ({needed} processes)")
            }
        } else {
            format!("RmRegisterResources failed: error {registered}")
        };
        // SAFETY: ends the session started above.
        unsafe {
            RmEndSession(session);
        }
        result
    }
}
