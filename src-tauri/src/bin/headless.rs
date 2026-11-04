//! Headless backend for Playwright UI tests (`docs/ARCHITECTURE.md`,
//! section 7). Builds the app on Tauri's mock runtime with the same invoke
//! handler as the real app and serves IPC over HTTP on 127.0.0.1:
//!
//! - `GET /health` → `ok`
//! - `POST /invoke/<command>` with the JSON arguments as body →
//!   `{"ok": <result>}` or `{"err": <error>}`
//!
//! Usage: `ferrule-headless --port 1431 --data-dir <dir>`. Test-only: built
//! only with the `headless` feature, never bundled.

use std::path::PathBuf;

use anyhow::{Context, bail};
use ferrule_app::{InitialProject, SmokeMode, configure, init_tracing};
use ferrule_core::Session;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};
use tiny_http::{Header, Method, Request, Response, Server};

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
    let session = Session::new(args.data_dir.join("recovery"))?;
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
    for request in server.incoming_requests() {
        if let Err(err) = handle(&webview, request) {
            tracing::error!(%err, "request failed");
        }
    }
    Ok(())
}

fn handle(webview: &WebviewWindow<MockRuntime>, mut request: Request) -> anyhow::Result<()> {
    let cors = Header::from_bytes("Access-Control-Allow-Origin", "*")
        .map_err(|()| anyhow::anyhow!("invalid header"))?;
    let json = Header::from_bytes("Content-Type", "application/json")
        .map_err(|()| anyhow::anyhow!("invalid header"))?;
    let url = request.url().to_owned();

    if request.method() == &Method::Get && url == "/health" {
        request.respond(Response::from_string("ok").with_header(cors))?;
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
