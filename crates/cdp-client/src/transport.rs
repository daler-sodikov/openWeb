use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::CdpError;

/// Default time a call waits for its answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed for the TCP connect and the WebSocket handshake together.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time `close` waits for the connection task to finish.
const CLOSE_WAIT: Duration = Duration::from_secs(2);
/// Events buffered between the connection task and the consumer. When full,
/// the task stops reading the socket, so a flooding debuggee is throttled by
/// TCP rather than buffered without bound.
const EVENT_BUFFER: usize = 256;
/// Calls queued for the connection task.
const COMMAND_BUFFER: usize = 64;

/// An event from the inspector, or the end of the connection.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CdpEvent {
    /// A protocol event (`Debugger.paused`, `Runtime.consoleAPICalled`, ...).
    /// The typed subset is layered on top of this by the protocol module.
    Event {
        /// The event name, e.g. `Debugger.paused`.
        method: String,
        /// The raw `params` object (`null` when absent).
        params: Value,
    },
    /// The connection ended. Always the last event.
    Closed {
        /// A short human-readable reason.
        reason: String,
    },
}

/// Events and the end of the connection, in arrival order.
#[derive(Debug)]
pub struct CdpEvents {
    rx: mpsc::Receiver<CdpEvent>,
}

impl CdpEvents {
    /// The next event; `None` after [`CdpEvent::Closed`] was delivered.
    pub async fn recv(&mut self) -> Option<CdpEvent> {
        self.rx.recv().await
    }
}

enum Command {
    Call {
        id: u64,
        text: String,
        reply: oneshot::Sender<Result<Value, CdpError>>,
    },
    /// The caller stopped waiting (timeout, drop): forget the id.
    Forget(u64),
    Close,
}

struct Shared {
    next_id: AtomicU64,
    commands: mpsc::Sender<Command>,
    finished: watch::Receiver<bool>,
    cancel: CancellationToken,
}

/// A connection to Node's inspector. Cheap to clone: every clone is a handle
/// to the same connection task.
#[derive(Clone)]
pub struct CdpClient {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for CdpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpClient").finish_non_exhaustive()
    }
}

impl CdpClient {
    /// Connects to a `ws://127.0.0.1:<port>/<path>` URL. Any other URL is
    /// refused with [`CdpError::Refused`] before a socket is opened.
    ///
    /// Cancelling `cancel` ends the connection later too: it closes the
    /// socket and fails every pending call with [`CdpError::Cancelled`].
    pub async fn connect(
        url: &str,
        cancel: CancellationToken,
    ) -> Result<(CdpClient, CdpEvents), CdpError> {
        let addr = loopback_addr(url)?;
        let handshake = async {
            let stream = TcpStream::connect(addr)
                .await
                .map_err(|e| CdpError::Transport(e.to_string()))?;
            tokio_tungstenite::client_async(url, stream)
                .await
                .map(|(ws, _response)| ws)
                .map_err(|e| CdpError::Transport(e.to_string()))
        };
        let ws = tokio::select! {
            () = cancel.cancelled() => return Err(CdpError::Cancelled),
            r = tokio::time::timeout(CONNECT_TIMEOUT, handshake) => match r {
                Ok(connected) => connected?,
                Err(_) => return Err(CdpError::Timeout),
            },
        };

        let (commands, command_rx) = mpsc::channel(COMMAND_BUFFER);
        let (event_tx, event_rx) = mpsc::channel(EVENT_BUFFER);
        let (finished_tx, finished) = watch::channel(false);
        tokio::spawn(run(ws, command_rx, event_tx, cancel.clone(), finished_tx));
        let shared = Arc::new(Shared {
            next_id: AtomicU64::new(1),
            commands,
            finished,
            cancel,
        });
        Ok((CdpClient { shared }, CdpEvents { rx: event_rx }))
    }

    /// One request; resolves with the result or the protocol error. Waits at
    /// most ten seconds ([`CdpError::Timeout`]).
    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, CdpError> {
        let params = serde_json::to_value(params).map_err(|e| CdpError::Decode(e.to_string()))?;
        let value = self.call_raw(method, params, CALL_TIMEOUT).await?;
        serde_json::from_value(value).map_err(|e| CdpError::Decode(e.to_string()))
    }

    pub(crate) async fn call_raw(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        let shared = &self.shared;
        let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = serde_json::json!({ "id": id, "method": method });
        if !params.is_null() {
            request["params"] = params;
        }
        let (reply, answer) = oneshot::channel();
        let command = Command::Call {
            id,
            text: request.to_string(),
            reply,
        };
        tokio::select! {
            () = shared.cancel.cancelled() => return Err(CdpError::Cancelled),
            sent = shared.commands.send(command) => {
                if sent.is_err() {
                    return Err(self.ended_error());
                }
            }
        }
        tokio::select! {
            () = shared.cancel.cancelled() => Err(CdpError::Cancelled),
            r = tokio::time::timeout(timeout, answer) => match r {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(self.ended_error()),
                Err(_) => {
                    // Best effort: the answer, if it ever comes, is dropped.
                    let _ = shared.commands.try_send(Command::Forget(id));
                    Err(CdpError::Timeout)
                }
            },
        }
    }

    fn ended_error(&self) -> CdpError {
        if self.shared.cancel.is_cancelled() {
            CdpError::Cancelled
        } else {
            CdpError::Closed
        }
    }

    /// Closes the socket and waits for the connection task to end. Pending
    /// calls fail with [`CdpError::Closed`]. Idempotent.
    pub async fn close(&self) {
        let _ = self.shared.commands.send(Command::Close).await;
        let mut finished = self.shared.finished.clone();
        let _ = tokio::time::timeout(CLOSE_WAIT, finished.wait_for(|done| *done)).await;
    }
}

/// Accepts exactly `ws://127.0.0.1:<port>[/path]` and returns the address.
fn loopback_addr(url: &str) -> Result<SocketAddr, CdpError> {
    let refused = |why: &str| CdpError::Refused(format!("{why}: {url}"));
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| refused("only ws:// URLs are allowed"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let addr: SocketAddr = authority
        .parse()
        .map_err(|_| refused("the host must be 127.0.0.1 with a port"))?;
    if addr.ip() != Ipv4Addr::LOCALHOST || addr.port() == 0 {
        return Err(refused("only 127.0.0.1 is allowed"));
    }
    Ok(addr)
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

/// The connection task: the only owner of the socket and the pending map.
async fn run(
    mut ws: Ws,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<CdpEvent>,
    cancel: CancellationToken,
    finished: watch::Sender<bool>,
) {
    let mut pending: HashMap<u64, oneshot::Sender<Result<Value, CdpError>>> = HashMap::new();
    let mut consumer_gone = false;

    let (end_reason, fail_with) = loop {
        tokio::select! {
            () = cancel.cancelled() => break ("cancelled".to_string(), CdpError::Cancelled),
            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    break ("closed by this side".to_string(), CdpError::Closed);
                }
                Some(Command::Forget(id)) => {
                    pending.remove(&id);
                }
                Some(Command::Call { id, text, reply }) => {
                    match ws.send(Message::text(text)).await {
                        Ok(()) => {
                            pending.insert(id, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(CdpError::Transport(error.to_string())));
                            break (format!("send failed: {error}"), CdpError::Closed);
                        }
                    }
                }
            },
            message = ws.next() => match message {
                None => break ("the inspector closed the connection".to_string(), CdpError::Closed),
                Some(Err(error)) => break (format!("socket error: {error}"), CdpError::Closed),
                Some(Ok(Message::Close(_))) => {
                    break ("the inspector closed the connection".to_string(), CdpError::Closed);
                }
                Some(Ok(Message::Text(text))) => {
                    let Some(event) = route(text.as_str(), &mut pending) else { continue };
                    if consumer_gone {
                        continue;
                    }
                    // Backpressure: while the buffer is full the socket is
                    // not read. Cancelling still wins.
                    tokio::select! {
                        () = cancel.cancelled() => {
                            break ("cancelled".to_string(), CdpError::Cancelled);
                        }
                        sent = events.send(event) => consumer_gone = sent.is_err(),
                    }
                }
                // Pings are answered by the library; binary frames are not
                // part of the inspector protocol.
                Some(Ok(_)) => {}
            },
        }
    };

    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(fail_with.clone()));
    }
    // Best effort, bounded: a peer that is already gone must not hold us.
    let _ = tokio::time::timeout(Duration::from_millis(200), ws.close(None)).await;
    drop(ws);
    let _ = finished.send(true);
    if !consumer_gone {
        let _ = events.send(CdpEvent::Closed { reason: end_reason }).await;
    }
}

/// Handles one text frame: answers a pending call, or returns an event.
fn route(
    text: &str,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>,
) -> Option<CdpEvent> {
    let mut message: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "ignoring a malformed inspector message");
            return None;
        }
    };
    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        // An answer for a call that timed out or was never ours: ignored.
        let reply = pending.remove(&id)?;
        let result = match message.get_mut("error") {
            Some(error) => Err(CdpError::Protocol {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }),
            None => Ok(message
                .get_mut("result")
                .map(Value::take)
                .unwrap_or(Value::Null)),
        };
        let _ = reply.send(result);
        return None;
    }
    let method = message.get("method")?.as_str()?.to_string();
    let params = message
        .get_mut("params")
        .map(Value::take)
        .unwrap_or(Value::Null);
    Some(CdpEvent::Event { method, params })
}
