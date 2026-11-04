//! Events the session pushes to the frontend.

use ferrule_ipc::{FsChangeBatch, IndexState};

/// An event for the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Files or directories changed on disk.
    FsChanged(FsChangeBatch),
    /// Index progress.
    IndexStatus(IndexState),
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
