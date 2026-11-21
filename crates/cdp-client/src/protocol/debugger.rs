//! `Debugger` domain requests.

use serde::{Deserialize, Serialize};

use super::types::{ExceptionDetails, Location, RemoteObject};
use super::{Empty, Request};

macro_rules! request {
    ($ty:ty, $method:literal, $response:ty) => {
        impl Request for $ty {
            const METHOD: &'static str = $method;
            type Response = $response;
        }
    };
}

/// `Debugger.enable`: starts debugger events (and replays `scriptParsed`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebuggerEnable {
    /// Cache size for script sources, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scripts_cache_size: Option<u64>,
}
/// The result of [`DebuggerEnable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebuggerEnableResult {
    /// The unique id of this debugger.
    pub debugger_id: String,
}
request!(DebuggerEnable, "Debugger.enable", DebuggerEnableResult);

/// `Debugger.disable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebuggerDisable;
request!(DebuggerDisable, "Debugger.disable", Empty);

/// `Debugger.setBreakpointByUrl`: set by URL (or regex), so it also applies
/// to scripts loaded later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointByUrl {
    /// 0-based line.
    pub line_number: u32,
    /// The script URL (`file://...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A regex for the URL, instead of `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_regex: Option<String>,
    /// A script hash, instead of `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_hash: Option<String>,
    /// 0-based column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_number: Option<u32>,
    /// A JavaScript condition; the breakpoint pauses when it is truthy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}
/// The result of [`SetBreakpointByUrl`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointByUrlResult {
    /// The id for `Debugger.removeBreakpoint` and `hitBreakpoints`.
    pub breakpoint_id: String,
    /// Where it resolved so far; empty until a matching script is parsed.
    #[serde(default)]
    pub locations: Vec<Location>,
}
request!(
    SetBreakpointByUrl,
    "Debugger.setBreakpointByUrl",
    SetBreakpointByUrlResult
);

/// `Debugger.removeBreakpoint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveBreakpoint {
    /// The id from [`SetBreakpointByUrlResult`].
    pub breakpoint_id: String,
}
request!(RemoveBreakpoint, "Debugger.removeBreakpoint", Empty);

/// `Debugger.setBreakpointsActive`: turns all breakpoints on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetBreakpointsActive {
    /// Whether breakpoints are active.
    pub active: bool,
}
request!(SetBreakpointsActive, "Debugger.setBreakpointsActive", Empty);

/// When to pause on exceptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PauseOnExceptions {
    /// Never.
    None,
    /// Only exceptions nothing catches.
    Uncaught,
    /// Every throw.
    All,
}

/// `Debugger.setPauseOnExceptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPauseOnExceptions {
    /// The mode.
    pub state: PauseOnExceptions,
}
request!(SetPauseOnExceptions, "Debugger.setPauseOnExceptions", Empty);

/// `Debugger.setBlackboxPatterns`: scripts whose URL matches are skipped
/// when stepping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlackboxPatterns {
    /// Regular expressions matched against script URLs.
    pub patterns: Vec<String>,
}
request!(BlackboxPatterns, "Debugger.setBlackboxPatterns", Empty);

/// `Debugger.resume`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resume {
    /// Terminate the execution after resuming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminate_on_resume: Option<bool>,
}
request!(Resume, "Debugger.resume", Empty);

/// `Debugger.stepOver`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOver;
request!(StepOver, "Debugger.stepOver", Empty);

/// `Debugger.stepInto`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepInto;
request!(StepInto, "Debugger.stepInto", Empty);

/// `Debugger.stepOut`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOut;
request!(StepOut, "Debugger.stepOut", Empty);

/// `Debugger.pause`: pauses at the next statement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause;
request!(Pause, "Debugger.pause", Empty);

/// `Debugger.evaluateOnCallFrame`: evaluates in the scope of a paused frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateOnCallFrame {
    /// The frame from `Debugger.paused`.
    pub call_frame_id: String,
    /// The JavaScript to run.
    pub expression: String,
    /// A group name for `Runtime.releaseObjectGroup`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_group: Option<String>,
    /// Return the value itself instead of a handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_by_value: Option<bool>,
    /// Include an object preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generate_preview: Option<bool>,
    /// Fail on side effects (safe hover evaluation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throw_on_side_effect: Option<bool>,
    /// Milliseconds before the evaluation is aborted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}
/// The result of [`EvaluateOnCallFrame`] and [`RuntimeEvaluate`](super::RuntimeEvaluate).
///
/// A thrown error is not a protocol error: `exception_details` is set and
/// `result` holds the thrown value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResult {
    /// The value, or the thrown value.
    pub result: RemoteObject,
    /// Set when the evaluation threw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception_details: Option<ExceptionDetails>,
}
request!(
    EvaluateOnCallFrame,
    "Debugger.evaluateOnCallFrame",
    EvaluateResult
);

/// `Debugger.getScriptSource`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetScriptSource {
    /// The script id from `scriptParsed`.
    pub script_id: String,
}
/// The result of [`GetScriptSource`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetScriptSourceResult {
    /// The source text.
    pub script_source: String,
}
request!(
    GetScriptSource,
    "Debugger.getScriptSource",
    GetScriptSourceResult
);
