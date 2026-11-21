//! `Runtime` domain requests.

use serde::{Deserialize, Serialize};

use super::debugger::EvaluateResult;
use super::types::{ExceptionDetails, InternalPropertyDescriptor, PropertyDescriptor};
use super::{Empty, Request};

/// `Runtime.enable`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEnable;
impl Request for RuntimeEnable {
    const METHOD: &'static str = "Runtime.enable";
    type Response = Empty;
}

/// `Runtime.runIfWaitingForDebugger`: lets a `--inspect-brk` Node run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIfWaitingForDebugger;
impl Request for RunIfWaitingForDebugger {
    const METHOD: &'static str = "Runtime.runIfWaitingForDebugger";
    type Response = Empty;
}

/// `Runtime.getProperties`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProperties {
    /// The object handle.
    pub object_id: String,
    /// Only own properties, not inherited ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_properties: Option<bool>,
    /// Only accessor properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessor_properties_only: Option<bool>,
    /// Include previews of the values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generate_preview: Option<bool>,
}
/// The result of [`GetProperties`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPropertiesResult {
    /// The properties.
    pub result: Vec<PropertyDescriptor>,
    /// Internal slots such as `[[Prototype]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_properties: Option<Vec<InternalPropertyDescriptor>>,
    /// Set when reading threw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception_details: Option<ExceptionDetails>,
}
impl Request for GetProperties {
    const METHOD: &'static str = "Runtime.getProperties";
    type Response = GetPropertiesResult;
}

/// `Runtime.evaluate`: evaluates in the global scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvaluate {
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
    /// Await a returned promise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub await_promise: Option<bool>,
}
impl Request for RuntimeEvaluate {
    const METHOD: &'static str = "Runtime.evaluate";
    type Response = EvaluateResult;
}

/// `Runtime.releaseObjectGroup`: frees the handles of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseObjectGroup {
    /// The group name used when evaluating.
    pub object_group: String,
}
impl Request for ReleaseObjectGroup {
    const METHOD: &'static str = "Runtime.releaseObjectGroup";
    type Response = Empty;
}
