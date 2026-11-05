//! A running terminal and the set of open terminals.
//!
//! Threads per terminal (all blocking, none on the async runtime):
//! - reader: reads the PTY in 64 KiB chunks and feeds the screen;
//! - frames: sends at most one frame per interval, one in flight
//!   ([`Pacer`]), with only the rows that changed;
//! - waiter: notices the shell's exit, closes the PTY (on Windows the
//!   output only ends then) and reports the exit code.
//!
//! Lock order: `screen` and `state` are never held together, `writer` is
//! taken alone.

use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::backend::{Backend, Process, Screen, ScreenEvent, SpawnSpec, TerminalError};
use crate::keys::{Key, encode_key, encode_paste};
use crate::model::{CellPos, Dirty, Frame, SelectionKind, TermSize};
use crate::pacer::{Decision, Pacer};

/// Something for the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Screen update.
    Frame(Frame),
    /// The shell exited (its code, if known).
    Exited {
        /// Terminal id.
        id: u32,
        /// Exit code.
        code: Option<i32>,
    },
}

/// Receives terminal events; called from terminal threads, must not block.
pub type EventSink = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

/// Timing and size limits.
#[derive(Debug, Clone)]
pub struct TerminalOptions {
    /// Shortest time between two frames.
    pub frame_interval: Duration,
    /// How long a frame may wait for its acknowledgement.
    pub ack_timeout: Duration,
    /// Lines of scrollback.
    pub scrollback: usize,
    /// How often the waiter checks whether the shell exited.
    pub exit_poll: Duration,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            frame_interval: Duration::from_millis(16),
            ack_timeout: Duration::from_secs(1),
            scrollback: 10_000,
            exit_poll: Duration::from_millis(50),
        }
    }
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded data stays usable even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct State {
    pacer: Pacer,
    size: TermSize,
    full: bool,
    title: String,
    exited: Option<Option<i32>>,
    closed: bool,
}

struct Shared {
    id: u32,
    screen: Mutex<Box<dyn Screen>>,
    writer: Mutex<Box<dyn Write + Send>>,
    process: Box<dyn Process>,
    state: Mutex<State>,
    wake: Condvar,
    sink: EventSink,
    /// Set while the frame thread waits for the screen: the reader then
    /// yields instead of taking the lock again (std mutexes are not fair,
    /// so a flooding reader would otherwise starve the frames).
    frame_waiting: AtomicBool,
}

impl Shared {
    fn changed(&self, full: bool) {
        let mut state = lock(&self.state);
        state.full |= full;
        state.pacer.mark_dirty();
        drop(state);
        self.wake.notify_all();
    }

    fn write(&self, bytes: &[u8]) -> Result<(), TerminalError> {
        let mut writer = lock(&self.writer);
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|e| TerminalError::Io(e.to_string()))
    }

    fn build_frame(&self, seq: u64, mut full: bool) -> Frame {
        let (size, title, exited) = {
            let state = lock(&self.state);
            (state.size, state.title.clone(), state.exited)
        };
        self.frame_waiting.store(true, Ordering::Release);
        let mut screen = lock(&self.screen);
        self.frame_waiting.store(false, Ordering::Release);
        let rows: Vec<u16> = match screen.take_dirty() {
            Dirty::All => {
                full = true;
                (0..size.rows).collect()
            }
            Dirty::Rows(rows) if !full => rows.into_iter().filter(|&r| r < size.rows).collect(),
            Dirty::Rows(_) => (0..size.rows).collect(),
        };
        let rendered = rows.into_iter().map(|row| screen.render_row(row)).collect();
        let (display_offset, history) = screen.scroll_state();
        Frame {
            id: self.id,
            seq,
            size,
            full,
            rows: rendered,
            cursor: screen.cursor(),
            display_offset,
            history,
            title,
            exited,
        }
    }
}

/// One running terminal. Dropping it does not stop it; call
/// [`Terminal::close`].
pub struct Terminal {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("id", &self.shared.id)
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// Starts the shell and the terminal's threads.
    pub fn start(
        id: u32,
        backend: &dyn Backend,
        spec: &SpawnSpec,
        size: TermSize,
        options: &TerminalOptions,
        sink: EventSink,
    ) -> Result<Self, TerminalError> {
        let spawned = backend.spawn(spec, size)?;
        let screen = backend.screen(size, options.scrollback);
        let mut pacer = Pacer::new(options.frame_interval, options.ack_timeout);
        pacer.mark_dirty();
        let shared = Arc::new(Shared {
            id,
            screen: Mutex::new(screen),
            writer: Mutex::new(spawned.writer),
            process: spawned.process,
            state: Mutex::new(State {
                pacer,
                size,
                full: true,
                title: String::new(),
                exited: None,
                closed: false,
            }),
            wake: Condvar::new(),
            sink,
            frame_waiting: AtomicBool::new(false),
        });
        let reader = spawned.reader;
        let started = [
            spawn_named(format!("terminal-{id}-reader"), {
                let shared = shared.clone();
                move || read_loop(&shared, reader)
            }),
            spawn_named(format!("terminal-{id}-frames"), {
                let shared = shared.clone();
                move || frame_loop(&shared)
            }),
            spawn_named(format!("terminal-{id}-waiter"), {
                let shared = shared.clone();
                let poll = options.exit_poll;
                move || wait_loop(&shared, poll)
            }),
        ];
        if let Some(err) = started.into_iter().find_map(Result::err) {
            shared.process.terminate();
            return Err(TerminalError::Io(err.to_string()));
        }
        Ok(Self { shared })
    }

    /// The terminal id.
    pub fn id(&self) -> u32 {
        self.shared.id
    }

    /// Typed or pasted text without bracketing (follows output again).
    pub fn input(&self, text: &str) -> Result<(), TerminalError> {
        self.follow_output();
        self.shared.write(text.as_bytes())
    }

    /// A special key, encoded for the current modes. Unknown keys do nothing.
    pub fn key(&self, key: &Key) -> Result<(), TerminalError> {
        let modes = lock(&self.shared.screen).modes();
        match encode_key(key, modes) {
            Some(bytes) => {
                self.follow_output();
                self.shared.write(&bytes)
            }
            None => Ok(()),
        }
    }

    /// Pasted text (bracketed when the program asked for it).
    pub fn paste(&self, text: &str) -> Result<(), TerminalError> {
        let modes = lock(&self.shared.screen).modes();
        self.follow_output();
        self.shared.write(&encode_paste(text, modes))
    }

    fn follow_output(&self) {
        let mut screen = lock(&self.shared.screen);
        if screen.scroll_state().0 > 0 {
            screen.scroll_to_bottom();
            drop(screen);
            self.shared.changed(true);
        }
    }

    /// Resizes the PTY and the screen.
    pub fn resize(&self, size: TermSize) -> Result<(), TerminalError> {
        {
            let mut state = lock(&self.shared.state);
            if state.size == size {
                return Ok(());
            }
            state.size = size;
        }
        lock(&self.shared.screen).resize(size);
        self.shared.changed(true);
        self.shared.process.resize(size)
    }

    /// Scrolls the viewport (positive: up into the history).
    pub fn scroll(&self, lines: i32) {
        lock(&self.shared.screen).scroll(lines);
        self.shared.changed(true);
    }

    /// Selects from `anchor` to `head`.
    pub fn select(&self, anchor: CellPos, head: CellPos, kind: SelectionKind) {
        lock(&self.shared.screen).select(anchor, head, kind);
        self.shared.changed(true);
    }

    /// Clears the selection.
    pub fn clear_selection(&self) {
        lock(&self.shared.screen).clear_selection();
        self.shared.changed(true);
    }

    /// The selected text.
    pub fn selection_text(&self) -> Option<String> {
        lock(&self.shared.screen).selection_text()
    }

    /// The text of a viewport row (one character per column).
    pub fn row_text(&self, row: u16) -> String {
        lock(&self.shared.screen).row_text(row)
    }

    /// The view drew frame `seq`.
    pub fn ack(&self, seq: u64) {
        lock(&self.shared.state).pacer.ack(seq);
        self.shared.wake.notify_all();
    }

    /// The title set by the program.
    pub fn title(&self) -> String {
        lock(&self.shared.state).title.clone()
    }

    /// The exit code once the shell has exited.
    pub fn exited(&self) -> Option<Option<i32>> {
        lock(&self.shared.state).exited
    }

    /// Ends the shell and everything it started, and stops the threads.
    /// Does not wait for them.
    pub fn close(&self) {
        lock(&self.shared.state).closed = true;
        self.shared.process.terminate();
        self.shared.wake.notify_all();
    }
}

fn spawn_named(
    name: String,
    body: impl FnOnce() + Send + 'static,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new().name(name).spawn(body)
}

fn read_loop(shared: &Shared, mut reader: Box<dyn Read + Send>) {
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        while shared.frame_waiting.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let events = lock(&shared.screen).feed(&buf[..n]);
        for event in events {
            match event {
                ScreenEvent::Title(title) => lock(&shared.state).title = title,
                ScreenEvent::Reply(bytes) => {
                    if let Err(err) = shared.write(&bytes) {
                        tracing::debug!(%err, "terminal reply not written");
                    }
                }
            }
        }
        shared.changed(false);
        if lock(&shared.state).closed {
            break;
        }
    }
}

fn frame_loop(shared: &Shared) {
    let mut state = lock(&shared.state);
    loop {
        if state.closed {
            return;
        }
        match state.pacer.decide(Instant::now()) {
            Decision::Send(seq) => {
                let full = std::mem::take(&mut state.full);
                drop(state);
                let frame = shared.build_frame(seq, full);
                (shared.sink)(TerminalEvent::Frame(frame));
                state = lock(&shared.state);
            }
            Decision::Idle => {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            Decision::Wait(delay) => {
                state = shared
                    .wake
                    .wait_timeout(state, delay)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }
}

fn wait_loop(shared: &Shared, poll: Duration) {
    loop {
        if let Some(code) = shared.process.exit_status() {
            {
                let mut state = lock(&shared.state);
                state.exited = Some(code);
                state.pacer.mark_dirty();
            }
            shared.wake.notify_all();
            // Closes the PTY: on Windows the output ends only then.
            shared.process.terminate();
            (shared.sink)(TerminalEvent::Exited {
                id: shared.id,
                code,
            });
            return;
        }
        if lock(&shared.state).closed {
            return;
        }
        thread::sleep(poll);
    }
}

/// What the view shows about a terminal tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    /// Id.
    pub id: u32,
    /// Tab title: the user's name for it, else the program's title, else
    /// the shell's name.
    pub title: String,
    /// The shell program.
    pub shell: PathBuf,
    /// Working directory it started in.
    pub cwd: PathBuf,
}

struct Entry {
    terminal: Arc<Terminal>,
    shell: PathBuf,
    cwd: PathBuf,
    renamed: Option<String>,
}

impl Entry {
    fn info(&self) -> TerminalInfo {
        let title = self.renamed.clone().unwrap_or_else(|| {
            let program = self.terminal.title();
            if program.is_empty() {
                self.shell
                    .file_stem()
                    .map_or_else(|| "Local".to_owned(), |s| s.to_string_lossy().into_owned())
            } else {
                program
            }
        });
        TerminalInfo {
            id: self.terminal.id(),
            title,
            shell: self.shell.clone(),
            cwd: self.cwd.clone(),
        }
    }
}

/// The open terminals of a project.
pub struct Terminals {
    backend: Option<Arc<dyn Backend>>,
    options: TerminalOptions,
    sink: EventSink,
    next_id: AtomicU32,
    open: Mutex<Vec<Entry>>,
}

impl std::fmt::Debug for Terminals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminals").finish_non_exhaustive()
    }
}

impl Terminals {
    /// A set using `backend` (`None`: terminals are unavailable).
    pub fn new(
        backend: Option<Arc<dyn Backend>>,
        options: TerminalOptions,
        sink: EventSink,
    ) -> Self {
        Self {
            backend,
            options,
            sink,
            next_id: AtomicU32::new(1),
            open: Mutex::new(Vec::new()),
        }
    }

    /// Whether terminals can be started at all.
    pub fn available(&self) -> bool {
        self.backend.is_some()
    }

    /// Starts a terminal.
    pub fn open(&self, spec: &SpawnSpec, size: TermSize) -> Result<TerminalInfo, TerminalError> {
        let backend = self.backend.as_deref().ok_or(TerminalError::Unavailable)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let terminal = Terminal::start(id, backend, spec, size, &self.options, self.sink.clone())?;
        let entry = Entry {
            terminal: Arc::new(terminal),
            shell: spec.program.clone(),
            cwd: spec.cwd.clone(),
            renamed: None,
        };
        let info = entry.info();
        lock(&self.open).push(entry);
        Ok(info)
    }

    /// A terminal by id.
    pub fn get(&self, id: u32) -> Result<Arc<Terminal>, TerminalError> {
        lock(&self.open)
            .iter()
            .find(|e| e.terminal.id() == id)
            .map(|e| e.terminal.clone())
            .ok_or(TerminalError::NotFound(id))
    }

    /// Every open terminal, in opening order.
    pub fn list(&self) -> Vec<TerminalInfo> {
        lock(&self.open).iter().map(Entry::info).collect()
    }

    /// Renames a tab (an empty name goes back to the program's title).
    pub fn rename(&self, id: u32, title: &str) -> Result<TerminalInfo, TerminalError> {
        let mut open = lock(&self.open);
        let entry = open
            .iter_mut()
            .find(|e| e.terminal.id() == id)
            .ok_or(TerminalError::NotFound(id))?;
        let title = title.trim();
        entry.renamed = (!title.is_empty()).then(|| title.to_owned());
        Ok(entry.info())
    }

    /// Closes one terminal (ends its processes).
    pub fn close(&self, id: u32) -> Result<(), TerminalError> {
        let entry = {
            let mut open = lock(&self.open);
            let index = open
                .iter()
                .position(|e| e.terminal.id() == id)
                .ok_or(TerminalError::NotFound(id))?;
            open.remove(index)
        };
        entry.terminal.close();
        Ok(())
    }

    /// Closes every terminal (project closed or app quitting).
    pub fn close_all(&self) {
        let entries: Vec<Entry> = lock(&self.open).drain(..).collect();
        for entry in entries {
            entry.terminal.close();
        }
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        self.close_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::fake::FakeBackend;

    fn spec() -> SpawnSpec {
        SpawnSpec {
            program: PathBuf::from("/bin/fake-sh"),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
        }
    }

    fn options() -> TerminalOptions {
        TerminalOptions {
            frame_interval: Duration::from_millis(5),
            ack_timeout: Duration::from_millis(400),
            scrollback: 100,
            exit_poll: Duration::from_millis(5),
        }
    }

    fn channel_sink() -> (EventSink, mpsc::Receiver<TerminalEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: EventSink = Arc::new(move |event| {
            let _ = lock(&tx).send(event);
        });
        (sink, rx)
    }

    /// Receives frames (acknowledging each) until `done` holds for the
    /// screen text assembled from them.
    fn frames_until(
        terminal: &Terminal,
        rx: &mpsc::Receiver<TerminalEvent>,
        done: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let mut rows: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(TerminalEvent::Frame(frame)) = rx.recv_timeout(Duration::from_millis(50)) {
                rows.resize(usize::from(frame.size.rows), String::new());
                for row in &frame.rows {
                    rows[usize::from(row.row)] = row.runs.iter().map(|r| r.text.as_str()).collect();
                }
                terminal.ack(frame.seq);
                if done(&rows.join("\n")) {
                    return rows;
                }
            }
        }
        panic!("timed out; screen was:\n{}", rows.join("\n"));
    }

    #[test]
    fn echoes_input_and_answers_queries() {
        let backend = FakeBackend::default();
        let (sink, rx) = channel_sink();
        let t =
            Terminal::start(1, &backend, &spec(), TermSize::new(20, 4), &options(), sink).unwrap();
        t.input("echo hi\r").unwrap();
        frames_until(&t, &rx, |screen| screen.contains("$ echo hi"));
        // A cursor position query is answered through the PTY.
        t.input("\x1b[6n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !String::from_utf8_lossy(&lock(&backend.log.written)).contains("R") {
            assert!(Instant::now() < deadline, "no reply written");
            thread::sleep(Duration::from_millis(5));
        }
        t.close();
    }

    #[test]
    fn a_flood_sends_few_frames_and_waits_for_acknowledgements() {
        let backend = FakeBackend {
            flood: true,
            ..FakeBackend::default()
        };
        let (sink, rx) = channel_sink();
        let t = Terminal::start(
            2,
            &backend,
            &spec(),
            TermSize::new(40, 10),
            &options(),
            sink,
        )
        .unwrap();
        // Never acknowledged: exactly one frame until the ack timeout.
        thread::sleep(Duration::from_millis(200));
        let unacked = rx
            .try_iter()
            .filter(|e| matches!(e, TerminalEvent::Frame(_)))
            .count();
        assert_eq!(unacked, 1, "one frame in flight without acknowledgements");
        // Acknowledged at once: bounded by the interval, not by the output.
        let started = Instant::now();
        let mut frames = 0_u32;
        while started.elapsed() < Duration::from_millis(300) {
            if let Ok(TerminalEvent::Frame(frame)) = rx.recv_timeout(Duration::from_millis(50)) {
                frames += 1;
                assert!(frame.rows.len() <= 10, "only visible rows are sent");
                t.ack(frame.seq);
            }
        }
        assert!(frames >= 5, "frames keep coming while flooding ({frames})");
        assert!(
            frames <= 300 / 5 + 2,
            "at most one frame per interval ({frames})"
        );
        t.close();
        assert_eq!(*lock(&backend.log.terminated), 1);
    }

    #[test]
    fn resize_reaches_the_process_and_repaints_everything() {
        let backend = FakeBackend::default();
        let (sink, rx) = channel_sink();
        let t =
            Terminal::start(3, &backend, &spec(), TermSize::new(20, 4), &options(), sink).unwrap();
        frames_until(&t, &rx, |screen| screen.contains('$'));
        t.resize(TermSize::new(30, 6)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(TerminalEvent::Frame(frame)) = rx.recv_timeout(Duration::from_millis(50)) {
                t.ack(frame.seq);
                if frame.size == TermSize::new(30, 6) {
                    assert!(frame.full);
                    assert_eq!(frame.rows.len(), 6);
                    break;
                }
            }
            assert!(Instant::now() < deadline, "no frame with the new size");
        }
        assert_eq!(*lock(&backend.log.resized), [TermSize::new(30, 6)]);
        t.close();
    }

    #[test]
    fn exit_and_close_report_and_end_the_process() {
        let backend = FakeBackend::default();
        let (sink, rx) = channel_sink();
        let t =
            Terminal::start(4, &backend, &spec(), TermSize::new(20, 4), &options(), sink).unwrap();
        t.input("exit\r").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(TerminalEvent::Exited { id, code }) => {
                    assert_eq!((id, code), (4, Some(0)));
                    break;
                }
                Ok(TerminalEvent::Frame(frame)) => t.ack(frame.seq),
                Err(_) => {}
            }
            assert!(Instant::now() < deadline, "no exit event");
        }
        assert_eq!(t.exited(), Some(Some(0)));
        // The waiter closed the PTY after the exit.
        assert_eq!(*lock(&backend.log.terminated), 1);
    }

    #[test]
    fn registry_opens_renames_lists_and_closes() {
        let backend: Arc<dyn Backend> = Arc::new(FakeBackend::default());
        let (sink, _rx) = channel_sink();
        let terminals = Terminals::new(Some(backend), options(), sink);
        let a = terminals.open(&spec(), TermSize::new(20, 4)).unwrap();
        let b = terminals.open(&spec(), TermSize::new(20, 4)).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.title, "fake-sh");
        assert_eq!(terminals.rename(b.id, " Build ").unwrap().title, "Build");
        assert_eq!(terminals.rename(b.id, "").unwrap().title, "fake-sh");
        terminals.close(a.id).unwrap();
        assert!(matches!(
            terminals.get(a.id),
            Err(TerminalError::NotFound(_))
        ));
        assert_eq!(terminals.list().len(), 1);
        terminals.close_all();
        assert!(terminals.list().is_empty());
    }

    #[test]
    fn failures_and_missing_backend_are_errors() {
        let (sink, _rx) = channel_sink();
        let none = Terminals::new(None, options(), sink.clone());
        assert!(!none.available());
        assert!(matches!(
            none.open(&spec(), TermSize::new(10, 2)),
            Err(TerminalError::Unavailable)
        ));
        let failing: Arc<dyn Backend> = Arc::new(FakeBackend {
            fail: Some("no such file".into()),
            ..FakeBackend::default()
        });
        let terminals = Terminals::new(Some(failing), options(), sink);
        let err = terminals.open(&spec(), TermSize::new(10, 2)).unwrap_err();
        assert!(err.to_string().contains("no such file"), "{err}");
    }
}
