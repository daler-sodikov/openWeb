//! Headless backend for Playwright UI tests (`docs/ARCHITECTURE.md`,
//! section 7). Builds the app on Tauri's mock runtime with the same invoke
//! handler as the real app and serves IPC over HTTP on 127.0.0.1:
//!
//! - `GET /health` → `ok`
//! - `POST /invoke/<command>` with the JSON arguments as body →
//!   `{"ok": <result>}` or `{"err": <error>}`
//! - `GET /events?after=<seq>` → `{"next": <seq>, "events": [{"event", "payload"}]}`,
//!   waiting up to 20 s for new events (long poll); `after=latest` returns
//!   the current position immediately
//!
//! Usage: `ferrule-headless --port 1431 --data-dir <dir>`. Test-only: built
//! only with the `headless` feature, never bundled.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use ferrule_app::{InitialProject, SmokeMode, configure, encode_event, init_tracing};
use ferrule_core::{EventSink, Session, SessionEvent};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};
use tiny_http::{Header, Method, Request, Response, Server};

/// Number of request threads; one may be parked in a long poll.
const WORKERS: usize = 4;
const LONG_POLL: Duration = Duration::from_secs(20);

/// Session events queued for `GET /events`.
#[derive(Default)]
struct EventQueue {
    events: Mutex<Vec<serde_json::Value>>,
    added: Condvar,
}

impl EventSink for EventQueue {
    fn emit(&self, event: SessionEvent) {
        let (name, payload) = encode_event(&event);
        if let Ok(mut events) = self.events.lock() {
            events.push(serde_json::json!({ "event": name, "payload": payload }));
            self.added.notify_all();
        }
    }
}

impl EventQueue {
    fn len(&self) -> usize {
        self.events.lock().map_or(0, |e| e.len())
    }

    /// Events after `after`, waiting up to [`LONG_POLL`] for at least one.
    fn after(&self, after: usize) -> (usize, Vec<serde_json::Value>) {
        let Ok(events) = self.events.lock() else {
            return (after, Vec::new());
        };
        let Ok((events, _)) = self
            .added
            .wait_timeout_while(events, LONG_POLL, |e| e.len() <= after)
        else {
            return (after, Vec::new());
        };
        let start = after.min(events.len());
        (events.len(), events[start..].to_vec())
    }
}

struct Args {
    port: u16,
    data_dir: PathBuf,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut port = 1431;
    let mut data_dir = std::env::temp_dir().join("ferrule-headless");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().context("--port needs a value")?.parse()?,
            "--data-dir" => data_dir = args.next().context("--data-dir needs a value")?.into(),
            other => bail!("unknown argument {other:?}"),
        }
    }
    Ok(Args { port, data_dir })
}

fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = parse_args()?;
    let events = Arc::new(EventQueue::default());
    let session = Session::new(args.data_dir, events.clone())?;
    let app = configure(mock_builder(), InitialProject(None), SmokeMode::new(false))
        .build(mock_context(noop_assets()))
        .context("building the mock app failed")?;
    app.manage(session);
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .context("creating the mock webview failed")?;

    let server = Server::http(("127.0.0.1", args.port))
        .map_err(|err| anyhow::anyhow!("cannot listen on port {}: {err}", args.port))?;
    tracing::info!(port = args.port, "headless backend listening");
    let server = Arc::new(server);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            let server = server.clone();
            let webview = &webview;
            let events = &events;
            scope.spawn(move || {
                while let Ok(request) = server.recv() {
                    if let Err(err) = handle(webview, events, request) {
                        tracing::error!(%err, "request failed");
                    }
                }
            });
        }
    });
    Ok(())
}

fn handle(
    webview: &WebviewWindow<MockRuntime>,
    events: &EventQueue,
    mut request: Request,
) -> anyhow::Result<()> {
    let cors = Header::from_bytes("Access-Control-Allow-Origin", "*")
        .map_err(|()| anyhow::anyhow!("invalid header"))?;
    let json = Header::from_bytes("Content-Type", "application/json")
        .map_err(|()| anyhow::anyhow!("invalid header"))?;
    let url = request.url().to_owned();

    if request.method() == &Method::Get && url == "/health" {
        request.respond(Response::from_string("ok").with_header(cors))?;
        return Ok(());
    }
    if request.method() == &Method::Get
        && let Some(query) = url.strip_prefix("/events?after=")
    {
        // `latest` returns the current position without waiting, so a newly
        // loaded page does not replay events from earlier sessions.
        let (next, batch) = if query == "latest" {
            (events.len(), Vec::new())
        } else {
            events.after(query.parse().unwrap_or(0))
        };
        let body = serde_json::json!({ "next": next, "events": batch });
        request.respond(
            Response::from_string(body.to_string())
                .with_header(cors)
                .with_header(json),
        )?;
        return Ok(());
    }
    let Some(cmd) = url
        .strip_prefix("/invoke/")
        .filter(|_| request.method() == &Method::Post)
    else {
        request.respond(
            Response::from_string("not found")
                .with_status_code(404)
                .with_header(cors),
        )?;
        return Ok(());
    };
    let cmd = cmd.to_owned();

    let mut body = String::new();
    request.as_reader().read_to_string(&mut body)?;
    let args: serde_json::Value = if body.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&body)?
    };

    let invoke = InvokeRequest {
        cmd,
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
        .parse()?,
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_owned(),
    };
    let reply = match get_ipc_response(webview, invoke) {
        Ok(body) => serde_json::json!({ "ok": body.deserialize::<serde_json::Value>()? }),
        Err(err) => serde_json::json!({ "err": err }),
    };
    request.respond(
        Response::from_string(reply.to_string())
            .with_header(cors)
            .with_header(json),
    )?;
    Ok(())
}
