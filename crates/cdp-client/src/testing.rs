//! A scripted fake inspector for tests: it answers `GET /json/list` and
//! speaks the WebSocket on the same loopback port, the way Node does.
//!
//! A test supplies a handler that maps each request to a list of [`Reply`]
//! values, and can push events or drop the connection from the outside.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// One thing the fake server sends in response to a request.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A successful answer carrying this `result`.
    Result(Value),
    /// A protocol error answer.
    Error {
        /// Error code.
        code: i64,
        /// Error message.
        message: String,
    },
    /// An answer sent after a delay (later requests may be answered first).
    ResultAfter(Duration, Value),
    /// An event.
    Event {
        /// Event name.
        method: String,
        /// Event params.
        params: Value,
    },
    /// `count` events named `method`, numbered in `params.n`.
    Flood {
        /// Event name.
        method: String,
        /// How many to send.
        count: u32,
    },
    /// Text sent as is (for malformed input).
    Raw(String),
    /// Close the connection.
    Close,
    /// No answer at all.
    Ignore,
}

enum Out {
    Text(String),
    Close,
}

type Handler = dyn Fn(&Value) -> Vec<Reply> + Send + Sync;
type Writer = Arc<Mutex<Option<mpsc::UnboundedSender<Out>>>>;

/// The fake inspector. Dropping it stops the listener and the connection.
pub struct FakeCdpServer {
    port: u16,
    requests: Arc<Mutex<Vec<Value>>>,
    writer: Writer,
    stop: CancellationToken,
}

impl Drop for FakeCdpServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeCdpServer {
    /// Starts on a random loopback port. `handler` sees each request
    /// (`{"id", "method", "params"}`) and returns the replies to send.
    pub async fn start(
        handler: impl Fn(&Value) -> Vec<Reply> + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        Self::start_on(0, handler).await
    }

    /// Like [`start`](Self::start) on a given port (a test can reserve a
    /// port, start discovery, and only then bring the server up).
    pub async fn start_on(
        port: u16,
        handler: impl Fn(&Value) -> Vec<Reply> + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        let handler: Arc<Handler> = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let writer: Writer = Arc::new(Mutex::new(None));
        let stop = CancellationToken::new();
        let ctx = Ctx {
            port,
            handler,
            requests: requests.clone(),
            writer: writer.clone(),
            stop: stop.clone(),
        };
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = ctx.stop.cancelled() => return,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { return };
                        tokio::spawn(serve(stream, ctx.clone()));
                    }
                }
            }
        });
        Ok(Self {
            port,
            requests,
            writer,
            stop,
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The WebSocket URL `/json/list` advertises.
    pub fn url(&self) -> String {
        ws_url(self.port)
    }

    /// Sends an event on the current connection (no-op without one).
    pub fn send_event(&self, method: &str, params: Value) {
        self.push(Out::Text(
            json!({ "method": method, "params": params }).to_string(),
        ));
    }

    /// Drops the current connection from the server side.
    pub fn close_peer(&self) {
        self.push(Out::Close);
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<Value> {
        lock(&self.requests).clone()
    }

    fn push(&self, out: Out) {
        if let Some(tx) = lock(&self.writer).as_ref() {
            let _ = tx.send(out);
        }
    }
}

fn ws_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}/6f1c2d3e-0000-4000-8000-fake00000000")
}

#[derive(Clone)]
struct Ctx {
    port: u16,
    handler: Arc<Handler>,
    requests: Arc<Mutex<Vec<Value>>>,
    writer: Writer,
    stop: CancellationToken,
}

async fn serve(stream: TcpStream, ctx: Ctx) {
    let mut head = [0u8; 9];
    // Wait until the first bytes tell HTTP discovery from the WebSocket.
    loop {
        match stream.peek(&mut head).await {
            Ok(n) if n >= head.len() => break,
            Ok(0) | Err(_) => return,
            Ok(_) => tokio::time::sleep(Duration::from_millis(2)).await,
        }
    }
    if head.starts_with(b"GET /json") {
        serve_http(stream, ctx.port).await;
    } else if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
        serve_ws(ws, ctx).await;
    }
}

async fn serve_http(mut stream: TcpStream, port: u16) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let body = json!([{
        "description": "node.js instance",
        "id": "fake00000000",
        "title": "fake",
        "type": "node",
        "url": "file:///fake.js",
        "webSocketDebuggerUrl": ws_url(port),
    }])
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn serve_ws(ws: tokio_tungstenite::WebSocketStream<TcpStream>, ctx: Ctx) {
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    *lock(&ctx.writer) = Some(tx.clone());

    let writer_stop = ctx.stop.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = writer_stop.cancelled() => return,
                out = rx.recv() => match out {
                    Some(Out::Text(text)) => {
                        if sink.send(Message::text(text)).await.is_err() {
                            return;
                        }
                    }
                    Some(Out::Close) | None => {
                        let _ = sink.close().await;
                        return;
                    }
                },
            }
        }
    });

    loop {
        let message = tokio::select! {
            () = ctx.stop.cancelled() => break,
            m = stream.next() => m,
        };
        let Some(Ok(Message::Text(text))) = message else {
            break;
        };
        let Ok(request) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        lock(&ctx.requests).push(request.clone());
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        for reply in (ctx.handler)(&request) {
            match reply {
                Reply::Result(result) => {
                    let _ = tx.send(Out::Text(json!({ "id": id, "result": result }).to_string()));
                }
                Reply::Error { code, message } => {
                    let error = json!({ "code": code, "message": message });
                    let _ = tx.send(Out::Text(json!({ "id": id, "error": error }).to_string()));
                }
                Reply::ResultAfter(delay, result) => {
                    let tx = tx.clone();
                    let id = id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ =
                            tx.send(Out::Text(json!({ "id": id, "result": result }).to_string()));
                    });
                }
                Reply::Event { method, params } => {
                    let _ = tx.send(Out::Text(
                        json!({ "method": method, "params": params }).to_string(),
                    ));
                }
                Reply::Flood { method, count } => {
                    for n in 0..count {
                        let _ = tx.send(Out::Text(
                            json!({ "method": method, "params": { "n": n } }).to_string(),
                        ));
                    }
                }
                Reply::Raw(text) => {
                    let _ = tx.send(Out::Text(text));
                }
                Reply::Close => {
                    let _ = tx.send(Out::Close);
                }
                Reply::Ignore => {}
            }
        }
    }
    drop(tx);
    // Let queued output (a final close) go out before the socket is dropped.
    let _ = tokio::time::timeout(Duration::from_millis(500), writer).await;
}
