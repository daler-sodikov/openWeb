//! Typed backend → frontend events (tauri-specta) and the Tauri event sink.

use ferrule_core::{EventSink, SessionEvent};
use ferrule_ipc::{
    FileProblems, FsChangeBatch, IndexState, ServerStatus, TerminalExited, TerminalFrame,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

/// Files or directories changed on disk (`fs-changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct FsChanged(pub FsChangeBatch);

/// Index progress (`index-status`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct IndexStatus(pub IndexState);

/// A language server's state changed (`server-status-changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ServerStatusChanged(pub ServerStatus);

/// The problems of one file changed (`problems-changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ProblemsChanged(pub FileProblems);

/// A terminal screen update (`terminal-frame-event`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TerminalFrameEvent(pub TerminalFrame);

/// A terminal's shell exited (`terminal-exited-event`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TerminalExitedEvent(pub TerminalExited);

/// Event name and JSON payload of a session event, for transports other
/// than Tauri events (the headless test backend).
pub fn encode(event: &SessionEvent) -> (&'static str, serde_json::Value) {
    let (name, payload) = match event {
        SessionEvent::FsChanged(batch) => (FsChanged::NAME, serde_json::to_value(batch)),
        SessionEvent::IndexStatus(state) => (IndexStatus::NAME, serde_json::to_value(state)),
        SessionEvent::ServerStatus(status) => {
            (ServerStatusChanged::NAME, serde_json::to_value(status))
        }
        SessionEvent::ProblemsChanged(problems) => {
            (ProblemsChanged::NAME, serde_json::to_value(problems))
        }
        SessionEvent::TerminalFrame(frame) => {
            (TerminalFrameEvent::NAME, serde_json::to_value(frame))
        }
        SessionEvent::TerminalExited(exited) => {
            (TerminalExitedEvent::NAME, serde_json::to_value(exited))
        }
    };
    (name, payload.unwrap_or(serde_json::Value::Null))
}

/// Emits session events as Tauri events.
pub struct TauriSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> EventSink for TauriSink<R> {
    fn emit(&self, event: SessionEvent) {
        let result = match event {
            SessionEvent::FsChanged(batch) => FsChanged(batch).emit(&self.0),
            SessionEvent::IndexStatus(state) => IndexStatus(state).emit(&self.0),
            SessionEvent::ServerStatus(status) => ServerStatusChanged(status).emit(&self.0),
            SessionEvent::ProblemsChanged(problems) => ProblemsChanged(problems).emit(&self.0),
            SessionEvent::TerminalFrame(frame) => TerminalFrameEvent(frame).emit(&self.0),
            SessionEvent::TerminalExited(exited) => TerminalExitedEvent(exited).emit(&self.0),
        };
        if let Err(err) = result {
            tracing::warn!(%err, "emitting an event failed");
        }
    }
}
