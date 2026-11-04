//! Serializable data types exchanged between the Rust backend and the
//! TypeScript frontend.
//!
//! Every type here derives [`specta::Type`] so that `ferrule-app` can export
//! TypeScript bindings for it. This crate holds no logic; see
//! `docs/ARCHITECTURE.md`, section 3, for the conventions.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Version of the IPC protocol. Bump on any breaking change to a command,
/// event or DTO so that a mismatched frontend refuses to run.
pub const PROTOCOL_VERSION: u32 = 1;

/// Operating system the backend runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    /// Linux and other freedesktop systems.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
    /// Any other target.
    Other,
}

/// Static information about the running application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// Product name shown in the UI.
    pub name: String,
    /// Application version (semver).
    pub version: String,
    /// IPC protocol version the backend speaks; see [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Operating system the backend runs on.
    pub platform: Platform,
}

/// What the frontend rendered after mounting. Sent once at startup; the
/// backend uses it to verify that the window, the bundled frontend and the
/// IPC round trip all work (the e2e smoke test depends on this).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrontendReadyReport {
    /// Product name as rendered in the DOM.
    pub rendered_app_name: String,
    /// `document.title` after mounting.
    pub document_title: String,
    /// Protocol version the frontend bindings were generated for.
    pub protocol_version: u32,
}

/// Category of an [`IpcError`]. The frontend switches on this, never on the
/// message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum IpcErrorKind {
    /// The referenced entity does not exist.
    NotFound,
    /// An argument was malformed or out of range.
    InvalidArgument,
    /// The request was superseded or cancelled.
    Cancelled,
    /// A file system or process error occurred.
    Io,
    /// The request conflicts with current state (e.g. stale version).
    Conflict,
    /// A bug or unexpected condition in the backend.
    Internal,
}

/// Error returned by every fallible IPC command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{kind:?}: {message}")]
pub struct IpcError {
    /// Machine-readable category.
    pub kind: IpcErrorKind,
    /// Human-readable description, for logs and display only.
    pub message: String,
}

impl IpcError {
    /// Creates an error of the given kind.
    pub fn new(kind: IpcErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_serializes_camel_case() {
        let info = AppInfo {
            name: "Ferrule".into(),
            version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            platform: Platform::Macos,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "Ferrule",
                "version": "0.1.0",
                "protocolVersion": PROTOCOL_VERSION,
                "platform": "macos",
            })
        );
    }

    #[test]
    fn ipc_error_round_trips() {
        let err = IpcError::new(IpcErrorKind::NotFound, "no such file");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, r#"{"kind":"notFound","message":"no such file"}"#);
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, err);
        assert_eq!(err.to_string(), "NotFound: no such file");
    }
}
