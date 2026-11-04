//! The TypeScript language server of a project (`docs/ARCHITECTURE.md`,
//! 4.7): finding the server, starting it lazily, mirroring documents from
//! the Rust buffer, storing diagnostics and reporting status.
//!
//! Lock order: a document's lock may be held while calling into this service
//! (and the supervisor); this service never takes a document lock while
//! holding its own locks.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use ferrule_buffer::LspChange;
use ferrule_ipc::{self as ipc, DocId, IpcError, IpcErrorKind};
use ferrule_lang_js::LanguageServerDef;
use ferrule_lsp_client::lsp_types::notification::DidChangeWatchedFiles;
use ferrule_lsp_client::lsp_types::{
    self as lsp, DiagnosticSeverity, DiagnosticTag, DidChangeWatchedFilesParams, FileChangeType,
    FileEvent, NumberOrString, Position, PublishDiagnosticsParams, Range,
    TextDocumentContentChangeEvent,
};
use ferrule_lsp_client::{
    ClientHooks, LspError, ServerCommand, ServerConfig, ServerState, Supervisor, find_in_dir,
    find_on_path, is_executable, is_node_script, path_to_uri, uri_to_path,
};
use ferrule_project::{Project, Workspace};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::convert::to_u32;
use crate::events::{EventSink, SessionEvent};
use crate::session::{DocHandle, DocState, Session};

/// Identifies one server run: our supervisor instance and its generation.
pub(crate) type SyncKey = (u64, u64);

pub(crate) struct LanguageService {
    def: LanguageServerDef,
    project: Arc<Project>,
    sink: Arc<dyn EventSink>,
    settings_file: PathBuf,
    session: Weak<Session>,
    inner: Mutex<Inner>,
    /// Last published diagnostics per project-relative path.
    diagnostics: Mutex<BTreeMap<String, Vec<lsp::Diagnostic>>>,
    hover_tokens: Mutex<HashMap<DocId, CancellationToken>>,
    nav_token: Mutex<Option<CancellationToken>>,
}

struct Inner {
    supervisor: Option<(u64, Supervisor)>,
    next_instance: u64,
    status: ipc::ServerStatus,
    program: Option<PathBuf>,
    workspace: Option<Workspace>,
    ts_version: Option<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain data; a panic elsewhere does not leave it inconsistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Maps a language server error to an IPC error.
pub(crate) fn lsp_error(err: LspError) -> IpcError {
    match err {
        LspError::Cancelled => IpcError::new(IpcErrorKind::Cancelled, "cancelled"),
        LspError::NotRunning | LspError::Closed => IpcError::new(
            IpcErrorKind::Conflict,
            "the TypeScript server is not running",
        ),
        LspError::Spawn(message) => IpcError::new(IpcErrorKind::Io, message),
        other => IpcError::new(IpcErrorKind::Internal, other.to_string()),
    }
}

/// The LSP version of a document version.
pub(crate) fn lsp_version(version: u32) -> i32 {
    i32::try_from(version).unwrap_or(i32::MAX)
}

fn to_lsp_change(change: &LspChange) -> TextDocumentContentChangeEvent {
    let pos = |p: ferrule_buffer::LinePos| Position::new(to_u32(p.line), to_u32(p.character));
    TextDocumentContentChangeEvent {
        range: Some(Range::new(pos(change.start), pos(change.end))),
        range_length: None,
        text: change.text.clone(),
    }
}

impl LanguageService {
    pub fn new(
        def: LanguageServerDef,
        project: Arc<Project>,
        sink: Arc<dyn EventSink>,
        settings_file: PathBuf,
        session: Weak<Session>,
    ) -> Self {
        Self {
            def,
            project,
            sink,
            settings_file,
            session,
            inner: Mutex::new(Inner {
                supervisor: None,
                next_instance: 0,
                status: ipc::ServerStatus {
                    name: def.name.to_owned(),
                    state: ipc::ServerStateKind::NotStarted,
                    message: String::new(),
                    detail: String::new(),
                },
                program: None,
                workspace: None,
                ts_version: None,
            }),
            diagnostics: Mutex::new(BTreeMap::new()),
            hover_tokens: Mutex::new(HashMap::new()),
            nav_token: Mutex::new(None),
        }
    }

    /// The LSP language id for a file this server handles.
    pub fn language_id(&self, path: &Path) -> Option<&'static str> {
        self.def.language_id(path.extension()?.to_str()?)
    }

    pub fn status(&self) -> ipc::ServerStatus {
        lock(&self.inner).status.clone()
    }

    fn supervisor(&self) -> Option<(u64, Supervisor)> {
        lock(&self.inner).supervisor.clone()
    }

    fn set_status(
        &self,
        inner: &mut Inner,
        state: ipc::ServerStateKind,
        message: String,
        extra: &str,
    ) -> ipc::ServerStatus {
        inner.status = ipc::ServerStatus {
            name: self.def.name.to_owned(),
            state,
            message,
            detail: self.detail(inner, extra),
        };
        inner.status.clone()
    }

    fn detail(&self, inner: &Inner, extra: &str) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !extra.is_empty() {
            lines.push(extra.to_owned());
        }
        if let Some(program) = &inner.program {
            let info = inner
                .supervisor
                .as_ref()
                .and_then(|(_, s)| s.server_info())
                .map(|i| format!(" ({i})"))
                .unwrap_or_default();
            lines.push(format!("Server: {}{info}", program.display()));
        }
        if let Some(version) = &inner.ts_version {
            lines.push(format!("TypeScript {version}"));
        }
        if let Some(ws) = &inner.workspace {
            let list = |items: &[String]| {
                let mut text = items.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
                if items.len() > 5 {
                    text.push_str(&format!(", … ({} in total)", items.len()));
                }
                text
            };
            if ws.config_files.is_empty() {
                lines.push("No tsconfig.json or jsconfig.json found".into());
            } else {
                lines.push(format!("Config files: {}", list(&ws.config_files)));
            }
            let manager = ws
                .package_manager
                .map(|m| format!(" ({})", m.name()))
                .unwrap_or_default();
            if ws.packages.is_empty() {
                lines.push(format!("No workspace packages{manager}"));
            } else {
                lines.push(format!(
                    "Workspace packages{manager}: {}",
                    list(&ws.packages)
                ));
            }
        }
        lines.join("\n")
    }

    /// Finds the server: project `node_modules/.bin`, then the user setting,
    /// then `PATH`. A Node script also needs `node` on `PATH`.
    fn locate(&self) -> Result<PathBuf, String> {
        let program = self.def.program;
        let local_bin = self.project.root().join("node_modules").join(".bin");
        let found = match find_in_dir(&local_bin, program) {
            Some(path) => path,
            None => {
                let settings = ferrule_settings::load_user_settings(&self.settings_file)?;
                match settings.typescript_server_path {
                    Some(path) if is_executable(&path) => path,
                    Some(path) => {
                        return Err(format!(
                            "{} is set to {}, which is not an executable file",
                            self.def.setting,
                            path.display()
                        ));
                    }
                    None => find_on_path(program).ok_or_else(|| {
                        format!(
                            "{program} was not found in the project's node_modules/.bin or on PATH; {}",
                            self.def.install_hint
                        )
                    })?,
                }
            }
        };
        if is_node_script(&found) && find_on_path("node").is_none() {
            return Err(format!(
                "Node.js was not found on PATH; it is needed to run {}. Install Node.js 22 or newer",
                found.display()
            ));
        }
        Ok(found)
    }

    /// Starts the server if it is not running yet. Blocking (file system
    /// checks, workspace detection): call from a blocking task inside the
    /// runtime.
    pub fn ensure_started(self: &Arc<Self>) {
        let status = {
            let mut inner = lock(&self.inner);
            if inner.supervisor.is_some() {
                return;
            }
            match self.locate() {
                Err(problem) => {
                    let message = format!("{}: not available", self.def.name);
                    let status = self.set_status(
                        &mut inner,
                        ipc::ServerStateKind::NotFound,
                        message,
                        &problem,
                    );
                    tracing::warn!(server = self.def.name, "{problem}");
                    status
                }
                Ok(program) => {
                    let workspace = self.project.detect_workspace();
                    let root = self.project.root().to_path_buf();
                    let mut config = ServerConfig::new(
                        self.def.name,
                        ServerCommand {
                            program: program.clone(),
                            args: self.def.args.iter().map(|a| (*a).to_owned()).collect(),
                            cwd: root.clone(),
                            env: Vec::new(),
                        },
                        root.clone(),
                    );
                    config.workspace_folders =
                        workspace.packages.iter().map(|p| root.join(p)).collect();
                    inner.next_instance += 1;
                    let instance = inner.next_instance;
                    let hooks = Arc::new(Hooks {
                        service: Arc::downgrade(self),
                        instance,
                    });
                    tracing::info!(server = self.def.name, program = ?program, "starting language server");
                    inner.program = Some(program);
                    inner.workspace = Some(workspace);
                    inner.ts_version = None;
                    inner.supervisor = Some((instance, Supervisor::start(config, hooks)));
                    let message = format!("{}: starting", self.def.name);
                    self.set_status(&mut inner, ipc::ServerStateKind::Starting, message, "")
                }
            }
        };
        self.sink.emit(SessionEvent::ServerStatus(status));
    }

    /// Mirrors a document to the server: `didOpen` (full text) if the running
    /// server has not seen it, else the queued changes. Call with the
    /// document lock held, right after applying `changes`.
    pub fn sync(&self, doc: &DocHandle, state: &mut DocState, changes: &[LspChange]) {
        let (Some(language), Some(uri)) = (doc.lsp_language, &doc.uri) else {
            return;
        };
        let Some((instance, supervisor)) = self.supervisor() else {
            return;
        };
        let Some(generation) = supervisor.generation() else {
            // Not running: `started` opens every document later.
            return;
        };
        let key = (instance, generation);
        let version = lsp_version(state.doc.version());
        if state.lsp_key == Some(key) {
            if !changes.is_empty() {
                let changes = changes.iter().map(to_lsp_change).collect();
                supervisor.did_change(generation, uri, version, changes, || state.doc.text());
            }
        } else if supervisor.did_open(generation, uri.clone(), language, version, state.doc.text())
        {
            state.lsp_key = Some(key);
        }
    }

    /// Makes sure the server has the document's current text and returns the
    /// supervisor to send a request to, or `None` if it is not running.
    pub fn prepare_request(&self, doc: &DocHandle, state: &mut DocState) -> Option<Supervisor> {
        self.sync(doc, state, &[]);
        let (instance, supervisor) = self.supervisor()?;
        let generation = supervisor.generation()?;
        if state.lsp_key != Some((instance, generation)) {
            return None;
        }
        if let Some(uri) = &doc.uri {
            supervisor.flush(uri);
        }
        Some(supervisor)
    }

    /// `didSave` after the document was written.
    pub fn saved(&self, doc: &DocHandle, state: &DocState) {
        if let (Some(uri), Some((instance, supervisor))) = (&doc.uri, self.supervisor())
            && let Some(generation) = supervisor.generation()
            && state.lsp_key == Some((instance, generation))
        {
            supervisor.did_save(generation, uri);
        }
    }

    /// `didClose` when the document was closed.
    pub fn closed(&self, doc: &DocHandle, state: &mut DocState) {
        lock(&self.hover_tokens).remove(&doc.id);
        if let (Some(uri), Some((instance, supervisor))) = (&doc.uri, self.supervisor())
            && let Some(generation) = supervisor.generation()
            && state.lsp_key == Some((instance, generation))
        {
            supervisor.did_close(generation, uri);
        }
        state.lsp_key = None;
    }

    /// A fresh token for a hover in `doc`, cancelling the previous one.
    pub fn hover_token(&self, doc: DocId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(previous) = lock(&self.hover_tokens).insert(doc, token.clone()) {
            previous.cancel();
        }
        token
    }

    /// A fresh token for a navigation, cancelling the previous one.
    pub fn nav_token(&self) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(previous) = lock(&self.nav_token).replace(token.clone()) {
            previous.cancel();
        }
        token
    }

    /// Diagnostics last published for a project-relative path.
    pub fn diagnostics_for(&self, path: &str) -> Vec<lsp::Diagnostic> {
        lock(&self.diagnostics)
            .get(path)
            .cloned()
            .unwrap_or_default()
    }

    /// Problems of every file.
    pub fn problems(&self) -> Vec<ipc::FileProblems> {
        lock(&self.diagnostics)
            .iter()
            .map(|(path, diagnostics)| file_problems(path, diagnostics))
            .collect()
    }

    /// Forwards files changed on disk (`workspace/didChangeWatchedFiles`).
    pub fn files_changed(&self, paths: &[(PathBuf, bool)]) {
        let Some((_, supervisor)) = self.supervisor() else {
            return;
        };
        let changes: Vec<FileEvent> = paths
            .iter()
            .filter(|(path, _)| {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                self.def.language_id(ext).is_some() || ext.eq_ignore_ascii_case("json")
            })
            .filter_map(|(path, exists)| {
                Some(FileEvent {
                    uri: path_to_uri(path)?,
                    typ: if *exists {
                        FileChangeType::CHANGED
                    } else {
                        FileChangeType::DELETED
                    },
                })
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        if let Err(err) =
            supervisor.notify::<DidChangeWatchedFiles>(DidChangeWatchedFilesParams { changes })
        {
            tracing::debug!(%err, "watched-file changes not sent");
        }
    }

    /// Stops the server; resolves when it is gone.
    pub fn stop(&self) -> impl Future<Output = ()> + Send + 'static {
        let supervisor = lock(&self.inner).supervisor.take();
        async move {
            if let Some((_, supervisor)) = supervisor {
                supervisor.stop().await;
            }
        }
    }

    /// Stops the server, clears its diagnostics and starts it again.
    pub async fn restart(self: &Arc<Self>) {
        self.stop().await;
        let cleared: Vec<String> = std::mem::take(&mut *lock(&self.diagnostics))
            .into_keys()
            .collect();
        for path in cleared {
            self.sink
                .emit(SessionEvent::ProblemsChanged(ipc::FileProblems {
                    path,
                    problems: Vec::new(),
                }));
        }
        let service = self.clone();
        if let Err(err) = tokio::task::spawn_blocking(move || service.ensure_started()).await {
            tracing::error!(%err, "restarting the language server failed");
        }
    }

    fn on_state(&self, instance: u64, state: &ServerState) {
        let name = self.def.name;
        let status = {
            let mut inner = lock(&self.inner);
            if inner.supervisor.as_ref().map(|(i, _)| *i) != Some(instance) {
                // A stopped instance reporting late.
                return;
            }
            let (kind, message, extra) = match state {
                ServerState::Starting => (
                    ipc::ServerStateKind::Starting,
                    format!("{name}: starting"),
                    String::new(),
                ),
                ServerState::Busy(title) => (
                    ipc::ServerStateKind::Indexing,
                    format!("{name}: indexing"),
                    title.clone(),
                ),
                ServerState::Ready => (
                    ipc::ServerStateKind::Ready,
                    format!("{name}: ready"),
                    String::new(),
                ),
                ServerState::Restarting { attempt, delay } => (
                    ipc::ServerStateKind::Restarting,
                    format!("{name}: restarting"),
                    format!(
                        "The server stopped; restart attempt {attempt} in {:.1} s",
                        delay.as_secs_f32()
                    ),
                ),
                ServerState::Failed(problem) => (
                    ipc::ServerStateKind::Error,
                    format!("{name}: error"),
                    format!("{problem}. Run \"Restart TypeScript Server\" to try again."),
                ),
                ServerState::Stopped => (
                    ipc::ServerStateKind::Stopped,
                    format!("{name}: stopped"),
                    String::new(),
                ),
            };
            self.set_status(&mut inner, kind, message, &extra)
        };
        self.sink.emit(SessionEvent::ServerStatus(status));
    }

    fn on_diagnostics(&self, params: &Value) {
        let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone()) else {
            tracing::warn!("ignoring malformed publishDiagnostics");
            return;
        };
        let Some(path) = uri_to_path(&params.uri).and_then(|p| self.project.relativize(&p)) else {
            return;
        };
        {
            let mut store = lock(&self.diagnostics);
            if params.diagnostics.is_empty() {
                store.remove(&path);
            } else {
                store.insert(path.clone(), params.diagnostics.clone());
            }
        }
        self.sink.emit(SessionEvent::ProblemsChanged(file_problems(
            &path,
            &params.diagnostics,
        )));
    }

    fn on_typescript_version(&self, params: &Value) {
        let Some(version) = params.get("version").and_then(Value::as_str) else {
            return;
        };
        let source = params
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status = {
            let mut inner = lock(&self.inner);
            inner.ts_version = Some(format!("{version} ({source})"));
            let previous = inner.status.clone();
            let extra = previous
                .detail
                .lines()
                .next()
                .filter(|line| !line.starts_with("Server:"))
                .unwrap_or("")
                .to_owned();
            self.set_status(&mut inner, previous.state, previous.message, &extra)
        };
        self.sink.emit(SessionEvent::ServerStatus(status));
    }

    /// Sends every open document of this project to a newly started server.
    fn replay_documents(self: &Arc<Self>) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            for doc in session.open_docs() {
                if !session.is_current_service(&service) {
                    return;
                }
                if doc.lsp_language.is_none() {
                    continue;
                }
                match doc.lock() {
                    Ok(mut state) => service.sync(&doc, &mut state, &[]),
                    Err(err) => tracing::error!(%err, "cannot reopen a document on the server"),
                }
            }
        });
    }
}

fn severity(severity: Option<DiagnosticSeverity>) -> ipc::Severity {
    match severity {
        Some(DiagnosticSeverity::WARNING) => ipc::Severity::Warning,
        Some(DiagnosticSeverity::INFORMATION) => ipc::Severity::Information,
        Some(DiagnosticSeverity::HINT) => ipc::Severity::Hint,
        // Errors, and diagnostics without severity (the client decides).
        _ => ipc::Severity::Error,
    }
}

fn code(code: Option<&NumberOrString>) -> Option<String> {
    code.map(|c| match c {
        NumberOrString::Number(n) => n.to_string(),
        NumberOrString::String(s) => s.clone(),
    })
}

fn file_problems(path: &str, diagnostics: &[lsp::Diagnostic]) -> ipc::FileProblems {
    ipc::FileProblems {
        path: path.to_owned(),
        problems: diagnostics
            .iter()
            .map(|d| ipc::Problem {
                severity: severity(d.severity),
                message: d.message.clone(),
                source: d.source.clone(),
                code: code(d.code.as_ref()),
                line: d.range.start.line,
                column: d.range.start.character,
                end_line: d.range.end.line,
                end_column: d.range.end.character,
            })
            .collect(),
    }
}

/// Converts stored diagnostics to offsets in the current text.
pub(crate) fn diagnostic_spans(
    doc: &ferrule_buffer::Document,
    diagnostics: &[lsp::Diagnostic],
) -> Vec<ipc::DiagnosticSpan> {
    let offset = |p: Position| {
        to_u32(doc.offset_of(ferrule_buffer::LinePos::new(
            p.line as usize,
            p.character as usize,
        )))
    };
    let mut spans: Vec<ipc::DiagnosticSpan> = diagnostics
        .iter()
        .map(|d| {
            let tags = d.tags.as_deref().unwrap_or(&[]);
            ipc::DiagnosticSpan {
                from: offset(d.range.start),
                to: offset(d.range.end),
                severity: severity(d.severity),
                message: d.message.clone(),
                source: d.source.clone(),
                code: code(d.code.as_ref()),
                unnecessary: tags.contains(&DiagnosticTag::UNNECESSARY),
                deprecated: tags.contains(&DiagnosticTag::DEPRECATED),
            }
        })
        .collect();
    spans.sort_by_key(|s| (s.from, s.to));
    spans
}

struct Hooks {
    service: Weak<LanguageService>,
    instance: u64,
}

impl ClientHooks for Hooks {
    fn state_changed(&self, state: &ServerState) {
        if let Some(service) = self.service.upgrade() {
            service.on_state(self.instance, state);
        }
    }

    fn started(&self, _generation: u64) {
        if let Some(service) = self.service.upgrade() {
            service.replay_documents();
        }
    }

    fn notification(&self, method: &str, params: &Value) {
        let Some(service) = self.service.upgrade() else {
            return;
        };
        match method {
            "textDocument/publishDiagnostics" => service.on_diagnostics(params),
            // typescript-language-server: which TypeScript it loaded.
            "$/typescriptVersion" => service.on_typescript_version(params),
            "window/showMessage" => {
                let message = params.get("message").and_then(Value::as_str).unwrap_or("");
                tracing::info!(server = service.def.name, "{message}");
            }
            _ => {}
        }
    }
}
