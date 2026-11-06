//! Which shell to start.

use std::path::PathBuf;

/// The shell program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    /// Program.
    pub program: PathBuf,
    /// Arguments.
    pub args: Vec<String>,
}

/// The operating system family, for choosing defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux and other Unix systems.
    Unix,
    /// macOS.
    Mac,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this binary runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Unix
        }
    }
}

fn is_powershell(program: &std::path::Path) -> bool {
    program.file_stem().is_some_and(|stem| {
        let stem = stem.to_string_lossy().to_ascii_lowercase();
        stem == "pwsh" || stem == "powershell"
    })
}

/// Picks the shell: the user setting if given; else on Unix `$SHELL`, then
/// the account's login shell (`login_shell`, from its passwd entry), then
/// `/bin/sh` (on macOS as a login shell, so `PATH` matches a terminal); on
/// Windows `pwsh`, `powershell`, then `%ComSpec%`. `env` reads an
/// environment variable, `which` finds a program on `PATH`.
pub fn detect_shell(
    setting: Option<(PathBuf, Vec<String>)>,
    os: Os,
    env: &dyn Fn(&str) -> Option<String>,
    which: &dyn Fn(&str) -> Option<PathBuf>,
    login_shell: &dyn Fn() -> Option<PathBuf>,
) -> Shell {
    if let Some((program, args)) = setting {
        return Shell { program, args };
    }
    match os {
        Os::Windows => {
            let program = which("pwsh")
                .or_else(|| which("powershell"))
                .or_else(|| env("ComSpec").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("cmd.exe"));
            let args = if is_powershell(&program) {
                vec!["-NoLogo".into()]
            } else {
                Vec::new()
            };
            Shell { program, args }
        }
        Os::Mac | Os::Unix => {
            // `SHELL` is missing in some environments (CI runners, services,
            // a desktop session started without a login shell).
            let program = env("SHELL")
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .or_else(login_shell)
                .unwrap_or_else(|| PathBuf::from("/bin/sh"));
            let args = if os == Os::Mac {
                vec!["-l".into()]
            } else {
                Vec::new()
            };
            Shell { program, args }
        }
    }
}

/// The current account's login shell from its passwd entry (Unix with the
/// `native` feature), if it has one.
pub fn login_shell() -> Option<PathBuf> {
    #[cfg(all(unix, feature = "native"))]
    {
        passwd::login_shell()
    }
    #[cfg(not(all(unix, feature = "native")))]
    {
        None
    }
}

#[cfg(all(unix, feature = "native"))]
mod passwd {
    use std::ffi::{CStr, OsStr};
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    /// Largest buffer tried for the passwd entry's strings.
    const MAX_BUFFER: usize = 1 << 20;

    pub(super) fn login_shell() -> Option<PathBuf> {
        let mut buf: Vec<libc::c_char> = vec![0; 4096];
        loop {
            let mut entry = MaybeUninit::<libc::passwd>::uninit();
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: getuid has no preconditions. getpwuid_r writes the
            // entry into `entry` and its strings into `buf` (whose length is
            // passed), and sets `found` to `entry` on success or to null;
            // nothing is read below unless `found` is non-null.
            let code = unsafe {
                libc::getpwuid_r(
                    libc::getuid(),
                    entry.as_mut_ptr(),
                    buf.as_mut_ptr(),
                    buf.len(),
                    &raw mut found,
                )
            };
            if code == libc::ERANGE && buf.len() < MAX_BUFFER {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if code != 0 || found.is_null() {
                return None;
            }
            // SAFETY: `found` is non-null, so getpwuid_r filled in the entry.
            let shell = unsafe { (*found).pw_shell };
            if shell.is_null() {
                return None;
            }
            // SAFETY: `pw_shell` is a NUL-terminated string inside `buf`,
            // which is alive and not modified until we return.
            let bytes = unsafe { CStr::from_ptr(shell) }.to_bytes();
            return (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)));
        }
    }
}

/// Environment variables every terminal gets.
pub fn terminal_env() -> Vec<(String, String)> {
    vec![
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("TERM_PROGRAM".into(), "Ferrule".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn setting_wins() {
        let shell = detect_shell(
            Some(("/usr/bin/fish".into(), vec!["-i".into()])),
            Os::Unix,
            &env_from(&[("SHELL", "/bin/bash")]),
            &|_| None,
            &|| None,
        );
        assert_eq!(shell.program, PathBuf::from("/usr/bin/fish"));
        assert_eq!(shell.args, ["-i"]);
    }

    #[test]
    fn unix_and_mac_use_the_shell_variable() {
        let login = || Some(PathBuf::from("/usr/bin/zsh"));
        let bash = detect_shell(
            None,
            Os::Unix,
            &env_from(&[("SHELL", "/bin/bash")]),
            &|_| None,
            &login,
        );
        assert_eq!(bash.program, PathBuf::from("/bin/bash"));
        assert!(bash.args.is_empty());
        let mac = detect_shell(
            None,
            Os::Mac,
            &env_from(&[("SHELL", "/bin/zsh")]),
            &|_| None,
            &|| None,
        );
        assert_eq!(mac.program, PathBuf::from("/bin/zsh"));
        assert_eq!(mac.args, ["-l"]);
    }

    #[test]
    fn without_shell_the_login_shell_then_bin_sh() {
        let login = || Some(PathBuf::from("/usr/bin/zsh"));
        // SHELL unset or blank: the passwd entry's shell.
        for env in [env_from(&[]), env_from(&[("SHELL", " ")])] {
            let shell = detect_shell(None, Os::Unix, &env, &|_| None, &login);
            assert_eq!(shell.program, PathBuf::from("/usr/bin/zsh"));
        }
        let mac = detect_shell(None, Os::Mac, &env_from(&[]), &|_| None, &login);
        assert_eq!(mac.program, PathBuf::from("/usr/bin/zsh"));
        assert_eq!(mac.args, ["-l"]);
        // No passwd shell either: /bin/sh.
        for os in [Os::Unix, Os::Mac] {
            let shell = detect_shell(None, os, &env_from(&[]), &|_| None, &|| None);
            assert_eq!(shell.program, PathBuf::from("/bin/sh"));
        }
    }

    /// The real lookup (needs the passwd entry of the user running the
    /// tests, which CI runners and desktops have).
    #[cfg(all(unix, feature = "native"))]
    #[test]
    fn the_login_shell_comes_from_the_passwd_entry() {
        let shell = login_shell().expect("the account has a login shell");
        assert!(shell.is_absolute(), "{}", shell.display());
    }

    #[test]
    fn windows_prefers_pwsh_then_powershell_then_comspec() {
        let comspec = env_from(&[("ComSpec", r"C:\Windows\system32\cmd.exe")]);
        let pwsh = detect_shell(
            None,
            Os::Windows,
            &comspec,
            &|name| (name == "pwsh").then(|| PathBuf::from("pwsh.exe")),
            &|| None,
        );
        assert_eq!(pwsh.program, PathBuf::from("pwsh.exe"));
        assert_eq!(pwsh.args, ["-NoLogo"]);
        let ps = detect_shell(
            None,
            Os::Windows,
            &comspec,
            &|name| (name == "powershell").then(|| PathBuf::from("powershell.exe")),
            &|| None,
        );
        assert_eq!(ps.program, PathBuf::from("powershell.exe"));
        let cmd = detect_shell(None, Os::Windows, &comspec, &|_| None, &|| None);
        assert_eq!(cmd.program, PathBuf::from(r"C:\Windows\system32\cmd.exe"));
        assert!(cmd.args.is_empty());
    }
}
