//! What a terminal needs from the platform: a process attached to a PTY and
//! a screen (VT emulator + grid). The native implementations live in
//! `native` (portable-pty + alacritty_terminal); tests use `fake`.

use std::io::{Read, Write};
use std::path::PathBuf;

use crate::model::{CellPos, CursorPos, Dirty, Modes, RenderRow, SelectionKind, TermSize};

/// Errors starting or driving a terminal.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// The shell could not be started.
    #[error("cannot start {program}: {message}")]
    Spawn {
        /// The program.
        program: String,
        /// Why.
        message: String,
    },
    /// No terminal backend is built into this binary.
    #[error("terminals are not available in this build")]
    Unavailable,
    /// The terminal does not exist (closed).
    #[error("terminal {0} is not open")]
    NotFound(u32),
    /// Writing to or resizing the PTY failed.
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

/// What to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The program (a shell).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// The process side of a running terminal. Implementations synchronize
/// internally; every method may be called from any thread.
pub trait Process: Send + Sync {
    /// Resizes the PTY (the program receives SIGWINCH / a console resize).
    fn resize(&self, size: TermSize) -> Result<(), TerminalError>;
    /// Ends the shell and everything it started (process group on Unix,
    /// Job Object on Windows). Idempotent; must not block for long.
    fn terminate(&self);
    /// The exit code once the shell has exited (`Some(None)`: exited, code
    /// unknown); `None` while it runs.
    fn exit_status(&self) -> Option<Option<i32>>;
    /// Whether the shell or anything it started still runs (by default:
    /// the shell).
    fn running(&self) -> bool {
        self.exit_status().is_none()
    }
}

/// A started shell: its output, its input and its process.
pub struct Spawned {
    /// The PTY output (read on a dedicated thread; blocking).
    pub reader: Box<dyn Read + Send>,
    /// The PTY input.
    pub writer: Box<dyn Write + Send>,
    /// The process.
    pub process: Box<dyn Process>,
}

/// Something the screen asks for while parsing output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenEvent {
    /// The program set the window title.
    Title(String),
    /// Bytes to send back to the program (answers to queries such as the
    /// cursor position report).
    Reply(Vec<u8>),
}

/// The terminal emulator: parses output and keeps the screen and
/// scrollback. Accessed under the session's lock.
pub trait Screen: Send {
    /// Parses program output.
    fn feed(&mut self, bytes: &[u8]) -> Vec<ScreenEvent>;
    /// Changes the grid size (reflowing as the emulator does).
    fn resize(&mut self, size: TermSize);
    /// What changed since the last call (and forgets it).
    fn take_dirty(&mut self) -> Dirty;
    /// One viewport row, styled.
    fn render_row(&self, row: u16) -> RenderRow;
    /// The text of one viewport row, exactly one character per column
    /// (wide characters are followed by a space).
    fn row_text(&self, row: u16) -> String;
    /// The cursor, if it is visible in the viewport.
    fn cursor(&self) -> Option<CursorPos>;
    /// Scrolls the viewport; positive is up (into the history).
    fn scroll(&mut self, lines: i32);
    /// Back to the bottom (following output).
    fn scroll_to_bottom(&mut self);
    /// Lines scrolled back, and lines of history.
    fn scroll_state(&self) -> (u32, u32);
    /// Input-relevant modes.
    fn modes(&self) -> Modes;
    /// Selects from `anchor` to `head` (viewport cells).
    fn select(&mut self, anchor: CellPos, head: CellPos, kind: SelectionKind);
    /// Clears the selection.
    fn clear_selection(&mut self);
    /// The selected text, if any.
    fn selection_text(&self) -> Option<String>;
}

/// Creates processes and screens.
pub trait Backend: Send + Sync {
    /// Starts `spec` attached to a new PTY of `size`.
    fn spawn(&self, spec: &SpawnSpec, size: TermSize) -> Result<Spawned, TerminalError>;
    /// A new screen of `size` with `scrollback` lines of history.
    fn screen(&self, size: TermSize, scrollback: usize) -> Box<dyn Screen>;
}
