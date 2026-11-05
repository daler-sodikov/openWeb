//! The real backend: PTYs with portable-pty (ConPTY on Windows) and the VT
//! emulator of alacritty_terminal. Every call into those crates is in this
//! module; it is built only with the `native` feature.

mod process;
mod screen;

use crate::backend::{Backend, Screen, SpawnSpec, Spawned, TerminalError};
use crate::model::TermSize;

pub use screen::AlacrittyScreen;

/// Starts shells on real PTYs and emulates them with alacritty_terminal.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeBackend;

impl Backend for NativeBackend {
    fn spawn(&self, spec: &SpawnSpec, size: TermSize) -> Result<Spawned, TerminalError> {
        process::spawn(spec, size)
    }

    fn screen(&self, size: TermSize, scrollback: usize) -> Box<dyn Screen> {
        Box::new(AlacrittyScreen::new(size, scrollback))
    }
}
