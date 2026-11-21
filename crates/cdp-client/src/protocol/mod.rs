//! The typed subset of the Chrome DevTools Protocol that the debugger uses.
//!
//! Requests implement [`Request`] and go through [`CdpClient::send`];
//! events arrive as [`Event`] via [`CdpEvent::typed`]. The types are written
//! by hand for this subset (see ARCHITECTURE.md 4.10.1) and tested against
//! JSON recorded from real Node. Unknown fields are ignored and unknown
//! events yield `None`, so a newer Node does not break the client. These
//! types never cross the IPC boundary.

mod debugger;
mod events;
mod runtime;
mod types;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{CdpClient, CdpError};

pub use debugger::{
    BlackboxPatterns, DebuggerDisable, DebuggerEnable, DebuggerEnableResult, EvaluateOnCallFrame,
    EvaluateResult, GetScriptSource, GetScriptSourceResult, Pause, PauseOnExceptions,
    RemoveBreakpoint, Resume, SetBreakpointByUrl, SetBreakpointByUrlResult, SetBreakpointsActive,
    SetPauseOnExceptions, StepInto, StepOut, StepOver,
};
pub use events::{
    BreakpointResolved, ConsoleApiCalled, Event, ExceptionThrown, Paused, ScriptParsed,
};
pub use runtime::{
    GetProperties, GetPropertiesResult, ReleaseObjectGroup, RunIfWaitingForDebugger, RuntimeEnable,
    RuntimeEvaluate,
};
pub use types::{
    CallFrame, ExceptionDetails, InternalPropertyDescriptor, Location, ObjectPreview,
    PropertyDescriptor, PropertyPreview, RemoteObject, Scope, StackFrame, StackTrace,
};

/// A protocol request: the method name, the parameters (the type itself) and
/// the shape of the result.
pub trait Request: Serialize {
    /// The protocol method, e.g. `Debugger.enable`.
    const METHOD: &'static str;
    /// The `result` object of the answer.
    type Response: DeserializeOwned;
}

/// The result of a request whose answer carries nothing the client uses.
/// Accepts any `result` object, so extra fields in newer Node are fine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Empty {}

impl CdpClient {
    /// Sends a typed request and decodes its typed result.
    pub async fn send<R: Request>(&self, request: R) -> Result<R::Response, CdpError> {
        self.call(R::METHOD, request).await
    }
}
