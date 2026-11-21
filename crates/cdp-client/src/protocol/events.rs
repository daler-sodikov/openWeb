//! Typed events.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{CallFrame, ExceptionDetails, Location, RemoteObject, StackTrace};
use crate::{CdpError, CdpEvent};

/// `Debugger.paused`: execution stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paused {
    /// The stack, innermost frame first.
    pub call_frames: Vec<CallFrame>,
    /// Why: `Break on start`, `exception`, `step`, `other` (breakpoints and
    /// `Debugger.pause`), ... Kept as text so new reasons do not break us.
    pub reason: String,
    /// Reason-specific data, e.g. the thrown value for `exception`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// The ids of the breakpoints that were hit.
    #[serde(default)]
    pub hit_breakpoints: Vec<String>,
    /// The async part of the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_stack_trace: Option<StackTrace>,
}

/// `Debugger.scriptParsed`: a script became known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptParsed {
    /// The id used in locations.
    pub script_id: String,
    /// `file://...` for user code, `node:...` for Node's own.
    pub url: String,
    /// First line of the script within its resource.
    pub start_line: u32,
    /// First column.
    pub start_column: u32,
    /// Last line.
    pub end_line: u32,
    /// Last column.
    pub end_column: u32,
    /// The execution context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_context_id: Option<i64>,
    /// A content hash.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
    /// The `//# sourceMappingURL` value; empty when there is none.
    #[serde(
        default,
        rename = "sourceMapURL",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_map_url: Option<String>,
    /// Whether the script has a `//# sourceURL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_source_url: Option<bool>,
    /// Whether it is an ES module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_module: Option<bool>,
    /// Length in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

/// `Debugger.breakpointResolved`: a breakpoint found its script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakpointResolved {
    /// The breakpoint id.
    pub breakpoint_id: String,
    /// Where it resolved, possibly moved to the next valid position.
    pub location: Location,
}

/// `Runtime.consoleAPICalled`: `console.log` and friends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleApiCalled {
    /// `log`, `warn`, `error`, `info`, `debug`, ...
    #[serde(rename = "type")]
    pub kind: String,
    /// The arguments.
    #[serde(default)]
    pub args: Vec<RemoteObject>,
    /// The execution context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_context_id: Option<i64>,
    /// Milliseconds since the epoch.
    pub timestamp: f64,
    /// Where the call happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<StackTrace>,
}

/// `Runtime.exceptionThrown`: an uncaught exception.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionThrown {
    /// Milliseconds since the epoch.
    pub timestamp: f64,
    /// The details.
    pub exception_details: ExceptionDetails,
}

/// A decoded event of the supported subset.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// `Debugger.paused`.
    Paused(Paused),
    /// `Debugger.resumed`.
    Resumed,
    /// `Debugger.scriptParsed`.
    ScriptParsed(ScriptParsed),
    /// `Debugger.breakpointResolved`.
    BreakpointResolved(BreakpointResolved),
    /// `Runtime.consoleAPICalled`.
    ConsoleApiCalled(ConsoleApiCalled),
    /// `Runtime.exceptionThrown`.
    ExceptionThrown(Box<ExceptionThrown>),
}

fn decode<T: DeserializeOwned>(params: &Value) -> Result<T, CdpError> {
    T::deserialize(params).map_err(|e| CdpError::Decode(e.to_string()))
}

impl Event {
    /// Decodes an event by method name. Events outside the subset give
    /// `Ok(None)`; a supported event with malformed params is an error.
    pub fn from_raw(method: &str, params: &Value) -> Result<Option<Event>, CdpError> {
        Ok(Some(match method {
            "Debugger.paused" => Event::Paused(decode(params)?),
            "Debugger.resumed" => Event::Resumed,
            "Debugger.scriptParsed" => Event::ScriptParsed(decode(params)?),
            "Debugger.breakpointResolved" => Event::BreakpointResolved(decode(params)?),
            "Runtime.consoleAPICalled" => Event::ConsoleApiCalled(decode(params)?),
            "Runtime.exceptionThrown" => Event::ExceptionThrown(Box::new(decode(params)?)),
            _ => return Ok(None),
        }))
    }
}

impl CdpEvent {
    /// The typed form of a protocol event: `Ok(None)` for events outside the
    /// subset and for [`CdpEvent::Closed`].
    pub fn typed(&self) -> Result<Option<Event>, CdpError> {
        match self {
            CdpEvent::Event { method, params } => Event::from_raw(method, params),
            _ => Ok(None),
        }
    }
}
