//! Finding server executables without guessing install locations: a given
//! directory (e.g. `node_modules/.bin`) or the `PATH`.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The first executable called `name` in the directories of `PATH` (on
/// Windows also trying the `PATHEXT` extensions).
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    find_in_path_list(name, std::env::var_os("PATH")?)
}

/// [`find_on_path`] for an explicit `PATH`-style list (tests, or a custom
/// search path).
pub fn find_in_path_list(name: &str, paths: OsString) -> Option<PathBuf> {
    std::env::split_paths(&paths).find_map(|dir| find_in_dir(&dir, name))
}

/// An executable called `name` directly inside `dir` (on Windows also
/// trying the `PATHEXT` extensions).
pub fn find_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    candidates(name)
        .into_iter()
        .map(|file| dir.join(file))
        .find(|path| is_executable(path))
}

/// Whether `path` is a file we may run.
pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Whether running `path` needs Node.js: a `.js`/`.mjs`/`.cjs` file, a
/// Windows `.cmd`/`.ps1` shim (npm and pnpm generate those for Node
/// packages), or a script whose `#!` line names `node`.
pub fn is_node_script(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("js" | "mjs" | "cjs") => return true,
        Some("cmd" | "ps1") if cfg!(windows) => return true,
        _ => {}
    }
    let mut head = [0_u8; 128];
    let Ok(read) = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) else {
        return false;
    };
    let head = &head[..read];
    if !head.starts_with(b"#!") {
        return false;
    }
    let first_line = head.split(|&b| b == b'\n').next().unwrap_or(head);
    String::from_utf8_lossy(first_line).contains("node")
}

fn candidates(name: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let mut names: Vec<String> = exts
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{name}{}", e.to_ascii_lowercase()))
            .collect();
        names.push(name.to_owned());
        names
    } else {
        vec![name.to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_exec(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn finds_programs_in_order_and_detects_node_scripts() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let file = if cfg!(windows) { "tool.cmd" } else { "tool" };
        let script = write_exec(second.path(), file, "#!/usr/bin/env node\nconsole.log(1)\n");
        let list = std::env::join_paths([first.path(), second.path()]).unwrap();
        assert_eq!(
            find_in_path_list("tool", list.clone()),
            Some(script.clone())
        );
        assert!(is_node_script(&script));
        assert_eq!(find_in_path_list("missing", list), None);

        let native = write_exec(first.path(), "native.bin", "\u{7f}ELF....");
        assert!(!is_node_script(&native));
        let shell = write_exec(first.path(), "run.sh", "#!/bin/sh\nexec x\n");
        assert!(!is_node_script(&shell));
    }

    #[cfg(unix)]
    #[test]
    fn skips_files_without_execute_permission() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plain"), "x").unwrap();
        assert_eq!(find_in_dir(dir.path(), "plain"), None);
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        assert_eq!(find_in_dir(dir.path(), "folder"), None);
    }
}
