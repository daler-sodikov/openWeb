//! A JSON-RPC 2.0 connection over a pair of byte streams: numbered
//! requests with cancellation, notifications, and server→client requests.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::LspError;
use crate::framing::{read_message, write_message};

/// JSON-RPC error codes used here.
pub mod codes {
    /// The method is not handled.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// LSP: the request was cancelled.
    pub const REQUEST_CANCELLED: i64 = -32800;
    /// LSP: the document changed so the result is no longer valid.
    pub const CONTENT_MODIFIED: i64 = -32801;
}

/// An error object of a JSON-RPC response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseError {
    /// Error code.
    pub code: i64,
    /// Message.
    pub message: String,
    /// Optional data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ResponseError {
    /// A `MethodNotFound` error for `method`.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: codes::METHOD_NOT_FOUND,
            message: format!("method not handled by the client: {method}"),
            data: None,
        }
    }
}

/// Handles messages the server sends on its own. Called on the connection's
/// reader task, so implementations must return quickly and never block.
pub trait Incoming: Send + Sync + 'static {
    /// A notification from the server.
    fn notification(&self, method: &str, params: Value);
    /// A request from the server; the result is sent back as the response.
    fn request(&self, method: &str, params: Value) -> Result<Value, ResponseError>;
}

type Reply = oneshot::Sender<Result<Value, ResponseError>>;

struct Inner {
    name: String,
    out: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<i64, Reply>>,
    next_id: AtomicI64,
    closed: CancellationToken,
}

impl Inner {
    fn pending(&self) -> MutexGuard<'_, HashMap<i64, Reply>> {
        // A poisoned map only means a panic elsewhere; its data is still valid.
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send(&self, message: &Value) -> Result<(), LspError> {
        if self.closed.is_cancelled() {
            return Err(LspError::Closed);
        }
        self.out
            .send(message.to_string())
            .map_err(|_| LspError::Closed)
    }

    fn cancel_request(&self, id: i64) {
        if self.pending().remove(&id).is_some() {
            // Best effort: the server may already be gone.
            let _ = self.send(&json!({
                "jsonrpc": "2.0",
                "method": "$/cancelRequest",
                "params": { "id": id },
            }));
        }
    }
}

/// A connection to one server. Cheap to clone.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Starts reading from `reader` and writing to `writer` on background
    /// tasks (must be called inside a tokio runtime). `name` labels logs.
    pub fn new<R, W>(name: &str, reader: R, writer: W, incoming: Arc<dyn Incoming>) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            name: name.to_owned(),
            out,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            closed: CancellationToken::new(),
        });
        tokio::spawn(write_loop(inner.clone(), writer, rx));
        tokio::spawn(read_loop(inner.clone(), reader, incoming));
        Self { inner }
    }

    /// Resolves when the connection has closed (the server's output ended or
    /// could not be read or written).
    pub fn closed(&self) -> impl Future<Output = ()> + Send + 'static {
        self.inner.closed.clone().cancelled_owned()
    }

    /// Whether the connection has closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.is_cancelled()
    }

    /// Sends a request now and returns a handle to await its response. The
    /// message is queued synchronously, so it is ordered after everything
    /// sent before this call.
    pub fn start_request<R: Request>(
        &self,
        params: R::Params,
    ) -> Result<PendingRequest<R::Result>, LspError> {
        let params = serde_json::to_value(params).map_err(|e| LspError::Protocol(e.to_string()))?;
        self.start_raw(R::METHOD, params).map(PendingRequest::typed)
    }

    /// Sends a request and waits for its result, cancelling it when `cancel`
    /// fires.
    pub async fn request<R: Request>(
        &self,
        params: R::Params,
        cancel: &CancellationToken,
    ) -> Result<R::Result, LspError> {
        self.start_request::<R>(params)?.wait(cancel).await
    }

    /// [`Connection::start_request`] with an untyped method and parameters.
    pub fn start_raw(
        &self,
        method: &str,
        params: Value,
    ) -> Result<PendingRequest<Value>, LspError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending().insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(err) = self.inner.send(&message) {
            self.inner.pending().remove(&id);
            return Err(err);
        }
        Ok(PendingRequest {
            guard: CancelGuard {
                inner: self.inner.clone(),
                id,
                done: false,
            },
            rx,
            _result: PhantomData,
        })
    }

    /// Sends a notification.
    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<(), LspError> {
        let params = serde_json::to_value(params).map_err(|e| LspError::Protocol(e.to_string()))?;
        self.notify_raw(N::METHOD, params)
    }

    /// Sends a notification with an untyped method and parameters.
    pub fn notify_raw(&self, method: &str, params: Value) -> Result<(), LspError> {
        self.inner
            .send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }
}

/// Cancels the request on the server if its handle is dropped unanswered.
struct CancelGuard {
    inner: Arc<Inner>,
    id: i64,
    done: bool,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.done {
            self.inner.cancel_request(self.id);
        }
    }
}

/// A request that has been sent. Dropping it unanswered cancels it.
pub struct PendingRequest<T> {
    guard: CancelGuard,
    rx: oneshot::Receiver<Result<Value, ResponseError>>,
    _result: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for PendingRequest<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingRequest")
            .field("id", &self.guard.id)
            .finish_non_exhaustive()
    }
}

impl PendingRequest<Value> {
    fn typed<T>(self) -> PendingRequest<T> {
        PendingRequest {
            guard: self.guard,
            rx: self.rx,
            _result: PhantomData,
        }
    }
}

impl<T: serde::de::DeserializeOwned> PendingRequest<T> {
    /// The request id.
    pub fn id(&self) -> i64 {
        self.guard.id
    }

    /// Waits for the response. When `cancel` fires first, sends
    /// `$/cancelRequest` and returns [`LspError::Cancelled`].
    pub async fn wait(mut self, cancel: &CancellationToken) -> Result<T, LspError> {
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            reply = &mut self.rx => Some(reply),
        };
        let Some(reply) = outcome else {
            // The guard sends $/cancelRequest when dropped.
            return Err(LspError::Cancelled);
        };
        self.guard.done = true;
        match reply {
            Ok(Ok(value)) => {
                serde_json::from_value(value).map_err(|e| LspError::Protocol(e.to_string()))
            }
            Ok(Err(error)) => Err(match error.code {
                codes::REQUEST_CANCELLED | codes::CONTENT_MODIFIED => LspError::Cancelled,
                code => LspError::Server {
                    code,
                    message: error.message,
                },
            }),
            Err(_) => Err(LspError::Closed),
        }
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(
    inner: Arc<Inner>,
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(message) = rx.recv().await {
        if let Err(err) = write_message(&mut writer, &message).await {
            tracing::warn!(server = %inner.name, %err, "writing to the language server failed");
            inner.closed.cancel();
            break;
        }
    }
}

async fn read_loop<R: AsyncRead + Unpin>(
    inner: Arc<Inner>,
    reader: R,
    incoming: Arc<dyn Incoming>,
) {
    let mut reader = BufReader::new(reader);
    loop {
        match read_message(&mut reader).await {
            Ok(Some(text)) => dispatch(&inner, incoming.as_ref(), &text),
            Ok(None) => {
                tracing::debug!(server = %inner.name, "language server output ended");
                break;
            }
            Err(err) => {
                tracing::warn!(server = %inner.name, %err, "reading from the language server failed");
                break;
            }
        }
    }
    inner.closed.cancel();
    // Dropping the reply senders fails every waiting request with `Closed`.
    inner.pending().clear();
}

fn dispatch(inner: &Inner, incoming: &dyn Incoming, text: &str) {
    let message: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(server = %inner.name, %err, "ignoring a message that is not JSON");
            return;
        }
    };
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").filter(|id| !id.is_null());
    match (method, id) {
        (Some(method), Some(id)) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let response = match incoming.request(method, params) {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            };
            if inner.send(&response).is_err() {
                tracing::debug!(server = %inner.name, method, "could not answer: connection closed");
            }
        }
        (Some("$/cancelRequest"), None) => {
            // We answer server requests synchronously; nothing to cancel.
        }
        (Some(method), None) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            incoming.notification(method, params);
        }
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else {
                tracing::warn!(server = %inner.name, %id, "response with an id we never sent");
                return;
            };
            let Some(reply) = inner.pending().remove(&id) else {
                // Cancelled or timed out on our side.
                return;
            };
            let result = match message.get("error") {
                Some(error) => {
                    Err(
                        serde_json::from_value(error.clone()).unwrap_or_else(|_| ResponseError {
                            code: 0,
                            message: error.to_string(),
                            data: None,
                        }),
                    )
                }
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            // The waiter may have gone away; that is fine.
            let _ = reply.send(result);
        }
        (None, None) => {
            tracing::warn!(server = %inner.name, "ignoring a message without method or id");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lsp_types::request::{HoverRequest, Shutdown};
    use tokio::io::{BufReader, DuplexStream};

    use super::*;

    /// The server side of an in-memory connection.
    struct Peer {
        reader: BufReader<DuplexStream>,
        writer: DuplexStream,
    }

    impl Peer {
        async fn recv(&mut self) -> Value {
            let text = read_message(&mut self.reader).await.unwrap().unwrap();
            serde_json::from_str(&text).unwrap()
        }

        async fn send(&mut self, value: Value) {
            write_message(&mut self.writer, &value.to_string())
                .await
                .unwrap();
        }
    }

    #[derive(Default)]
    struct Recorder {
        notifications: Mutex<Vec<(String, Value)>>,
    }

    impl Incoming for Recorder {
        fn notification(&self, method: &str, params: Value) {
            self.notifications
                .lock()
                .unwrap()
                .push((method.to_owned(), params));
        }

        fn request(&self, method: &str, params: Value) -> Result<Value, ResponseError> {
            match method {
                "workspace/configuration" => Ok(json!([params["items"].as_array().unwrap().len()])),
                _ => Err(ResponseError::method_not_found(method)),
            }
        }
    }

    fn connect() -> (Connection, Peer, Arc<Recorder>) {
        let (client_read, server_write) = tokio::io::duplex(1 << 16);
        let (server_read, client_write) = tokio::io::duplex(1 << 16);
        let recorder = Arc::new(Recorder::default());
        let connection = Connection::new("test", client_read, client_write, recorder.clone());
        (
            connection,
            Peer {
                reader: BufReader::new(server_read),
                writer: server_write,
            },
            recorder,
        )
    }

    #[tokio::test]
    async fn matches_responses_to_requests_out_of_order() {
        let (connection, mut peer, _) = connect();
        let never = CancellationToken::new();
        let first = connection.start_raw("a", json!(1)).unwrap();
        let second = connection.start_raw("b", json!(2)).unwrap();
        let (m1, m2) = (peer.recv().await, peer.recv().await);
        assert_eq!(m1["method"], "a");
        assert_eq!(m2["method"], "b");
        assert_ne!(m1["id"], m2["id"]);
        peer.send(json!({ "jsonrpc": "2.0", "id": m2["id"], "result": "two" }))
            .await;
        peer.send(json!({ "jsonrpc": "2.0", "id": m1["id"], "result": "one" }))
            .await;
        assert_eq!(second.wait(&never).await.unwrap(), json!("two"));
        assert_eq!(first.wait(&never).await.unwrap(), json!("one"));
    }

    #[tokio::test]
    async fn cancellation_sends_cancel_request_and_ignores_late_responses() {
        let (connection, mut peer, _) = connect();
        let cancel = CancellationToken::new();
        let request = tokio::spawn({
            let connection = connection.clone();
            let cancel = cancel.clone();
            async move {
                let params = serde_json::from_value(json!({
                    "textDocument": { "uri": "file:///a.ts" },
                    "position": { "line": 0, "character": 0 },
                }))
                .unwrap();
                connection.request::<HoverRequest>(params, &cancel).await
            }
        });
        let sent = peer.recv().await;
        assert_eq!(sent["method"], "textDocument/hover");
        cancel.cancel();
        assert!(matches!(request.await.unwrap(), Err(LspError::Cancelled)));
        let cancel_message = peer.recv().await;
        assert_eq!(cancel_message["method"], "$/cancelRequest");
        assert_eq!(cancel_message["params"]["id"], sent["id"]);
        // A late response is ignored; the connection keeps working.
        peer.send(json!({ "jsonrpc": "2.0", "id": sent["id"], "result": null }))
            .await;
        let next = connection.start_raw("next", Value::Null).unwrap();
        let next_sent = peer.recv().await;
        peer.send(json!({ "jsonrpc": "2.0", "id": next_sent["id"], "result": 5 }))
            .await;
        assert_eq!(
            next.wait(&CancellationToken::new()).await.unwrap(),
            json!(5)
        );
    }

    #[tokio::test]
    async fn dropping_a_pending_request_cancels_it() {
        let (connection, mut peer, _) = connect();
        let pending = connection.start_raw("slow", Value::Null).unwrap();
        let sent = peer.recv().await;
        drop(pending);
        let cancel_message = peer.recv().await;
        assert_eq!(cancel_message["method"], "$/cancelRequest");
        assert_eq!(cancel_message["params"]["id"], sent["id"]);
    }

    #[tokio::test]
    async fn maps_server_errors() {
        let (connection, mut peer, _) = connect();
        let never = CancellationToken::new();
        let cancelled = connection.start_raw("a", Value::Null).unwrap();
        let failed = connection.start_raw("b", Value::Null).unwrap();
        let (m1, m2) = (peer.recv().await, peer.recv().await);
        peer.send(json!({ "jsonrpc": "2.0", "id": m1["id"],
            "error": { "code": codes::CONTENT_MODIFIED, "message": "modified" } }))
            .await;
        peer.send(json!({ "jsonrpc": "2.0", "id": m2["id"],
            "error": { "code": -32603, "message": "boom" } }))
            .await;
        assert!(matches!(
            cancelled.wait(&never).await,
            Err(LspError::Cancelled)
        ));
        assert!(matches!(
            failed.wait(&never).await,
            Err(LspError::Server { code: -32603, ref message }) if message == "boom"
        ));
    }

    #[tokio::test]
    async fn answers_server_requests_and_forwards_notifications() {
        let (_connection, mut peer, recorder) = connect();
        peer.send(
            json!({ "jsonrpc": "2.0", "id": "cfg-1", "method": "workspace/configuration",
            "params": { "items": [{}, {}] } }),
        )
        .await;
        let answer = peer.recv().await;
        assert_eq!(answer["id"], "cfg-1");
        assert_eq!(answer["result"], json!([2]));
        peer.send(json!({ "jsonrpc": "2.0", "id": 7, "method": "unknown/thing" }))
            .await;
        let answer = peer.recv().await;
        assert_eq!(answer["error"]["code"], codes::METHOD_NOT_FOUND);
        peer.send(json!({ "jsonrpc": "2.0", "method": "window/logMessage",
            "params": { "type": 3, "message": "hi" } }))
            .await;
        // Wait until the reader task has dispatched it.
        for _ in 0..100 {
            if !recorder.notifications.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let notifications = recorder.notifications.lock().unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].0, "window/logMessage");
    }

    #[tokio::test]
    async fn closing_fails_pending_and_new_requests() {
        let (connection, peer, _) = connect();
        let pending = connection.start_raw("a", Value::Null).unwrap();
        drop(peer);
        connection.closed().await;
        assert!(matches!(
            pending.wait(&CancellationToken::new()).await,
            Err(LspError::Closed)
        ));
        assert!(matches!(
            connection
                .request::<Shutdown>((), &CancellationToken::new())
                .await,
            Err(LspError::Closed)
        ));
        assert!(connection.notify_raw("x", Value::Null).is_err());
    }
}
