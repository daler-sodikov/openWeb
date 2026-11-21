/// Why a call, a connection or a discovery failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CdpError {
    /// The connection is closed (by either side) or never came up.
    #[error("the inspector connection is closed")]
    Closed,
    /// The caller's cancellation token fired.
    #[error("cancelled")]
    Cancelled,
    /// No answer arrived in time.
    #[error("timed out")]
    Timeout,
    /// The inspector answered with a protocol error.
    #[error("protocol error {code}: {message}")]
    Protocol {
        /// The JSON-RPC style error code.
        code: i64,
        /// The inspector's message.
        message: String,
    },
    /// The socket or an HTTP exchange failed.
    #[error("transport error: {0}")]
    Transport(String),
    /// A message could not be encoded or decoded.
    #[error("bad message: {0}")]
    Decode(String),
    /// The URL was refused because it is not a loopback `ws://127.0.0.1` URL.
    #[error("refused: {0}")]
    Refused(String),
}
