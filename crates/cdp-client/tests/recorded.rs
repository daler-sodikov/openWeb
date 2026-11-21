//! The typed subset against JSON recorded from real Node (see tests/data).

use std::collections::HashMap;

use ferrule_cdp_client::protocol::{self as p, Event, Request};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Recording {
    /// `(method, params)` of every request, with its recorded `result`.
    calls: Vec<(String, Value, Value)>,
    /// `(method, params)` of every event.
    events: Vec<(String, Value)>,
}

fn load(name: &str) -> Result<Recording, Box<dyn std::error::Error>> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut requests: HashMap<u64, (String, Value)> = HashMap::new();
    let mut recording = Recording {
        calls: Vec::new(),
        events: Vec::new(),
    };
    for entry in doc["messages"].as_array().ok_or("no messages")? {
        let msg = &entry["msg"];
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        if entry["dir"] == "out" {
            let id = msg["id"].as_u64().ok_or("request without id")?;
            let method = msg["method"].as_str().ok_or("request without method")?;
            requests.insert(id, (method.to_string(), params));
        } else if let Some(id) = msg["id"].as_u64() {
            let (method, params) = requests.remove(&id).ok_or("answer without request")?;
            let result = msg.get("result").cloned().ok_or("answer without result")?;
            recording.calls.push((method, params, result));
        } else {
            let method = msg["method"].as_str().ok_or("event without method")?;
            recording.events.push((method.to_string(), params));
        }
    }
    Ok(recording)
}

/// Request params must survive a decode and re-encode unchanged (we write
/// them, so nothing may be lost); the result must decode and be stable.
fn round_trip<R>(
    rec: &Recording,
    count: usize,
) -> Result<Vec<R::Response>, Box<dyn std::error::Error>>
where
    R: Request + DeserializeOwned,
    R::Response: Serialize,
{
    let mut out = Vec::new();
    for (method, params, result) in rec.calls.iter().filter(|c| c.0 == R::METHOD) {
        // No recorded params: unit requests decode from null, option-only
        // structs from an empty object.
        let sent = params.clone();
        let request: R = match serde_json::from_value(sent.clone()) {
            Ok(request) => request,
            Err(_) if params.is_null() => serde_json::from_value(Value::Object(Default::default()))
                .map_err(|e| format!("{method} params: {e}"))?,
            Err(e) => return Err(format!("{method} params: {e}").into()),
        };
        let encoded = serde_json::to_value(&request)?;
        if params.is_null() {
            assert!(
                encoded.is_null() || encoded == Value::Object(Default::default()),
                "{method} sends no params, got {encoded}"
            );
        } else {
            assert_eq!(&encoded, params, "{method} request re-encodes unchanged");
        }

        let response: R::Response =
            serde_json::from_value(result.clone()).map_err(|e| format!("{method} result: {e}"))?;
        let again: R::Response = serde_json::from_value(serde_json::to_value(&response)?)?;
        assert_eq!(
            serde_json::to_value(&response)?,
            serde_json::to_value(&again)?
        );
        out.push(response);
    }
    assert_eq!(out.len(), count, "{} recorded calls", R::METHOD);
    Ok(out)
}

#[test]
fn debugger_requests_round_trip() -> TestResult {
    let rec = load("node-session.json")?;
    let enable = round_trip::<p::DebuggerEnable>(&rec, 1)?;
    assert!(!enable[0].debugger_id.is_empty());
    round_trip::<p::BlackboxPatterns>(&rec, 1)?;
    round_trip::<p::SetPauseOnExceptions>(&rec, 1)?;
    round_trip::<p::SetBreakpointsActive>(&rec, 1)?;
    let set = round_trip::<p::SetBreakpointByUrl>(&rec, 2)?;
    assert_eq!(set[0].breakpoint_id, "1:1:0:file:///project/target.js");
    assert!(set[0].locations.is_empty());
    round_trip::<p::Resume>(&rec, 3)?;
    round_trip::<p::StepOver>(&rec, 1)?;
    round_trip::<p::StepInto>(&rec, 1)?;
    round_trip::<p::StepOut>(&rec, 1)?;
    let source = round_trip::<p::GetScriptSource>(&rec, 1)?;
    assert!(source[0].script_source.starts_with("function add(a, b)"));

    let eval = round_trip::<p::EvaluateOnCallFrame>(&rec, 2)?;
    assert_eq!(eval[0].result.kind, "number");
    assert_eq!(eval[0].result.value, Some(Value::from(3)));
    assert!(eval[0].exception_details.is_none());
    // A thrown ReferenceError is an ordinary result with exception details.
    assert_eq!(eval[1].result.subtype.as_deref(), Some("error"));
    assert!(eval[1].exception_details.is_some());

    let pause = load("node-session-pause.json")?;
    round_trip::<p::Pause>(&pause, 1)?;
    round_trip::<p::RemoveBreakpoint>(&pause, 1)?;
    round_trip::<p::DebuggerDisable>(&pause, 1)?;
    Ok(())
}

#[test]
fn runtime_requests_round_trip() -> TestResult {
    let rec = load("node-session.json")?;
    round_trip::<p::RuntimeEnable>(&rec, 1)?;
    round_trip::<p::RunIfWaitingForDebugger>(&rec, 1)?;
    round_trip::<p::ReleaseObjectGroup>(&rec, 1)?;
    let eval = round_trip::<p::RuntimeEvaluate>(&rec, 1)?;
    assert_eq!(eval[0].result.class_name.as_deref(), Some("Object"));
    assert!(eval[0].result.object_id.is_some());

    let props = round_trip::<p::GetProperties>(&rec, 2)?;
    let names: Vec<_> = props[0].result.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(&names[..2], ["a", "b"]);
    let a = &props[0].result[0];
    assert_eq!(
        a.value.as_ref().and_then(|v| v.value.clone()),
        Some(Value::from(1))
    );
    assert_eq!(a.is_own, Some(true));
    // The second object has an array property: a handle to expand later.
    let b = &props[1].result[1];
    assert_eq!(
        b.value.as_ref().and_then(|v| v.subtype.as_deref()),
        Some("array")
    );
    assert!(
        b.value
            .as_ref()
            .and_then(|v| v.object_id.as_ref())
            .is_some()
    );
    Ok(())
}

fn typed_events(rec: &Recording, method: &str) -> Result<Vec<Event>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for (name, params) in rec.events.iter().filter(|e| e.0 == method) {
        let event = Event::from_raw(name, params)?.ok_or("event not decoded")?;
        out.push(event);
    }
    assert!(!out.is_empty(), "no recorded {method}");
    Ok(out)
}

#[test]
fn recorded_events_decode() -> TestResult {
    let rec = load("node-session.json")?;

    let paused = typed_events(&rec, "Debugger.paused")?;
    let reasons: Vec<_> = paused
        .iter()
        .map(|e| match e {
            Event::Paused(p) => p.reason.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(reasons, ["Break on start", "other", "step", "exception"]);
    let Event::Paused(hit) = &paused[1] else {
        return Err("not paused".into());
    };
    assert_eq!(hit.hit_breakpoints, ["1:1:0:file:///project/target.js"]);
    assert_eq!(hit.call_frames[0].function_name, "add");
    assert_eq!(hit.call_frames[0].location.line_number, 1);
    assert_eq!(hit.call_frames[0].scope_chain[0].kind, "local");
    assert!(hit.call_frames[0].scope_chain[0].object.object_id.is_some());
    let Event::Paused(thrown) = &paused[3] else {
        return Err("not paused".into());
    };
    assert!(thrown.data.is_some());

    let parsed = typed_events(&rec, "Debugger.scriptParsed")?;
    let urls: Vec<_> = parsed
        .iter()
        .filter_map(|e| match e {
            Event::ScriptParsed(s) => Some(s.url.as_str()),
            _ => None,
        })
        .collect();
    assert!(urls.contains(&"file:///project/target.js"));
    assert!(urls.iter().any(|u| u.starts_with("node:")));

    let resumed = typed_events(&rec, "Debugger.resumed")?;
    assert_eq!(resumed[0], Event::Resumed);

    let resolved = typed_events(&rec, "Debugger.breakpointResolved")?;
    let Event::BreakpointResolved(r) = &resolved[0] else {
        return Err("wrong event".into());
    };
    assert_eq!(r.breakpoint_id, "1:1:0:file:///project/target.js");
    assert_eq!(r.location.line_number, 1);

    let console = typed_events(&rec, "Runtime.consoleAPICalled")?;
    let Event::ConsoleApiCalled(c) = &console[0] else {
        return Err("wrong event".into());
    };
    assert_eq!(c.kind, "log");
    assert_eq!(c.args[0].value, Some(Value::from("hello")));
    assert_eq!(
        c.args[1].preview.as_ref().map(|p| p.properties.len()),
        Some(2)
    );

    let thrown = typed_events(&rec, "Runtime.exceptionThrown")?;
    let Event::ExceptionThrown(t) = &thrown[0] else {
        return Err("wrong event".into());
    };
    assert_eq!(t.exception_details.text, "Uncaught");
    assert!(
        t.exception_details
            .exception
            .as_ref()
            .and_then(|e| e.description.as_deref())
            .is_some_and(|d| d.starts_with("Error: boom"))
    );
    Ok(())
}

#[test]
fn every_event_round_trips_through_its_type() -> TestResult {
    for name in ["node-session.json", "node-session-pause.json"] {
        let rec = load(name)?;
        for (method, params) in &rec.events {
            let Some(event) = Event::from_raw(method, params)? else {
                continue;
            };
            // Decoding what we would encode gives the same typed value.
            let encoded = match &event {
                Event::Paused(v) => serde_json::to_value(v)?,
                Event::Resumed => Value::Null,
                Event::ScriptParsed(v) => serde_json::to_value(v)?,
                Event::BreakpointResolved(v) => serde_json::to_value(v)?,
                Event::ConsoleApiCalled(v) => serde_json::to_value(v)?,
                Event::ExceptionThrown(v) => serde_json::to_value(v)?,
                _ => continue,
            };
            assert_eq!(Event::from_raw(method, &encoded)?, Some(event), "{method}");
        }
    }
    Ok(())
}

#[test]
fn unknown_events_and_fields_are_ignored() -> TestResult {
    assert_eq!(
        Event::from_raw("Profiler.somethingNew", &Value::Null)?,
        None
    );
    let extended = serde_json::json!({
        "breakpointId": "1:1:0:x", "futureField": [1, 2],
        "location": { "scriptId": "1", "lineNumber": 3, "columnNumber": 4, "extra": true }
    });
    let event = Event::from_raw("Debugger.breakpointResolved", &extended)?;
    assert!(matches!(event, Some(Event::BreakpointResolved(_))));
    assert!(Event::from_raw("Debugger.paused", &serde_json::json!({ "reason": 5 })).is_err());
    Ok(())
}
