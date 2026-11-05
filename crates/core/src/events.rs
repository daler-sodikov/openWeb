//! Events the session pushes to the frontend.

use ferrule_ipc::{
    FileProblems, FsChangeBatch, IndexState, ServerStatus, TerminalExited, TerminalFrame,
};

/// An event for the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Files or directories changed on disk.
    FsChanged(FsChangeBatch),
    /// Index progress.
    IndexStatus(IndexState),
    /// A language server's state changed.
    ServerStatus(ServerStatus),
    /// The problems of one file changed.
    ProblemsChanged(FileProblems),
    /// A terminal screen update.
    TerminalFrame(TerminalFrame),
    /// A terminal's shell exited.
    TerminalExited(TerminalExited),
}

/// Where session events go (Tauri events in the app, a queue in tests).
pub trait EventSink: Send + Sync + 'static {
    /// Delivers an event. Must not block for long.
    fn emit(&self, event: SessionEvent);
}

/// Discards events.
#[derive(Debug, Default)]
pub struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _event: SessionEvent) {}
}
