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

/// Picks the shell: the user setting if given, else `$SHELL` (Unix; macOS
/// as a login shell so `PATH` matches a terminal) or `pwsh`, `powershell`,
/// `%ComSpec%` (Windows). `env` reads an environment variable, `which`
/// finds a program on `PATH`.
pub fn detect_shell(
    setting: Option<(PathBuf, Vec<String>)>,
    os: Os,
    env: &dyn Fn(&str) -> Option<String>,
    which: &dyn Fn(&str) -> Option<PathBuf>,
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
            let program = env("SHELL").filter(|s| !s.trim().is_empty()).map_or_else(
                || PathBuf::from(if os == Os::Mac { "/bin/zsh" } else { "/bin/sh" }),
                PathBuf::from,
            );
            let args = if os == Os::Mac {
                vec!["-l".into()]
            } else {
                Vec::new()
            };
            Shell { program, args }
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
        );
        assert_eq!(shell.program, PathBuf::from("/usr/bin/fish"));
        assert_eq!(shell.args, ["-i"]);
    }

    #[test]
    fn unix_and_mac_use_the_shell_variable() {
        let bash = detect_shell(
            None,
            Os::Unix,
            &env_from(&[("SHELL", "/bin/bash")]),
            &|_| None,
        );
        assert_eq!(bash.program, PathBuf::from("/bin/bash"));
        assert!(bash.args.is_empty());
        let mac = detect_shell(None, Os::Mac, &env_from(&[]), &|_| None);
        assert_eq!(mac.program, PathBuf::from("/bin/zsh"));
        assert_eq!(mac.args, ["-l"]);
        let fallback = detect_shell(None, Os::Unix, &env_from(&[("SHELL", " ")]), &|_| None);
        assert_eq!(fallback.program, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn windows_prefers_pwsh_then_powershell_then_comspec() {
        let comspec = env_from(&[("ComSpec", r"C:\Windows\system32\cmd.exe")]);
        let pwsh = detect_shell(None, Os::Windows, &comspec, &|name| {
            (name == "pwsh").then(|| PathBuf::from("pwsh.exe"))
        });
        assert_eq!(pwsh.program, PathBuf::from("pwsh.exe"));
        assert_eq!(pwsh.args, ["-NoLogo"]);
        let ps = detect_shell(None, Os::Windows, &comspec, &|name| {
            (name == "powershell").then(|| PathBuf::from("powershell.exe"))
        });
        assert_eq!(ps.program, PathBuf::from("powershell.exe"));
        let cmd = detect_shell(None, Os::Windows, &comspec, &|_| None);
        assert_eq!(cmd.program, PathBuf::from(r"C:\Windows\system32\cmd.exe"));
        assert!(cmd.args.is_empty());
    }
}
