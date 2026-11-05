//! The user settings file (`settings.json` in the app config directory).
//!
//! Only the keys used so far are read; unknown keys are ignored so the file
//! can grow without breaking older builds:
//!
//! ```json
//! {
//!   "typescript": { "serverPath": "/opt/tools/typescript-language-server" },
//!   "terminal": { "shellPath": "/bin/bash", "shellArgs": ["-l"] }
//! }
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Settings from the user settings file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserSettings {
    /// `typescript.serverPath`: the TypeScript language server executable.
    pub typescript_server_path: Option<PathBuf>,
    /// `terminal.shellPath` and `terminal.shellArgs`: the shell new
    /// terminals start (default: detected from the environment).
    pub terminal_shell: Option<(PathBuf, Vec<String>)>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    typescript: TypeScript,
    terminal: Terminal,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TypeScript {
    server_path: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Terminal {
    shell_path: Option<String>,
    shell_args: Vec<String>,
}

/// Reads the settings file. A missing file gives the defaults; an unreadable
/// or invalid file is an error with a message for the user.
pub fn load_user_settings(path: &Path) -> Result<UserSettings, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UserSettings::default());
        }
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let file: File = serde_json::from_str(&text)
        .map_err(|err| format!("{} is not valid settings JSON: {err}", path.display()))?;
    Ok(UserSettings {
        typescript_server_path: file
            .typescript
            .server_path
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from),
        terminal_shell: file
            .terminal
            .shell_path
            .filter(|p| !p.trim().is_empty())
            .map(|p| (PathBuf::from(p), file.terminal.shell_args)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_known_keys_and_ignores_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(load_user_settings(&path), Ok(UserSettings::default()));
        std::fs::write(
            &path,
            r#"{ "editor": { "x": 1 }, "typescript": { "serverPath": "/opt/ts-ls", "other": true } }"#,
        )
        .unwrap();
        assert_eq!(
            load_user_settings(&path).unwrap().typescript_server_path,
            Some(PathBuf::from("/opt/ts-ls"))
        );
        std::fs::write(&path, r#"{ "typescript": { "serverPath": "  " } }"#).unwrap();
        assert_eq!(
            load_user_settings(&path).unwrap().typescript_server_path,
            None
        );
    }

    #[test]
    fn reads_the_terminal_shell() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "terminal": { "shellPath": "/bin/bash", "shellArgs": ["-l", "-i"] } }"#,
        )
        .unwrap();
        assert_eq!(
            load_user_settings(&path).unwrap().terminal_shell,
            Some((PathBuf::from("/bin/bash"), vec!["-l".into(), "-i".into()]))
        );
        std::fs::write(&path, r#"{ "terminal": { "shellArgs": ["-l"] } }"#).unwrap();
        assert_eq!(load_user_settings(&path).unwrap().terminal_shell, None);
    }

    #[test]
    fn reports_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = load_user_settings(&path).unwrap_err();
        assert!(err.contains("not valid settings JSON"), "{err}");
    }
}
