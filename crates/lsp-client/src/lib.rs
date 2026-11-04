//! Generic Language Server Protocol client: process lifecycle, JSON-RPC,
//! cancellation, debounced notifications (`docs/ARCHITECTURE.md`, 4.7).
//!
//! - [`framing`]: the base protocol (`Content-Length` headers).
//! - [`Connection`]: JSON-RPC requests with ids, `$/cancelRequest` driven by
//!   a `CancellationToken`, notifications, server→client requests.
//! - [`Supervisor`]: spawns a server, initializes it (UTF-16 positions),
//!   restarts it with backoff when it dies, stops it, and sends document
//!   sync notifications (debounced `didChange`) to the right generation.
//! - [`find_in_dir`], [`find_on_path`], [`is_node_script`]: locating server
//!   executables.
//!
//! Nothing here knows a particular server; definitions live in `lang-*`.

mod connection;
mod discover;
pub mod framing;
mod supervisor;
mod uri;

pub use connection::{Connection, Incoming, PendingRequest, ResponseError, codes};
pub use discover::{find_in_dir, find_in_path_list, find_on_path, is_executable, is_node_script};
pub use lsp_types;
pub use supervisor::{
    ClientHooks, RestartPolicy, ServerCommand, ServerConfig, ServerState, Supervisor,
};
pub use uri::{path_to_uri, uri_to_path};

/// Errors from talking to a language server.
#[derive(Debug, thiserror::Error)]
pub enum LspError {
    /// The request was cancelled (by us, or the server reported it as
    /// cancelled or outdated).
    #[error("the request was cancelled")]
    Cancelled,
    /// No server is running right now.
    #[error("the language server is not running")]
    NotRunning,
    /// The connection closed (the server exited or restarted).
    #[error("the connection to the language server closed")]
    Closed,
    /// The server answered with an error.
    #[error("language server error {code}: {message}")]
    Server {
        /// JSON-RPC error code.
        code: i64,
        /// Message.
        message: String,
    },
    /// The server sent something we could not understand.
    #[error("language server protocol error: {0}")]
    Protocol(String),
    /// The server process could not be started.
    #[error("starting the language server failed: {0}")]
    Spawn(String),
    /// The server did not answer in time.
    #[error("the language server did not answer in time")]
    Timeout,
}
