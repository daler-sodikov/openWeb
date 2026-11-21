//! Shared protocol types (`Runtime` and `Debugger` domains).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A script position, 0-based line and column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// The script the position is in.
    pub script_id: String,
    /// 0-based line.
    pub line_number: u32,
    /// 0-based column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_number: Option<u32>,
}

/// A value in the debuggee, by value or by reference (`object_id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteObject {
    /// `object`, `function`, `number`, `string`, `undefined`, ...
    #[serde(rename = "type")]
    pub kind: String,
    /// `array`, `error`, `null`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    /// The constructor name for objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    /// The primitive value, when it is JSON-representable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// `NaN`, `Infinity`, `-0`, bigints: values JSON cannot carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unserializable_value: Option<String>,
    /// A short rendering of the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The handle for `Runtime.getProperties`; present for objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    /// A preview when requested with `generatePreview`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ObjectPreview>,
}

/// A shallow preview of an object's properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectPreview {
    /// The object type.
    #[serde(rename = "type")]
    pub kind: String,
    /// The object subtype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    /// A short rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// True when properties were left out.
    #[serde(default)]
    pub overflow: bool,
    /// The previewed properties.
    #[serde(default)]
    pub properties: Vec<PropertyPreview>,
}

/// One previewed property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyPreview {
    /// The property name.
    pub name: String,
    /// The value's type.
    #[serde(rename = "type")]
    pub kind: String,
    /// The value's subtype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    /// The value rendered as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// One property of an object (`Runtime.getProperties`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDescriptor {
    /// The property name.
    pub name: String,
    /// The value; absent for accessors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<RemoteObject>,
    /// Whether the value can be changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    /// The getter, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub get: Option<RemoteObject>,
    /// The setter, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<RemoteObject>,
    /// Whether the property can be deleted or redefined.
    #[serde(default)]
    pub configurable: bool,
    /// Whether it shows up in enumeration.
    #[serde(default)]
    pub enumerable: bool,
    /// True when the getter threw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was_thrown: Option<bool>,
    /// True for own (not inherited) properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_own: Option<bool>,
    /// The symbol, for symbol-keyed properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<RemoteObject>,
}

/// An engine-internal property such as `[[Prototype]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalPropertyDescriptor {
    /// The internal slot name.
    pub name: String,
    /// Its value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<RemoteObject>,
}

/// A variable scope of a paused frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    /// `global`, `local`, `block`, `closure`, `catch`, `module`, ...
    #[serde(rename = "type")]
    pub kind: String,
    /// The object holding the variables; read it with `Runtime.getProperties`.
    pub object: RemoteObject,
    /// The function name for function scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where the scope starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_location: Option<Location>,
    /// Where the scope ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_location: Option<Location>,
}

/// A frame of a paused call stack (`Debugger.paused`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    /// The handle for `Debugger.evaluateOnCallFrame`.
    pub call_frame_id: String,
    /// The function name; empty for top-level code.
    pub function_name: String,
    /// Where the function is defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_location: Option<Location>,
    /// The current position in this frame.
    pub location: Location,
    /// Deprecated and empty in current V8; use the script's URL by id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Innermost scope first.
    #[serde(default)]
    pub scope_chain: Vec<Scope>,
    /// The `this` value.
    #[serde(rename = "this")]
    pub this_object: RemoteObject,
    /// The return value, when paused at a return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_value: Option<RemoteObject>,
    /// Whether the frame can be restarted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_be_restarted: Option<bool>,
}

/// A frame of a stack trace attached to a console call or an exception.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    /// The function name; empty for top-level code.
    pub function_name: String,
    /// The script id.
    pub script_id: String,
    /// The script URL (`file://...` or `node:...`).
    pub url: String,
    /// 0-based line.
    pub line_number: u32,
    /// 0-based column.
    pub column_number: u32,
}

/// A stack trace of a console call or an exception.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTrace {
    /// A label such as `await` for async parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The frames, innermost first.
    pub call_frames: Vec<StackFrame>,
    /// The async parent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<StackTrace>>,
}

/// What went wrong when an evaluation or a script threw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDetails {
    /// The id of this exception report.
    pub exception_id: i64,
    /// `Uncaught` or a message.
    pub text: String,
    /// 0-based line.
    pub line_number: u32,
    /// 0-based column.
    pub column_number: u32,
    /// The script id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_id: Option<String>,
    /// The script URL or path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Where it was thrown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<StackTrace>,
    /// The thrown value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception: Option<RemoteObject>,
    /// The execution context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_context_id: Option<i64>,
}
