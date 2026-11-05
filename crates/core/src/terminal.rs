//! The project's terminals (`docs/ARCHITECTURE.md`, 4.9): IPC commands over
//! `ferrule-terminal`. Terminals start in the project root and end when the
//! project is replaced or the app quits.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrule_ipc::{self as ipc, IpcError, IpcErrorKind, TERMINAL_FLAGS};
use ferrule_terminal::{
    CellPos, Color, Frame, Key, Os, SelectionKind, SpawnSpec, Style, TermSize, Terminal,
    TerminalError, TerminalEvent, Terminals, detect_shell, links, terminal_env,
};

use crate::events::{EventSink, SessionEvent};
use crate::session::{Session, blocking};

/// Why terminals are missing when the app has no backend.
const UNAVAILABLE: &str =
    "This build has no terminal backend (built without the `native` feature).";

/// The terminal set of a project; events go to `sink` as session events.
pub(crate) fn project_terminals(
    backend: Option<Arc<dyn ferrule_terminal::Backend>>,
    sink: Arc<dyn EventSink>,
) -> Terminals {
    let forward: ferrule_terminal::EventSink = Arc::new(move |event| match event {
        TerminalEvent::Frame(frame) => sink.emit(SessionEvent::TerminalFrame(frame_to_ipc(&frame))),
        TerminalEvent::Exited { id, code } => {
            sink.emit(SessionEvent::TerminalExited(ipc::TerminalExited {
                id,
                code,
            }));
        }
    });
    Terminals::new(
        backend,
        ferrule_terminal::TerminalOptions::default(),
        forward,
    )
}

fn terminal_error(err: TerminalError) -> IpcError {
    match err {
        TerminalError::NotFound(_) => IpcError::new(IpcErrorKind::NotFound, err.to_string()),
        TerminalError::Unavailable => IpcError::new(IpcErrorKind::Conflict, UNAVAILABLE),
        TerminalError::Spawn { .. } | TerminalError::Io(_) => {
            IpcError::new(IpcErrorKind::Io, err.to_string())
        }
    }
}

fn to_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

fn size_from_ipc(size: ipc::TerminalSize) -> TermSize {
    TermSize::new(to_u16(size.cols), to_u16(size.rows))
}

fn cell_from_ipc(cell: ipc::TerminalCell) -> CellPos {
    CellPos {
        row: to_u16(cell.row),
        col: to_u16(cell.col),
    }
}

/// See [`ipc::TerminalRun`] for the encoding.
fn color_to_ipc(color: Color) -> u32 {
    match color {
        Color::Default => 0,
        Color::Indexed(i) => u32::from(i) + 1,
        Color::Rgb(r, g, b) => {
            0x0100_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        }
    }
}

fn flags_to_ipc(style: &Style) -> u32 {
    let f = TERMINAL_FLAGS;
    [
        (style.bold, f.bold),
        (style.dim, f.dim),
        (style.italic, f.italic),
        (style.underline, f.underline),
        (style.strike, f.strike),
        (style.inverse, f.inverse),
        (style.hidden, f.hidden),
        (style.selected, f.selected),
        (style.wide, f.wide),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |bits, (_, bit)| bits | bit)
}

fn frame_to_ipc(frame: &Frame) -> ipc::TerminalFrame {
    ipc::TerminalFrame {
        id: frame.id,
        seq: u32::try_from(frame.seq).unwrap_or(u32::MAX),
        size: ipc::TerminalSize {
            cols: u32::from(frame.size.cols),
            rows: u32::from(frame.size.rows),
        },
        full: frame.full,
        rows: frame
            .rows
            .iter()
            .map(|row| ipc::TerminalRow {
                row: u32::from(row.row),
                runs: row
                    .runs
                    .iter()
                    .map(|run| ipc::TerminalRun {
                        text: run.text.clone(),
                        fg: color_to_ipc(run.style.fg),
                        bg: color_to_ipc(run.style.bg),
                        flags: flags_to_ipc(&run.style),
                    })
                    .collect(),
            })
            .collect(),
        cursor: frame.cursor.map(|c| ipc::TerminalCell {
            row: u32::from(c.row),
            col: u32::from(c.col),
        }),
        display_offset: frame.display_offset,
        history: frame.history,
        title: frame.title.clone(),
    }
}

fn info_to_ipc(info: ferrule_terminal::TerminalInfo) -> ipc::TerminalInfo {
    ipc::TerminalInfo {
        id: info.id,
        title: info.title,
        shell: info.shell.to_string_lossy().into_owned(),
        cwd: info.cwd.to_string_lossy().into_owned(),
    }
}

/// The shell for new terminals: the `terminal.shellPath` setting, else the
/// environment's.
fn shell_spec(settings_file: &Path, cwd: PathBuf) -> SpawnSpec {
    let setting = match ferrule_settings::load_user_settings(settings_file) {
        Ok(settings) => settings.terminal_shell,
        Err(err) => {
            tracing::warn!(%err, "settings unreadable; detecting the shell");
            None
        }
    };
    let shell = detect_shell(
        setting,
        Os::current(),
        &|name| std::env::var(name).ok(),
        &ferrule_lsp_client::find_on_path,
    );
    SpawnSpec {
        program: shell.program,
        args: shell.args,
        cwd,
        env: terminal_env(),
    }
}

impl Session {
    /// The project's terminals, and why none can start if so.
    pub async fn terminals(&self) -> Result<ipc::TerminalList, IpcError> {
        let runtime = self.runtime()?;
        Ok(ipc::TerminalList {
            unavailable: (!runtime.terminals.available()).then(|| UNAVAILABLE.to_owned()),
            terminals: runtime
                .terminals
                .list()
                .into_iter()
                .map(info_to_ipc)
                .collect(),
        })
    }

    /// Starts a terminal in the project root.
    pub async fn open_terminal(
        &self,
        size: ipc::TerminalSize,
    ) -> Result<ipc::TerminalInfo, IpcError> {
        let runtime = self.runtime()?;
        let settings_file = self.settings_file.clone();
        blocking(move || {
            let spec = shell_spec(&settings_file, runtime.project.root().to_path_buf());
            tracing::info!(shell = ?spec.program, cwd = ?spec.cwd, "starting a terminal");
            runtime
                .terminals
                .open(&spec, size_from_ipc(size))
                .map(info_to_ipc)
                .map_err(terminal_error)
        })
        .await
    }

    fn terminal(&self, id: u32) -> Result<Arc<Terminal>, IpcError> {
        self.runtime()?.terminals.get(id).map_err(terminal_error)
    }

    /// Sends typed text.
    pub async fn terminal_input(&self, id: u32, text: String) -> Result<(), IpcError> {
        let terminal = self.terminal(id)?;
        blocking(move || terminal.input(&text).map_err(terminal_error)).await
    }

    /// Sends a key (arrows, function keys, Ctrl/Alt combinations).
    pub async fn terminal_key(&self, id: u32, key: ipc::TerminalKey) -> Result<(), IpcError> {
        let terminal = self.terminal(id)?;
        let key = Key {
            key: key.key,
            ctrl: key.ctrl,
            alt: key.alt,
            shift: key.shift,
        };
        blocking(move || terminal.key(&key).map_err(terminal_error)).await
    }

    /// Pastes text (bracketed when the program asked for it).
    pub async fn terminal_paste(&self, id: u32, text: String) -> Result<(), IpcError> {
        let terminal = self.terminal(id)?;
        blocking(move || terminal.paste(&text).map_err(terminal_error)).await
    }

    /// Resizes the grid and the PTY.
    pub async fn terminal_resize(&self, id: u32, size: ipc::TerminalSize) -> Result<(), IpcError> {
        let terminal = self.terminal(id)?;
        blocking(move || terminal.resize(size_from_ipc(size)).map_err(terminal_error)).await
    }

    /// Scrolls the view by `lines` (positive: back into the scrollback).
    pub async fn terminal_scroll(&self, id: u32, lines: i32) -> Result<(), IpcError> {
        self.terminal(id)?.scroll(lines);
        Ok(())
    }

    /// Selects from `anchor` to `head`.
    pub async fn terminal_select(
        &self,
        id: u32,
        anchor: ipc::TerminalCell,
        head: ipc::TerminalCell,
        kind: ipc::TerminalSelectionKind,
    ) -> Result<(), IpcError> {
        let kind = match kind {
            ipc::TerminalSelectionKind::Simple => SelectionKind::Simple,
            ipc::TerminalSelectionKind::Word => SelectionKind::Word,
            ipc::TerminalSelectionKind::Line => SelectionKind::Line,
        };
        self.terminal(id)?
            .select(cell_from_ipc(anchor), cell_from_ipc(head), kind);
        Ok(())
    }

    /// Clears the selection.
    pub async fn terminal_clear_selection(&self, id: u32) -> Result<(), IpcError> {
        self.terminal(id)?.clear_selection();
        Ok(())
    }

    /// The selected text, for copying.
    pub async fn terminal_selection_text(&self, id: u32) -> Result<Option<String>, IpcError> {
        Ok(self.terminal(id)?.selection_text())
    }

    /// The file link under a cell (`src/a.ts:12:5`), if it names an
    /// existing file. Relative paths resolve against the project root.
    pub async fn terminal_link_at(
        &self,
        id: u32,
        cell: ipc::TerminalCell,
    ) -> Result<Option<ipc::NavTarget>, IpcError> {
        let runtime = self.runtime()?;
        let text = runtime
            .terminals
            .get(id)
            .map_err(terminal_error)?
            .row_text(to_u16(cell.row));
        let col = usize::from(to_u16(cell.col));
        blocking(move || {
            let project = &runtime.project;
            let Some(link) = links::link_at(&text, col)
                .and_then(|found| links::resolve(&found, &[project.root()]))
            else {
                return Ok(None);
            };
            let rel = project.relativize(&link.path);
            Ok(Some(ipc::NavTarget {
                in_project: rel.is_some(),
                path: rel.unwrap_or_else(|| link.path.to_string_lossy().into_owned()),
                line: link.line.map_or(0, |l| l.saturating_sub(1)),
                column: link.column.map_or(0, |c| c.saturating_sub(1)),
                preview: String::new(),
            }))
        })
        .await
    }

    /// Renames a tab (empty: back to the program's title).
    pub async fn rename_terminal(
        &self,
        id: u32,
        title: String,
    ) -> Result<ipc::TerminalInfo, IpcError> {
        self.runtime()?
            .terminals
            .rename(id, &title)
            .map(info_to_ipc)
            .map_err(terminal_error)
    }

    /// Closes a terminal and ends its processes.
    pub async fn close_terminal(&self, id: u32) -> Result<(), IpcError> {
        let runtime = self.runtime()?;
        blocking(move || runtime.terminals.close(id).map_err(terminal_error)).await
    }

    /// The view drew frame `seq`; the next one may come.
    pub async fn terminal_ack(&self, id: u32, seq: u32) -> Result<(), IpcError> {
        self.terminal(id)?.ack(u64::from(seq));
        Ok(())
    }

    /// Ends every terminal (the app is quitting).
    pub fn shutdown_terminals(&self) {
        if let Ok(runtime) = self.runtime() {
            runtime.terminals.close_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_flags_encode_as_documented() {
        assert_eq!(color_to_ipc(Color::Default), 0);
        assert_eq!(color_to_ipc(Color::Indexed(0)), 1);
        assert_eq!(color_to_ipc(Color::Indexed(255)), 256);
        assert_eq!(color_to_ipc(Color::Rgb(0x12, 0x34, 0x56)), 0x0112_3456);
        let style = Style {
            bold: true,
            inverse: true,
            ..Style::default()
        };
        assert_eq!(
            flags_to_ipc(&style),
            TERMINAL_FLAGS.bold | TERMINAL_FLAGS.inverse
        );
    }
}
