//! A supervised language server: spawns the process, initializes it,
//! restarts it with backoff when it dies, stops it, and keeps documents in
//! sync with a debounced, generation-checked `didChange` queue
//! (`docs/ARCHITECTURE.md`, 4.7).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use ferrule_process::ProcessTree;
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Exit,
    Initialized, Notification,
};
use lsp_types::request::{Request, Shutdown};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeResult, InitializedParams, PositionEncodingKind,
    ServerCapabilities, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncSaveOptions, Uri,
    VersionedTextDocumentIdentifier,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::LspError;
use crate::connection::{Connection, Incoming, PendingRequest, ResponseError};
use crate::uri::path_to_uri;

/// How to start a server process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCommand {
    /// The executable.
    pub program: PathBuf,
    /// Arguments (e.g. `--stdio`).
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// When and how often a crashed server is restarted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPolicy {
    /// Delay before the first restart; doubled for each further one.
    pub base_delay: Duration,
    /// Restarts in a row before giving up.
    pub max_attempts: u32,
    /// A server that ran this long resets the count.
    pub stable_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(500),
            max_attempts: 5,
            stable_after: Duration::from_secs(60),
        }
    }
}

/// Everything needed to run one server for one project.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Display name ("TypeScript").
    pub name: String,
    /// The process to start.
    pub command: ServerCommand,
    /// Project root (`rootUri`, first workspace folder).
    pub root: PathBuf,
    /// More workspace folders (e.g. monorepo packages).
    pub workspace_folders: Vec<PathBuf>,
    /// Server-specific `initializationOptions`.
    pub initialization_options: Option<Value>,
    /// Restart behaviour.
    pub restart: RestartPolicy,
    /// Delay after the last edit before `didChange` is sent.
    pub debounce: Duration,
    /// Longest wait for the `initialize` response.
    pub startup_timeout: Duration,
}

impl ServerConfig {
    /// A configuration with default timing.
    pub fn new(name: impl Into<String>, command: ServerCommand, root: PathBuf) -> Self {
        Self {
            name: name.into(),
            command,
            root,
            workspace_folders: Vec::new(),
            initialization_options: None,
            restart: RestartPolicy::default(),
            debounce: Duration::from_millis(100),
            startup_timeout: Duration::from_secs(60),
        }
    }
}

/// Lifecycle state, reported through [`ClientHooks::state_changed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerState {
    /// Spawning and initializing.
    Starting,
    /// Running, with work-done progress open (the title of the latest).
    Busy(String),
    /// Running and idle.
    Ready,
    /// Died; restarting after `delay` (attempt counts from 1).
    Restarting {
        /// Restart attempt.
        attempt: u32,
        /// Wait before the attempt.
        delay: Duration,
    },
    /// Gave up after repeated failures.
    Failed(String),
    /// Stopped on request.
    Stopped,
}

/// Callbacks from the supervisor. Called from async tasks: they must not
/// block for long.
pub trait ClientHooks: Send + Sync + 'static {
    /// The state changed.
    fn state_changed(&self, state: &ServerState);
    /// A server generation finished initializing: open documents now (with
    /// [`Supervisor::did_open`] for `generation`).
    fn started(&self, generation: u64);
    /// A notification from the server (`$/progress` is also handled here).
    fn notification(&self, method: &str, params: &Value);
    /// A request from the server the client wants to answer itself (e.g.
    /// `workspace/applyEdit`); `None` falls back to the built-in answers.
    /// Called on the reader task: must not block.
    fn request(&self, method: &str, params: &Value) -> Option<Result<Value, ResponseError>> {
        let _ = (method, params);
        None
    }
}

/// A supervised server. Cheap to clone; the server stops when the last
/// clone is dropped or [`Supervisor::stop`] is called.
#[derive(Clone)]
pub struct Supervisor {
    handle: Arc<Handle>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("name", &self.handle.shared.config.name)
            .finish_non_exhaustive()
    }
}

struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shared.stop.cancel();
    }
}

struct Shared {
    config: ServerConfig,
    hooks: Arc<dyn ClientHooks>,
    sync: Mutex<SyncState>,
    status: Mutex<StatusState>,
    stop: CancellationToken,
    done: CancellationToken,
    flush: Notify,
}

#[derive(Default)]
struct SyncState {
    running: Option<Running>,
    next_generation: u64,
    pending: HashMap<String, PendingDoc>,
    last_edit: Option<Instant>,
}

struct Running {
    generation: u64,
    conn: Connection,
    capabilities: Arc<ServerCapabilities>,
}

struct PendingDoc {
    uri: Uri,
    version: i32,
    changes: Vec<TextDocumentContentChangeEvent>,
}

struct StatusState {
    state: ServerState,
    /// Open work-done progress: token → title.
    progress: HashMap<String, String>,
    server: Option<String>,
    running: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded data stays consistent even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Supervisor {
    /// Starts supervising: the server is spawned in the background. Must be
    /// called inside a tokio runtime.
    pub fn start(config: ServerConfig, hooks: Arc<dyn ClientHooks>) -> Self {
        let shared = Arc::new(Shared {
            config,
            hooks,
            sync: Mutex::new(SyncState::default()),
            status: Mutex::new(StatusState {
                state: ServerState::Starting,
                progress: HashMap::new(),
                server: None,
                running: false,
            }),
            stop: CancellationToken::new(),
            done: CancellationToken::new(),
            flush: Notify::new(),
        });
        tokio::spawn(run(shared.clone()));
        tokio::spawn(flusher(shared.clone()));
        Self {
            handle: Arc::new(Handle { shared }),
        }
    }

    fn shared(&self) -> &Shared {
        &self.handle.shared
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.shared().config.name
    }

    /// The configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.shared().config
    }

    /// Current state.
    pub fn state(&self) -> ServerState {
        lock(&self.shared().status).state.clone()
    }

    /// `name version` from the server's `initialize` result, if it said.
    pub fn server_info(&self) -> Option<String> {
        lock(&self.shared().status).server.clone()
    }

    /// The generation of the running server, `None` while not running.
    pub fn generation(&self) -> Option<u64> {
        lock(&self.shared().sync)
            .running
            .as_ref()
            .map(|r| r.generation)
    }

    /// Capabilities of the running server.
    pub fn capabilities(&self) -> Option<Arc<ServerCapabilities>> {
        lock(&self.shared().sync)
            .running
            .as_ref()
            .map(|r| r.capabilities.clone())
    }

    /// Sends `didOpen` if `generation` is the running one; `false` otherwise
    /// (the document is opened again by [`ClientHooks::started`]).
    pub fn did_open(
        &self,
        generation: u64,
        uri: Uri,
        language_id: &str,
        version: i32,
        text: String,
    ) -> bool {
        let mut sync = lock(&self.shared().sync);
        let Some(running) = sync.current(generation) else {
            return false;
        };
        if open_close(&running.capabilities) {
            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: language_id.to_owned(),
                    version,
                    text,
                },
            };
            send_notification::<DidOpenTextDocument>(&running.conn, params);
        }
        sync.pending.remove(uri.as_str());
        true
    }

    /// Queues edits for `didChange` (sent after the debounce delay, or by
    /// [`Supervisor::flush`]). `changes` are incremental changes in
    /// application order; `full_text` is called instead for servers that
    /// only accept full-text sync. `false` if `generation` is not running.
    pub fn did_change(
        &self,
        generation: u64,
        uri: &Uri,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
        full_text: impl FnOnce() -> String,
    ) -> bool {
        {
            let mut sync = lock(&self.shared().sync);
            let Some(running) = sync.current(generation) else {
                return false;
            };
            let kind = change_kind(&running.capabilities);
            if kind == TextDocumentSyncKind::NONE {
                return true;
            }
            let entry = sync
                .pending
                .entry(uri.as_str().to_owned())
                .or_insert_with(|| PendingDoc {
                    uri: uri.clone(),
                    version,
                    changes: Vec::new(),
                });
            entry.version = version;
            if kind == TextDocumentSyncKind::FULL {
                entry.changes = vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: full_text(),
                }];
            } else {
                entry.changes.extend(changes);
            }
            sync.last_edit = Some(Instant::now());
        }
        self.shared().flush.notify_one();
        true
    }

    /// Sends the pending changes of `uri` now.
    pub fn flush(&self, uri: &Uri) {
        lock(&self.shared().sync).flush_uri(uri);
    }

    /// Flushes and sends `didSave`. `false` if `generation` is not running.
    pub fn did_save(&self, generation: u64, uri: &Uri) -> bool {
        let mut sync = lock(&self.shared().sync);
        if sync.current(generation).is_none() {
            return false;
        }
        sync.flush_uri(uri);
        if let Some(running) = sync.current(generation)
            && save_notifications(&running.capabilities)
        {
            let params = DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                text: None,
            };
            send_notification::<DidSaveTextDocument>(&running.conn, params);
        }
        true
    }

    /// Flushes and sends `didClose`. `false` if `generation` is not running.
    pub fn did_close(&self, generation: u64, uri: &Uri) -> bool {
        let mut sync = lock(&self.shared().sync);
        if sync.current(generation).is_none() {
            return false;
        }
        sync.flush_uri(uri);
        if let Some(running) = sync.current(generation)
            && open_close(&running.capabilities)
        {
            let params = DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
            };
            send_notification::<DidCloseTextDocument>(&running.conn, params);
        }
        true
    }

    /// Sends a request to the running server now (ordered after everything
    /// sent before) and returns a handle to await the response.
    pub fn start_request<R: Request>(
        &self,
        params: R::Params,
    ) -> Result<PendingRequest<R::Result>, LspError> {
        let sync = lock(&self.shared().sync);
        let running = sync.running.as_ref().ok_or(LspError::NotRunning)?;
        running.conn.start_request::<R>(params)
    }

    /// Sends a request and waits for the result (or cancellation).
    pub async fn request<R: Request>(
        &self,
        params: R::Params,
        cancel: &CancellationToken,
    ) -> Result<R::Result, LspError> {
        self.start_request::<R>(params)?.wait(cancel).await
    }

    /// Sends a notification to the running server.
    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<(), LspError> {
        let sync = lock(&self.shared().sync);
        let running = sync.running.as_ref().ok_or(LspError::NotRunning)?;
        running.conn.notify::<N>(params)
    }

    /// Stops the server (shutdown, exit, kill if needed) and resolves when it
    /// is gone. Restarts end too.
    pub fn stop(&self) -> impl Future<Output = ()> + Send + 'static {
        self.shared().stop.cancel();
        self.shared().done.clone().cancelled_owned()
    }
}

impl SyncState {
    fn current(&self, generation: u64) -> Option<&Running> {
        self.running
            .as_ref()
            .filter(|running| running.generation == generation)
    }

    fn flush_uri(&mut self, uri: &Uri) {
        if let Some(doc) = self.pending.remove(uri.as_str()) {
            self.send_changes(doc);
        }
        if self.pending.is_empty() {
            self.last_edit = None;
        }
    }

    fn flush_all(&mut self) {
        let docs: Vec<PendingDoc> = self.pending.drain().map(|(_, doc)| doc).collect();
        for doc in docs {
            self.send_changes(doc);
        }
        self.last_edit = None;
    }

    fn send_changes(&self, doc: PendingDoc) {
        let Some(running) = &self.running else {
            return;
        };
        if doc.changes.is_empty() {
            return;
        }
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: doc.uri,
                version: doc.version,
            },
            content_changes: doc.changes,
        };
        send_notification::<DidChangeTextDocument>(&running.conn, params);
    }
}

fn send_notification<N: Notification>(conn: &Connection, params: N::Params) {
    if let Err(err) = conn.notify::<N>(params) {
        // The connection is closing; the restart re-sends documents.
        tracing::debug!(method = N::METHOD, %err, "notification not sent");
    }
}

fn open_close(caps: &ServerCapabilities) -> bool {
    match &caps.text_document_sync {
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind != TextDocumentSyncKind::NONE,
        Some(TextDocumentSyncCapability::Options(options)) => options.open_close.unwrap_or(false),
        None => false,
    }
}

fn change_kind(caps: &ServerCapabilities) -> TextDocumentSyncKind {
    match &caps.text_document_sync {
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind,
        Some(TextDocumentSyncCapability::Options(options)) => {
            options.change.unwrap_or(TextDocumentSyncKind::NONE)
        }
        None => TextDocumentSyncKind::NONE,
    }
}

fn save_notifications(caps: &ServerCapabilities) -> bool {
    match &caps.text_document_sync {
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind != TextDocumentSyncKind::NONE,
        Some(TextDocumentSyncCapability::Options(options)) => match &options.save {
            Some(TextDocumentSyncSaveOptions::Supported(supported)) => *supported,
            Some(TextDocumentSyncSaveOptions::SaveOptions(_)) => true,
            None => false,
        },
        None => false,
    }
}

impl Shared {
    fn set_state(&self, state: ServerState) {
        {
            let mut status = lock(&self.status);
            if status.state == state {
                return;
            }
            status.state = state.clone();
        }
        tracing::info!(server = %self.config.name, ?state, "language server state");
        self.hooks.state_changed(&state);
    }

    /// The state while running, from the open progress.
    fn running_state(status: &StatusState) -> ServerState {
        match status.progress.values().last() {
            Some(title) => ServerState::Busy(title.clone()),
            None => ServerState::Ready,
        }
    }

    fn on_progress(&self, params: &Value) {
        let token = params
            .get("token")
            .map(Value::to_string)
            .unwrap_or_default();
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        let next = {
            let mut status = lock(&self.status);
            match value.get("kind").and_then(Value::as_str) {
                Some("begin") => {
                    let title = value
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Working")
                        .to_owned();
                    status.progress.insert(token, title);
                }
                Some("end") => {
                    status.progress.remove(&token);
                }
                _ => {}
            }
            status.running.then(|| Self::running_state(&status))
        };
        if let Some(state) = next {
            self.set_state(state);
        }
    }
}

/// Answers server→client requests and forwards notifications.
struct Router {
    shared: Weak<Shared>,
}

impl Incoming for Router {
    fn notification(&self, method: &str, params: Value) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        if method == "$/progress" {
            shared.on_progress(&params);
        }
        if method == "window/logMessage" {
            let message = params.get("message").and_then(Value::as_str).unwrap_or("");
            tracing::debug!(server = %shared.config.name, "{message}");
        }
        shared.hooks.notification(method, &params);
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, ResponseError> {
        let Some(shared) = self.shared.upgrade() else {
            return Err(ResponseError::method_not_found(method));
        };
        if let Some(answer) = shared.hooks.request(method, &params) {
            return answer;
        }
        match method {
            // No client-side settings yet: every item is "not set".
            "workspace/configuration" => {
                let count = params
                    .get("items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Ok(Value::Array(vec![Value::Null; count]))
            }
            "client/registerCapability"
            | "client/unregisterCapability"
            | "window/workDoneProgress/create"
            | "window/showMessageRequest" => Ok(Value::Null),
            "workspace/workspaceFolders" => Ok(Value::Array(workspace_folders(&shared.config))),
            // Only accepted while the client runs a command that asked for it
            // (see ClientHooks::request).
            "workspace/applyEdit" => Ok(json!({
                "applied": false,
                "failureReason": "the editor did not expect a workspace edit now"
            })),
            _ => Err(ResponseError::method_not_found(method)),
        }
    }
}

fn workspace_folders(config: &ServerConfig) -> Vec<Value> {
    let mut seen = Vec::new();
    std::iter::once(&config.root)
        .chain(&config.workspace_folders)
        .filter(|path| {
            if seen.contains(path) {
                false
            } else {
                seen.push(*path);
                true
            }
        })
        .filter_map(|path| {
            let uri = path_to_uri(path)?;
            let name = path
                .file_name()
                .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
                .into_owned();
            Some(json!({ "uri": uri.as_str(), "name": name }))
        })
        .collect()
}

/// Client capabilities: what this client implements (grows with M3b).
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "window": { "workDoneProgress": true },
        "workspace": {
            "workspaceFolders": true,
            "configuration": true,
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": [],
                "failureHandling": "abort"
            },
            "executeCommand": { "dynamicRegistration": false },
            "didChangeWatchedFiles": { "dynamicRegistration": true }
        },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "willSave": false,
                "willSaveWaitUntil": false,
                "didSave": true
            },
            "publishDiagnostics": {
                "relatedInformation": false,
                "versionSupport": true,
                "tagSupport": { "valueSet": [1, 2] }
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "completion": {
                "contextSupport": true,
                "completionItem": {
                    "snippetSupport": true,
                    "deprecatedSupport": true,
                    "tagSupport": { "valueSet": [1] },
                    "labelDetailsSupport": true,
                    "insertReplaceSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                    "resolveSupport": {
                        "properties": ["documentation", "detail", "additionalTextEdits"]
                    }
                }
            },
            "signatureHelp": {
                "contextSupport": true,
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                    "activeParameterSupport": true
                }
            },
            "rename": { "prepareSupport": true },
            "codeAction": {
                "isPreferredSupport": true,
                "disabledSupport": true,
                "dataSupport": true,
                "resolveSupport": { "properties": ["edit"] },
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": ["quickfix", "refactor", "refactor.extract",
                            "refactor.inline", "refactor.rewrite", "source",
                            "source.organizeImports"]
                    }
                }
            },
            "formatting": {}
        }
    })
}

fn initialize_params(config: &ServerConfig) -> Value {
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "Ferrule", "version": env!("CARGO_PKG_VERSION") },
        "locale": "en",
        // Deprecated in LSP 3.17 but still read by servers such as
        // typescript-language-server to locate the project.
        "rootUri": path_to_uri(&config.root).map(|uri| uri.as_str().to_owned()),
        "workspaceFolders": workspace_folders(config),
        "initializationOptions": config.initialization_options,
        "capabilities": client_capabilities(),
        "trace": "off"
    })
}

struct Started {
    conn: Connection,
    child: Child,
    /// The server and the processes it started (TypeScript's `tsserver`,
    /// or Node behind a `.cmd` wrapper on Windows). `None` if it could not
    /// be set up; then only the server itself is stopped.
    tree: Option<ProcessTree>,
    result: InitializeResult,
}

/// How long to wait for what is left of a stopped server's processes.
const TREE_TIMEOUT: Duration = Duration::from_secs(3);

/// Ends what is left of a stopped server's process tree and waits until it
/// is gone: a process still running in the project folder keeps it in use
/// (Windows cannot delete or rename it) and is an orphan.
async fn end_tree(name: &str, tree: Option<&ProcessTree>) {
    let Some(tree) = tree else {
        return;
    };
    if !tree.is_alive() {
        return;
    }
    tree.kill();
    let deadline = Instant::now() + TREE_TIMEOUT;
    while tree.is_alive() {
        if Instant::now() >= deadline {
            tracing::warn!(server = %name, "processes of the stopped server are still running");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn spawn_and_initialize(shared: &Arc<Shared>) -> Result<Started, LspError> {
    let config = &shared.config;
    let mut command = Command::new(&config.command.program);
    command
        .args(&config.command.args)
        .envs(config.command.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&config.command.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Its own process group, so it and everything it starts end together.
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console window for the server.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|err| {
        LspError::Spawn(format!(
            "{}: {err}",
            config.command.program.to_string_lossy()
        ))
    })?;
    #[cfg(unix)]
    let tree = child.id().and_then(ProcessTree::group);
    #[cfg(windows)]
    let tree = child.raw_handle().and_then(ProcessTree::job);
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(LspError::Spawn(
            "the server's stdio was not captured".into(),
        ));
    };
    let name = config.name.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::info!(target: "lsp_stderr", server = %name, "{line}");
        }
    });
    let router: Arc<dyn Incoming> = Arc::new(Router {
        shared: Arc::downgrade(shared),
    });
    let conn = Connection::new(&config.name, stdout, stdin, router);
    let pending = conn.start_raw("initialize", initialize_params(config))?;
    let reply = tokio::time::timeout(config.startup_timeout, pending.wait(&shared.stop))
        .await
        .map_err(|_| LspError::Timeout)??;
    let result: InitializeResult =
        serde_json::from_value(reply).map_err(|e| LspError::Protocol(e.to_string()))?;
    if let Some(encoding) = &result.capabilities.position_encoding
        && *encoding != PositionEncodingKind::UTF16
    {
        return Err(LspError::Protocol(format!(
            "the server chose position encoding {}, but only utf-16 is supported",
            encoding.as_str()
        )));
    }
    conn.notify::<Initialized>(InitializedParams {})?;
    Ok(Started {
        conn,
        child,
        tree,
        result,
    })
}

async fn shutdown(conn: &Connection, child: &mut Child) {
    let never = CancellationToken::new();
    let _ =
        tokio::time::timeout(Duration::from_secs(2), conn.request::<Shutdown>((), &never)).await;
    let _ = conn.notify::<Exit>(());
    if tokio::time::timeout(Duration::from_secs(1), child.wait())
        .await
        .is_err()
    {
        // Did not exit on its own.
        let _ = child.kill().await;
    }
}

async fn describe_exit(child: &mut Child) -> String {
    match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
        Ok(Ok(status)) => format!("the server exited ({status})"),
        Ok(Err(err)) => format!("the server exited ({err})"),
        Err(_) => {
            let _ = child.kill().await;
            "the server closed its output".into()
        }
    }
}

async fn run(shared: Arc<Shared>) {
    let policy = shared.config.restart.clone();
    let name = shared.config.name.clone();
    let mut failures = 0_u32;
    let mut last_error: String;
    loop {
        if shared.stop.is_cancelled() {
            break;
        }
        shared.set_state(ServerState::Starting);
        match spawn_and_initialize(&shared).await {
            Ok(mut started) => {
                let up_since = Instant::now();
                let generation = {
                    let mut sync = lock(&shared.sync);
                    sync.next_generation += 1;
                    sync.pending.clear();
                    sync.last_edit = None;
                    sync.running = Some(Running {
                        generation: sync.next_generation,
                        conn: started.conn.clone(),
                        capabilities: Arc::new(started.result.capabilities.clone()),
                    });
                    sync.next_generation
                };
                lock(&shared.status).server =
                    started
                        .result
                        .server_info
                        .as_ref()
                        .map(|info| match &info.version {
                            Some(version) => format!("{} {version}", info.name),
                            None => info.name.clone(),
                        });
                tracing::info!(server = %name, generation, "language server initialized");
                shared.hooks.started(generation);
                let state = {
                    let mut status = lock(&shared.status);
                    status.running = true;
                    Shared::running_state(&status)
                };
                shared.set_state(state);

                let stopped = tokio::select! {
                    () = started.conn.closed() => false,
                    () = shared.stop.cancelled() => true,
                };
                {
                    let mut sync = lock(&shared.sync);
                    sync.running = None;
                    sync.pending.clear();
                    sync.last_edit = None;
                }
                {
                    let mut status = lock(&shared.status);
                    status.running = false;
                    status.progress.clear();
                }
                if stopped {
                    shutdown(&started.conn, &mut started.child).await;
                    end_tree(&name, started.tree.as_ref()).await;
                    break;
                }
                last_error = describe_exit(&mut started.child).await;
                end_tree(&name, started.tree.as_ref()).await;
                tracing::warn!(server = %name, "{last_error}");
                if up_since.elapsed() >= policy.stable_after {
                    failures = 0;
                }
            }
            Err(_) if shared.stop.is_cancelled() => break,
            Err(err) => {
                tracing::warn!(server = %name, %err, "starting the language server failed");
                last_error = err.to_string();
            }
        }
        failures += 1;
        if failures > policy.max_attempts {
            shared.set_state(ServerState::Failed(format!(
                "{name} stopped after {failures} failed starts: {last_error}"
            )));
            shared.stop.cancelled().await;
            shared.done.cancel();
            return;
        }
        let delay = policy
            .base_delay
            .saturating_mul(1_u32 << (failures - 1).min(16));
        shared.set_state(ServerState::Restarting {
            attempt: failures,
            delay,
        });
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = shared.stop.cancelled() => break,
        }
    }
    shared.set_state(ServerState::Stopped);
    shared.done.cancel();
}

/// Sends queued changes once no edit arrived for the debounce delay.
async fn flusher(shared: Arc<Shared>) {
    loop {
        tokio::select! {
            () = shared.flush.notified() => {}
            () = shared.stop.cancelled() => return,
        }
        loop {
            let wait = {
                let sync = lock(&shared.sync);
                match sync.last_edit {
                    None => break,
                    Some(at) => {
                        (at + shared.config.debounce).saturating_duration_since(Instant::now())
                    }
                }
            };
            if wait.is_zero() {
                lock(&shared.sync).flush_all();
                break;
            }
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = shared.stop.cancelled() => return,
            }
        }
    }
}
