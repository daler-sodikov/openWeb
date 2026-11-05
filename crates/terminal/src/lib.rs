//! Integrated terminal (`docs/ARCHITECTURE.md`, 4.9): the session (threads,
//! batching, input), key and paste encoding, file links in output and the
//! shell choice. The PTY and the VT emulator are behind [`Backend`]; the
//! real one ([`native`], portable-pty + alacritty_terminal) is built with
//! the `native` feature, so everything else builds and tests without it.

mod backend;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod keys;
pub mod links;
mod model;
#[cfg(feature = "native")]
pub mod native;
mod pacer;
mod session;
mod shell;

pub use backend::{Backend, Process, Screen, ScreenEvent, SpawnSpec, Spawned, TerminalError};
pub use keys::{Key, encode_key, encode_paste};
pub use model::{
    CellPos, Color, CursorPos, Dirty, Frame, Modes, RenderRow, Run, SelectionKind, Style, TermSize,
};
pub use pacer::{Decision, Pacer};
pub use session::{EventSink, Terminal, TerminalEvent, TerminalInfo, TerminalOptions, Terminals};
pub use shell::{Os, Shell, detect_shell, terminal_env};
