//! Debug sessions of the open project (`docs/ARCHITECTURE.md`, 4.10.4 and
//! 4.10.7): the [`DebugBackend`] a session talks to, the driver task that
//! turns its events into `debug-event`s, and the commands over IPC.
//!
//! A debug run is an M5a run (a PTY, trust, Stop, the process-tree cleanup)
//! started with `--inspect-brk=127.0.0.1:<port>`; a [`DebugConnector`] then
//! attaches to that port. The connector is injected, so tests and the
//! headless binary use a scripted one and the app a real one.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrule_ipc::{self as ipc, IpcError, IpcErrorKind};
use ferrule_run_debug::{self as rd, DebugCommand, DebugError, DebugState, RunConfig, RunKind};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::{EventSink, SessionEvent};
use crate::runs::Runs;
use crate::session::{Session, lock};

/// How long attaching may take before the session and its run are ended: a
/// program started with `--inspect-brk` waits for a debugger forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long closing the debugger's connection may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// How long Stop waits for a session's task to end.
const TASK_TIMEOUT: Duration = Duration::from_secs(3);

/// A boxed future, so the backend traits can be used as `dyn`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a session sends to the debugger.
pub trait DebugBackend: Send + Sync {
    /// Resume, a step or pause. Stop is not sent: the session ends by
    /// cancelling its task, which calls [`DebugBackend::close`].
    fn command(&self, command: DebugCommand) -> BoxFuture<'_, Result<(), DebugError>>;

    /// Detaches from the debuggee (best effort) and closes the connection.
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// A live connection to a debuggee.
pub struct Connection {
    /// For commands.
    pub backend: Arc<dyn DebugBackend>,
    /// What the debuggee does, in order. A closed channel means the
    /// connection ended.
    pub events: mpsc::Receiver<rd::DebugEvent>,
}

/// Attaches to a debuggee that waits on `127.0.0.1:<port>`.
pub trait DebugConnector: Send + Sync {
    /// Connects, or fails. Must give up when `cancel` fires.
    fn connect(
        &self,
        port: u16,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<Connection, DebugError>>;
}

/// The backend, once the task has attached.
type Slot = Arc<Mutex<Option<Arc<dyn DebugBackend>>>>;

/// What a command needs of an attached session: the backend and the state.
type Attached = (Arc<dyn DebugBackend>, Arc<Mutex<DebugState>>);

/// A session in progress.
struct Live {
    run: u32,
    state: Arc<Mutex<DebugState>>,
    backend: Slot,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

/// The project's debug sessions.
pub(crate) struct Debugs {
    live: Mutex<HashMap<u32, Live>>,
    next: AtomicU32,
    sink: Arc<dyn EventSink>,
    connector: Option<Arc<dyn DebugConnector>>,
}

impl Debugs {
    pub fn new(sink: Arc<dyn EventSink>, connector: Option<Arc<dyn DebugConnector>>) -> Arc<Self> {
        Arc::new(Self {
            live: Mutex::new(HashMap::new()),
            next: AtomicU32::new(1),
            sink,
            connector,
        })
    }

    /// Ends every session and waits for its task. Called before the runs
    /// are closed, so `Ended` events go out while the runtime is alive.
    pub async fn shut_down(&self) {
        let ids: Vec<u32> = match self.live.lock() {
            Ok(live) => live.keys().copied().collect(),
            Err(_) => return,
        };
        for id in ids {
            self.end(id).await;
        }
    }

    /// Cancels a session and waits until its task has sent `Ended` and
    /// closed the connection. Returns the run it debugged.
    async fn end(&self, id: u32) -> Option<u32> {
        let (run, cancel, task) = {
            let mut live = self.live.lock().ok()?;
            let entry = live.get_mut(&id)?;
            (entry.run, entry.cancel.clone(), entry.task.take())
        };
        cancel.cancel();
        if let Some(mut task) = task
            && tokio::time::timeout(TASK_TIMEOUT, &mut task).await.is_err()
        {
            tracing::warn!(session = id, "debug task did not end in time; aborting it");
            task.abort();
        }
        if let Ok(mut live) = self.live.lock() {
            live.remove(&id);
        }
        Some(run)
    }

    /// Sessions that have not been ended (the cleanup tests look at this).
    #[cfg(any(test, feature = "testing"))]
    pub fn count(&self) -> usize {
        self.live.lock().map(|l| l.len()).unwrap_or(0)
    }

    fn get(&self, id: u32) -> Result<Attached, IpcError> {
        let live = lock(&self.live)?;
        let entry = live.get(&id).ok_or_else(|| {
            IpcError::new(
                IpcErrorKind::NotFound,
                format!("debug session {id} is not running"),
            )
        })?;
        let backend = lock(&entry.backend)?.clone().ok_or_else(|| {
            IpcError::new(
                IpcErrorKind::Conflict,
                "the debugger is not attached yet".to_owned(),
            )
        })?;
        Ok((backend, entry.state.clone()))
    }
}

/// The IPC event for a session event, or `None` for the ones this slice
/// does not send (program output and breakpoint state come with the
/// breakpoint and console slices).
fn to_ipc(event: &rd::DebugEvent) -> Option<ipc::DebugEventKind> {
    Some(match event {
        rd::DebugEvent::Started => ipc::DebugEventKind::Started,
        rd::DebugEvent::Resumed => ipc::DebugEventKind::Resumed,
        rd::DebugEvent::Ended { exit_code, stopped } => ipc::DebugEventKind::Ended {
            exit_code: *exit_code,
            stopped: *stopped,
            message: None,
        },
        rd::DebugEvent::Paused(info) => ipc::DebugEventKind::Paused {
            reason: match &info.reason {
                rd::PauseReason::Breakpoint => ipc::DebugPauseReason::Breakpoint,
                rd::PauseReason::Step => ipc::DebugPauseReason::Step,
                rd::PauseReason::Pause => ipc::DebugPauseReason::Pause,
                rd::PauseReason::Exception { message } => ipc::DebugPauseReason::Exception {
                    message: message.clone(),
                },
                rd::PauseReason::Entry => ipc::DebugPauseReason::Entry,
            },
            frames: info
                .frames
                .iter()
                .map(|f| ipc::DebugFrame {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    location: ipc::DebugLocation {
                        path_or_script: f.location.path_or_script.clone(),
                        line: f.location.line,
                        column: f.location.column,
                        generated: f.location.generated.map(|g| ipc::DebugPosition {
                            line: g.line,
                            column: g.column,
                        }),
                    },
                    is_library: f.is_library,
                })
                .collect(),
            selected: u32::try_from(info.selected_frame).unwrap_or(0),
        },
        rd::DebugEvent::BreakpointState { .. } | rd::DebugEvent::Output { .. } => return None,
    })
}

fn to_ipc_error(err: &DebugError) -> IpcError {
    match err {
        DebugError::NotAllowed { .. } => IpcError::new(IpcErrorKind::Conflict, err.to_string()),
        DebugError::Backend(_) => IpcError::new(IpcErrorKind::Io, err.to_string()),
    }
}

/// A debug run must not open a second inspector or move the port
/// (`docs/ARCHITECTURE.md`, 4.10.7), and only a plain Node file is debugged
/// in this slice.
///
/// # Errors
/// [`IpcErrorKind::InvalidArgument`] saying what to change.
pub(crate) fn check_debuggable(config: &RunConfig) -> Result<(), IpcError> {
    let refuse = |message: String| Err(IpcError::new(IpcErrorKind::InvalidArgument, message));
    if !matches!(config.kind, RunKind::Node { .. }) {
        return refuse(
            "only a Node file can be debugged for now (npm scripts and tests come later)".into(),
        );
    }
    if let Some(arg) = config.args.iter().find(|a| a.starts_with("--inspect")) {
        return refuse(format!(
            "the argument {arg} opens an inspector; remove it, Ferrule starts the debugger itself"
        ));
    }
    let options = config
        .env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("NODE_OPTIONS"))
        .map(|(_, v)| v.as_str());
    if options.is_some_and(|v| v.contains("--inspect")) {
        return refuse(
            "NODE_OPTIONS contains --inspect; remove it, Ferrule starts the debugger itself".into(),
        );
    }
    Ok(())
}

/// A free loopback port. The window between closing it and Node binding it
/// is tiny; if it is lost, attaching fails and the session ends.
pub(crate) fn free_port() -> Result<u16, IpcError> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| IpcError::new(IpcErrorKind::Io, format!("no free local port: {e}")))
}

/// Everything a session's task needs.
struct Drive {
    id: u32,
    connector: Arc<dyn DebugConnector>,
    port: u16,
    cancel: CancellationToken,
    state: Arc<Mutex<DebugState>>,
    slot: Slot,
    sink: Arc<dyn EventSink>,
    runs: Arc<Runs>,
    run: u32,
}

/// What a session's task does: attaches, forwards events with a sequence
/// number, and on the way out always sends `Ended` once and closes the
/// connection.
async fn drive(d: Drive) {
    let mut seq = 0_u32;
    let mut emit = |kind: ipc::DebugEventKind| {
        seq = seq.saturating_add(1);
        d.sink.emit(SessionEvent::Debug(ipc::DebugEvent {
            session: d.id,
            seq,
            event: kind,
        }));
    };
    // `Err(None)`: the user stopped it while attaching; `Err(Some(text))`:
    // attaching failed, and the view is told why.
    let attached: Result<Connection, Option<String>> = tokio::select! {
        () = d.cancel.cancelled() => Err(None),
        r = tokio::time::timeout(CONNECT_TIMEOUT, d.connector.connect(d.port, d.cancel.clone())) => match r {
            Ok(Ok(connection)) => Ok(connection),
            Ok(Err(err)) => {
                tracing::warn!(session = d.id, %err, "attaching the debugger failed");
                Err(Some(format!("The debugger could not attach: {err}")))
            }
            Err(_) => {
                tracing::warn!(session = d.id, "attaching the debugger timed out");
                Err(Some(format!(
                    "The debugger did not attach within {} seconds",
                    CONNECT_TIMEOUT.as_secs()
                )))
            }
        },
    };
    let Connection {
        backend,
        mut events,
    } = match attached {
        Ok(connection) => connection,
        Err(message) => {
            // Never attached: the program waits for a debugger, so end it.
            if let Ok(terminal) = d.runs.terminals.get(d.run) {
                terminal.stop();
            }
            emit(ipc::DebugEventKind::Ended {
                exit_code: None,
                stopped: d.cancel.is_cancelled(),
                message,
            });
            return;
        }
    };
    if let Ok(mut slot) = d.slot.lock() {
        *slot = Some(backend.clone());
    }
    let mut ended = false;
    loop {
        tokio::select! {
            () = d.cancel.cancelled() => break,
            next = events.recv() => {
                let Some(event) = next else { break };
                if let Ok(mut state) = d.state.lock() {
                    *state = state.after(&event);
                }
                if let Some(kind) = to_ipc(&event) {
                    emit(kind);
                }
                if matches!(event, rd::DebugEvent::Ended { .. }) {
                    ended = true;
                    break;
                }
            }
        }
    }
    if !ended {
        emit(ipc::DebugEventKind::Ended {
            exit_code: None,
            stopped: d.cancel.is_cancelled(),
            message: None,
        });
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, backend.close()).await;
}

impl Session {
    /// Starts debugging a Node configuration: applies trust, starts the run
    /// with the inspector on a random loopback port and attaches. Documents
    /// are saved by the caller, as for a run.
    ///
    /// # Errors
    /// An inspector flag in the configuration, a configuration that cannot
    /// be debugged, no debugger in this build, or a run that cannot start.
    pub async fn start_debug(
        &self,
        config: ipc::RunConfig,
        size: ipc::TerminalSize,
    ) -> Result<ipc::StartDebug, IpcError> {
        let runtime = self.runtime()?;
        let Some(connector) = runtime.debug.connector.clone() else {
            return Err(IpcError::new(
                IpcErrorKind::Internal,
                "this build has no debugger",
            ));
        };
        let port = free_port()?;
        let run = match self.start_run_inspected(config, size, port).await? {
            ipc::StartRun::NeedsTrust { command, cwd } => {
                return Ok(ipc::StartDebug::NeedsTrust { command, cwd });
            }
            ipc::StartRun::Started { run } => run,
        };
        let debugs = runtime.debug.clone();
        let id = debugs.next.fetch_add(1, Ordering::Relaxed);
        let cancel = CancellationToken::new();
        let state = Arc::new(Mutex::new(DebugState::Starting));
        let slot: Slot = Arc::new(Mutex::new(None));
        // Registered before the task can end, so the end finds it.
        let mut live = lock(&debugs.live)?;
        let task = tokio::spawn(drive(Drive {
            id,
            connector,
            port,
            cancel: cancel.clone(),
            state: state.clone(),
            slot: slot.clone(),
            sink: debugs.sink.clone(),
            runs: runtime.runs.clone(),
            run: run.id,
        }));
        live.insert(
            id,
            Live {
                run: run.id,
                state,
                backend: slot,
                cancel,
                task: Some(task),
            },
        );
        drop(live);
        Ok(ipc::StartDebug::Started { session: id, run })
    }

    /// How many debug sessions exist (tests of the cleanup).
    ///
    /// # Errors
    /// No project is open.
    #[cfg(feature = "testing")]
    pub fn debug_session_count(&self) -> Result<usize, IpcError> {
        Ok(self.runtime()?.debug.count())
    }

    /// Sends one command to a session.
    async fn debug_command(&self, session: u32, command: DebugCommand) -> Result<(), IpcError> {
        let runtime = self.runtime()?;
        let (backend, state) = runtime.debug.get(session)?;
        lock(&state)?.check(command).map_err(|e| to_ipc_error(&e))?;
        backend.command(command).await.map_err(|e| to_ipc_error(&e))
    }

    /// Continues a paused program.
    ///
    /// # Errors
    /// Not paused, or the session is not running.
    pub async fn debug_resume(&self, session: u32) -> Result<(), IpcError> {
        self.debug_command(session, DebugCommand::Resume).await
    }

    /// Steps over the current line.
    ///
    /// # Errors
    /// Not paused, or the session is not running.
    pub async fn debug_step_over(&self, session: u32) -> Result<(), IpcError> {
        self.debug_command(session, DebugCommand::StepOver).await
    }

    /// Steps into the call on the current line.
    ///
    /// # Errors
    /// Not paused, or the session is not running.
    pub async fn debug_step_into(&self, session: u32) -> Result<(), IpcError> {
        self.debug_command(session, DebugCommand::StepInto).await
    }

    /// Steps out of the current function.
    ///
    /// # Errors
    /// Not paused, or the session is not running.
    pub async fn debug_step_out(&self, session: u32) -> Result<(), IpcError> {
        self.debug_command(session, DebugCommand::StepOut).await
    }

    /// Pauses a running program.
    ///
    /// # Errors
    /// Not running, or the session is not running.
    pub async fn debug_pause(&self, session: u32) -> Result<(), IpcError> {
        self.debug_command(session, DebugCommand::Pause).await
    }

    /// Ends a session and its process tree (the run tab and its output
    /// stay). A paused program is killed, never resumed first.
    ///
    /// # Errors
    /// The session is not running.
    pub async fn stop_debug(&self, session: u32) -> Result<(), IpcError> {
        let runtime = self.runtime()?;
        let Some(run) = runtime.debug.end(session).await else {
            return Err(IpcError::new(
                IpcErrorKind::NotFound,
                format!("debug session {session} is not running"),
            ));
        };
        self.stop_run(run).await
    }
}

pub use cdp_backend::CdpConnector;

/// The real debugger: Node's inspector over the Chrome DevTools Protocol
/// (`ferrule-cdp-client`).
mod cdp_backend {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ferrule_cdp_client::protocol::{self as cdp, Event};
    use ferrule_cdp_client::{CdpClient, CdpError, CdpEvent, CdpEvents, discover};
    use ferrule_run_debug::{self as rd, DebugCommand, DebugError};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{BoxFuture, Connection, DebugBackend, DebugConnector};

    /// How long discovering the inspector's URL may take.
    const DISCOVER_TIMEOUT: Duration = Duration::from_secs(10);
    /// How long `Debugger.disable` may take when closing.
    const DISABLE_TIMEOUT: Duration = Duration::from_millis(800);
    /// Events queued between the inspector and the session task.
    const QUEUE: usize = 256;
    /// Sent when the program has run to its end and Node waits for the
    /// debugger to disconnect (after `NodeRuntime.notifyWhenWaitingForDisconnect`).
    const WAITING_FOR_DISCONNECT: &str = "NodeRuntime.waitingForDisconnect";

    /// Attaches to Node's inspector.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct CdpConnector;

    /// What the next `Debugger.paused` caused: V8 reports a step end and a
    /// user pause both as `other`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Cause {
        Unknown,
        Step,
        Pause,
    }

    fn backend_error(err: &CdpError) -> DebugError {
        DebugError::Backend(err.to_string())
    }

    struct CdpBackend {
        client: CdpClient,
        /// Owned by the backend, not the session: cancelling the session
        /// must leave the socket open until `Debugger.disable` was sent.
        link: CancellationToken,
        cause: Arc<Mutex<Cause>>,
    }

    impl CdpBackend {
        fn expect(&self, cause: Cause) {
            if let Ok(mut c) = self.cause.lock() {
                *c = cause;
            }
        }
    }

    impl DebugBackend for CdpBackend {
        fn command(&self, command: DebugCommand) -> BoxFuture<'_, Result<(), DebugError>> {
            Box::pin(async move {
                let sent = match command {
                    DebugCommand::Resume => {
                        self.client.send(cdp::Resume::default()).await.map(|_| ())
                    }
                    DebugCommand::StepOver => {
                        self.expect(Cause::Step);
                        self.client.send(cdp::StepOver).await.map(|_| ())
                    }
                    DebugCommand::StepInto => {
                        self.expect(Cause::Step);
                        self.client.send(cdp::StepInto).await.map(|_| ())
                    }
                    DebugCommand::StepOut => {
                        self.expect(Cause::Step);
                        self.client.send(cdp::StepOut).await.map(|_| ())
                    }
                    DebugCommand::Pause => {
                        self.expect(Cause::Pause);
                        self.client.send(cdp::Pause).await.map(|_| ())
                    }
                    // The session ends by cancelling its task, which closes.
                    DebugCommand::Stop => Ok(()),
                    DebugCommand::Variables | DebugCommand::Evaluate => {
                        return Err(DebugError::Backend(
                            "variables and evaluation are not available yet".into(),
                        ));
                    }
                };
                sent.map_err(|e| backend_error(&e))
            })
        }

        fn close(&self) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                // Best effort: detach cleanly, then drop the socket.
                let _ =
                    tokio::time::timeout(DISABLE_TIMEOUT, self.client.send(cdp::DebuggerDisable))
                        .await;
                self.client.close().await;
                self.link.cancel();
            })
        }
    }

    impl DebugConnector for CdpConnector {
        fn connect(
            &self,
            port: u16,
            cancel: CancellationToken,
        ) -> BoxFuture<'_, Result<Connection, DebugError>> {
            Box::pin(async move {
                let url = discover(port, DISCOVER_TIMEOUT, &cancel)
                    .await
                    .map_err(|e| backend_error(&e))?;
                let link = CancellationToken::new();
                let (client, events) = CdpClient::connect(&url, link.clone())
                    .await
                    .map_err(|e| backend_error(&e))?;
                match start(&client, events).await {
                    Ok((events, cause)) => Ok(Connection {
                        backend: Arc::new(CdpBackend {
                            client,
                            link,
                            cause,
                        }),
                        events,
                    }),
                    Err(err) => {
                        client.close().await;
                        link.cancel();
                        Err(err)
                    }
                }
            })
        }
    }

    /// Enables the debugger, lets the waiting program run and starts the task
    /// that turns inspector events into session events.
    async fn start(
        client: &CdpClient,
        inspector: CdpEvents,
    ) -> Result<(mpsc::Receiver<rd::DebugEvent>, Arc<Mutex<Cause>>), DebugError> {
        let backend_failed = |e: CdpError| backend_error(&e);
        client
            .send(cdp::DebuggerEnable::default())
            .await
            .map_err(backend_failed)?;
        // Without this Node would wait for a disconnect after the program
        // ended, and we could not tell. Older Nodes may refuse it: then the
        // end is seen when the process closes the socket.
        if let Err(err) = client
            .call::<_, serde_json::Value>(
                "NodeRuntime.notifyWhenWaitingForDisconnect",
                serde_json::json!({ "enabled": true }),
            )
            .await
        {
            tracing::debug!(%err, "NodeRuntime.notifyWhenWaitingForDisconnect refused");
        }
        let (tx, rx) = mpsc::channel(QUEUE);
        // `Started` first, so it precedes the stop at the first line.
        tx.send(rd::DebugEvent::Started)
            .await
            .map_err(|_| DebugError::Backend("the session ended while attaching".into()))?;
        let cause = Arc::new(Mutex::new(Cause::Unknown));
        tokio::spawn(pump(inspector, tx, cause.clone()));
        client
            .send(cdp::RunIfWaitingForDebugger)
            .await
            .map_err(backend_failed)?;
        Ok((rx, cause))
    }

    /// Reads the inspector's events until the connection ends; the end of the
    /// channel tells the session task.
    async fn pump(
        mut inspector: CdpEvents,
        tx: mpsc::Sender<rd::DebugEvent>,
        cause: Arc<Mutex<Cause>>,
    ) {
        // Script id -> URL, for the file of a frame.
        let mut scripts: HashMap<String, String> = HashMap::new();
        while let Some(event) = inspector.recv().await {
            if matches!(&event, CdpEvent::Closed { .. }) {
                break;
            }
            if matches!(&event, CdpEvent::Event { method, .. } if method == WAITING_FOR_DISCONNECT)
            {
                // The program ended; the session task closes the socket,
                // and Node then exits.
                let _ = tx
                    .send(rd::DebugEvent::Ended {
                        exit_code: None,
                        stopped: false,
                    })
                    .await;
                break;
            }
            let session_event = match event.typed() {
                Ok(Some(Event::ScriptParsed(script))) => {
                    scripts.insert(script.script_id, script.url);
                    None
                }
                Ok(Some(Event::Paused(paused))) => {
                    let taken = cause
                        .lock()
                        .map(|mut c| std::mem::replace(&mut *c, Cause::Unknown))
                        .unwrap_or(Cause::Unknown);
                    Some(rd::DebugEvent::Paused(pause_info(&paused, &scripts, taken)))
                }
                Ok(Some(Event::Resumed)) => Some(rd::DebugEvent::Resumed),
                Ok(_) => None,
                Err(err) => {
                    tracing::warn!(%err, "an inspector event could not be decoded");
                    None
                }
            };
            if let Some(event) = session_event
                && tx.send(event).await.is_err()
            {
                break;
            }
        }
    }

    fn pause_info(
        paused: &cdp::Paused,
        scripts: &HashMap<String, String>,
        cause: Cause,
    ) -> rd::PauseInfo {
        let reason = match paused.reason.as_str() {
            "Break on start" => rd::PauseReason::Entry,
            "exception" | "promiseRejection" => rd::PauseReason::Exception {
                message: paused
                    .data
                    .as_ref()
                    .and_then(|d| d.get("description"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("exception")
                    .to_owned(),
            },
            _ if !paused.hit_breakpoints.is_empty() => rd::PauseReason::Breakpoint,
            "debugCommand" => rd::PauseReason::Breakpoint,
            _ => match cause {
                Cause::Step => rd::PauseReason::Step,
                Cause::Pause => rd::PauseReason::Pause,
                // A `debugger;` statement is reported as `other`.
                Cause::Unknown => rd::PauseReason::Breakpoint,
            },
        };
        let frames = paused
            .call_frames
            .iter()
            .map(|frame| {
                let url = scripts.get(&frame.location.script_id);
                let path = url
                    .and_then(|u| rd::file_url_to_path(u))
                    .or_else(|| url.cloned())
                    .unwrap_or_else(|| format!("script {}", frame.location.script_id));
                let is_library = path.starts_with("node:") || path.contains("/node_modules/");
                rd::Frame {
                    id: frame.call_frame_id.clone(),
                    name: frame.function_name.clone(),
                    location: rd::Location {
                        path_or_script: path,
                        line: frame.location.line_number,
                        column: frame.location.column_number.unwrap_or(0),
                        generated: None,
                    },
                    is_library,
                    scopes: Vec::new(),
                }
            })
            .collect();
        rd::PauseInfo::new(reason, frames)
    }
}

/// The scripted debugger for tests and the headless binary (feature
/// `testing`): it wraps [`ferrule_run_debug::fake_debug::FakeDebug`], plays
/// the script to the next stop after every command, and has no process.
#[cfg(feature = "testing")]
pub mod fake {
    use super::{BoxFuture, Connection, DebugBackend, DebugConnector};
    use ferrule_run_debug::fake_debug::FakeDebug;
    use ferrule_run_debug::{DebugCommand, DebugError, DebugEvent};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Builds a fresh scripted program for each session.
    pub struct FakeConnector {
        make: Box<dyn Fn() -> FakeDebug + Send + Sync>,
    }

    impl FakeConnector {
        /// A connector whose sessions play the program `make` returns.
        pub fn new(make: impl Fn() -> FakeDebug + Send + Sync + 'static) -> Self {
            Self {
                make: Box::new(make),
            }
        }
    }

    struct Fake {
        program: Mutex<FakeDebug>,
        events: mpsc::Sender<DebugEvent>,
    }

    impl Fake {
        fn send(&self, events: Vec<DebugEvent>) {
            for event in events {
                // A full queue means the driver is gone; nothing to tell.
                let _ = self.events.try_send(event);
            }
        }
    }

    impl DebugBackend for Fake {
        fn command(&self, command: DebugCommand) -> BoxFuture<'_, Result<(), DebugError>> {
            Box::pin(async move {
                let events = {
                    let mut program = self
                        .program
                        .lock()
                        .map_err(|_| DebugError::Backend("fake debugger poisoned".into()))?;
                    let mut out = match command {
                        DebugCommand::Resume => program.resume()?,
                        DebugCommand::StepOver => program.step_over()?,
                        DebugCommand::StepInto => program.step_into()?,
                        DebugCommand::StepOut => program.step_out()?,
                        DebugCommand::Pause => program.pause()?,
                        DebugCommand::Stop => program.stop()?,
                        DebugCommand::Variables | DebugCommand::Evaluate => {
                            return Err(DebugError::Backend("not in this slice".into()));
                        }
                    };
                    if !matches!(command, DebugCommand::Stop | DebugCommand::Pause) {
                        out.extend(program.advance());
                    }
                    out
                };
                self.send(events);
                Ok(())
            })
        }

        fn close(&self) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }

    impl DebugConnector for FakeConnector {
        fn connect(
            &self,
            _port: u16,
            _cancel: CancellationToken,
        ) -> BoxFuture<'_, Result<Connection, DebugError>> {
            Box::pin(async move {
                let (tx, rx) = mpsc::channel(256);
                let fake = Arc::new(Fake {
                    program: Mutex::new((self.make)()),
                    events: tx,
                });
                let first = {
                    let mut program = fake
                        .program
                        .lock()
                        .map_err(|_| DebugError::Backend("fake debugger poisoned".into()))?;
                    let mut events = program.start()?;
                    events.extend(program.advance());
                    events
                };
                fake.send(first);
                Ok(Connection {
                    backend: fake,
                    events: rx,
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn node(args: &[&str], env: &[(&str, &str)]) -> RunConfig {
        RunConfig {
            id: "c".into(),
            name: "c".into(),
            kind: RunKind::Node {
                file: "main.js".into(),
            },
            cwd: String::new(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            package_manager: None,
        }
    }

    #[test]
    fn inspector_flags_in_a_configuration_are_refused() {
        assert!(check_debuggable(&node(&[], &[])).is_ok());
        assert!(check_debuggable(&node(&["--verbose", "x"], &[("A", "b")])).is_ok());
        for arg in [
            "--inspect",
            "--inspect-brk=0.0.0.0:9229",
            "--inspect-port=1",
        ] {
            let err = check_debuggable(&node(&[arg], &[])).unwrap_err();
            assert_eq!(err.kind, IpcErrorKind::InvalidArgument, "{arg}");
        }
        for key in ["NODE_OPTIONS", "node_options"] {
            let err =
                check_debuggable(&node(&[], &[(key, "--max-old-space-size=1 --inspect-brk")]))
                    .unwrap_err();
            assert_eq!(err.kind, IpcErrorKind::InvalidArgument, "{key}");
        }
        assert!(
            check_debuggable(&node(&[], &[("NODE_OPTIONS", "--max-old-space-size=1")])).is_ok()
        );
    }

    #[test]
    fn only_node_files_are_debugged() {
        let mut config = node(&[], &[]);
        config.kind = RunKind::NpmScript {
            script: "dev".into(),
        };
        assert_eq!(
            check_debuggable(&config).unwrap_err().kind,
            IpcErrorKind::InvalidArgument
        );
    }

    #[test]
    fn a_free_port_is_a_loopback_port() {
        assert!(free_port().unwrap() > 0);
    }

    #[test]
    fn events_map_to_ipc() {
        let paused = rd::DebugEvent::Paused(rd::PauseInfo::new(
            rd::PauseReason::Exception {
                message: "boom".into(),
            },
            vec![rd::Frame {
                id: "1".into(),
                name: "f".into(),
                location: rd::Location {
                    path_or_script: "a.ts".into(),
                    line: 3,
                    column: 2,
                    generated: Some(rd::Position { line: 9, column: 1 }),
                },
                is_library: true,
                scopes: Vec::new(),
            }],
        ));
        let Some(ipc::DebugEventKind::Paused {
            reason,
            frames,
            selected,
        }) = to_ipc(&paused)
        else {
            panic!("expected paused");
        };
        assert_eq!(
            reason,
            ipc::DebugPauseReason::Exception {
                message: "boom".into()
            }
        );
        assert_eq!(selected, 0);
        assert_eq!(frames[0].location.generated.unwrap().line, 9);
        assert!(frames[0].is_library);
        assert!(to_ipc(&rd::DebugEvent::Output { text: "x".into() }).is_none());
    }

    #[tokio::test]
    async fn an_empty_registry_shuts_down_at_once() {
        let debugs = Debugs::new(Arc::new(crate::events::NoEvents), None);
        debugs.shut_down().await;
        assert_eq!(debugs.count(), 0);
        assert!(debugs.get(1).is_err());
    }
}
