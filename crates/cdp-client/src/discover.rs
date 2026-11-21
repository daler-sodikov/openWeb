use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::CdpError;

/// How long one HTTP attempt may take before it is retried.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
/// Pause between attempts while Node is not listening yet.
const RETRY_PAUSE: Duration = Duration::from_millis(40);
/// Upper bound for the answer; `/json/list` is one small document.
const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Target {
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

/// Finds the inspector's WebSocket URL: `GET http://127.0.0.1:<port>/json/list`,
/// retried until Node is listening, `deadline` has passed
/// ([`CdpError::Timeout`]) or `cancel` fires ([`CdpError::Cancelled`]).
///
/// No HTTP client is involved: one HTTP/1.1 request over a plain TCP stream.
pub async fn discover(
    port: u16,
    deadline: Duration,
    cancel: &CancellationToken,
) -> Result<String, CdpError> {
    let end = Instant::now() + deadline;
    loop {
        let attempt = async {
            tokio::time::timeout(ATTEMPT_TIMEOUT, list_once(port))
                .await
                .unwrap_or(Err(CdpError::Timeout))
        };
        let result = tokio::select! {
            () = cancel.cancelled() => return Err(CdpError::Cancelled),
            () = tokio::time::sleep_until(end) => return Err(CdpError::Timeout),
            r = attempt => r,
        };
        match result {
            Ok(url) => return Ok(url),
            Err(error) => tracing::debug!(port, %error, "inspector not ready"),
        }
        tokio::select! {
            () = cancel.cancelled() => return Err(CdpError::Cancelled),
            () = tokio::time::sleep_until(end) => return Err(CdpError::Timeout),
            () = tokio::time::sleep(RETRY_PAUSE) => {}
        }
    }
}

async fn list_once(port: u16) -> Result<String, CdpError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect(addr).await.map_err(transport)?;
    let request = format!(
        "GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(transport)?;
    // Node answers HTTP/1.0 but keeps the socket open despite
    // `Connection: close`, so waiting for EOF would hang: stop as soon as the
    // announced `Content-Length` bytes are in. Without that header the body
    // ends at EOF. The size cap bounds what a misbehaving peer can make grow.
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !response_complete(&raw) {
        if raw.len() >= MAX_RESPONSE {
            return Err(CdpError::Transport("response too large".into()));
        }
        let n = stream.read(&mut chunk).await.map_err(transport)?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
    }
    let body = http_body(&raw)?;
    let targets: Vec<Target> =
        serde_json::from_slice(&body).map_err(|e| CdpError::Decode(e.to_string()))?;
    targets
        .into_iter()
        .find_map(|t| t.web_socket_debugger_url)
        .ok_or_else(|| CdpError::Transport("no inspector target listed".into()))
}

/// True when `raw` holds the headers and as many body bytes as the
/// `Content-Length` header announces.
fn response_complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let Ok(head) = std::str::from_utf8(&raw[..split]) else {
        return false;
    };
    let length = head.split("\r\n").find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    length.is_some_and(|n| raw.len() - (split + 4) >= n)
}

fn transport(error: std::io::Error) -> CdpError {
    CdpError::Transport(error.to_string())
}

/// Checks the status line and returns the body (decoding `chunked`).
fn http_body(raw: &[u8]) -> Result<Vec<u8>, CdpError> {
    let bad = |what: &str| CdpError::Transport(format!("bad HTTP response: {what}"));
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| bad("no header end"))?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| bad("headers are not UTF-8"))?;
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        return Err(bad(status));
    }
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    if chunked {
        dechunk(body).ok_or_else(|| bad("broken chunked body"))
    } else {
        Ok(body.to_vec())
    }
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = body.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&body[..eol]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        body = &body[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}
