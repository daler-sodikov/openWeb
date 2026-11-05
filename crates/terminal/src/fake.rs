//! An in-memory backend for tests: a "shell" that echoes input (and can
//! flood output) and a plain-text screen. Not a terminal emulator.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use crate::backend::{Backend, Process, Screen, ScreenEvent, SpawnSpec, Spawned, TerminalError};
use crate::model::{
    CellPos, CursorPos, Dirty, Modes, RenderRow, Run, SelectionKind, Style, TermSize,
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the fake processes saw (shared with the test).
#[derive(Debug, Default)]
pub struct FakeLog {
    /// Every byte written to a terminal.
    pub written: Mutex<Vec<u8>>,
    /// Every resize.
    pub resized: Mutex<Vec<TermSize>>,
    /// How many processes were terminated.
    pub terminated: Mutex<usize>,
    /// Every spawn request.
    pub spawned: Mutex<Vec<SpawnSpec>>,
}

/// A backend whose shells echo their input; `flood` makes them print
/// `y` lines as fast as they are read (like `yes`).
#[derive(Debug, Default)]
pub struct FakeBackend {
    /// Print endlessly.
    pub flood: bool,
    /// Fail every spawn with this message.
    pub fail: Option<String>,
    /// Shared log.
    pub log: Arc<FakeLog>,
}

struct Pipe {
    tx: Mutex<Option<SyncSender<Vec<u8>>>>,
    stop: AtomicBool,
    exit: Mutex<Option<Option<i32>>>,
    /// The line typed so far (`exit` + Enter ends the "shell").
    line: Mutex<String>,
    log: Arc<FakeLog>,
}

impl Pipe {
    fn send(&self, bytes: Vec<u8>) {
        if let Some(tx) = lock(&self.tx).as_ref() {
            // The test reader drains quickly; a full pipe drops echo bytes.
            let _ = tx.try_send(bytes);
        }
    }
}

struct PipeReader {
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.pending = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

struct EchoWriter(Arc<Pipe>);

impl Write for EchoWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0.log.written).extend_from_slice(buf);
        let text = String::from_utf8_lossy(buf);
        {
            let mut line = lock(&self.0.line);
            for c in text.chars() {
                if c == '\r' {
                    if line.trim() == "exit" {
                        *lock(&self.0.exit) = Some(Some(0));
                    }
                    line.clear();
                } else {
                    line.push(c);
                }
            }
        }
        self.0.send(text.replace('\r', "\r\n").into_bytes());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FakeProcess(Arc<Pipe>);

impl Process for FakeProcess {
    fn resize(&self, size: TermSize) -> Result<(), TerminalError> {
        lock(&self.0.log.resized).push(size);
        Ok(())
    }

    fn terminate(&self) {
        if lock(&self.0.tx).take().is_some() {
            *lock(&self.0.log.terminated) += 1;
        }
        self.0.stop.store(true, Ordering::Relaxed);
        lock(&self.0.exit).get_or_insert(None);
    }

    fn exit_status(&self) -> Option<Option<i32>> {
        *lock(&self.0.exit)
    }
}

impl Backend for FakeBackend {
    fn spawn(&self, spec: &SpawnSpec, _size: TermSize) -> Result<Spawned, TerminalError> {
        lock(&self.log.spawned).push(spec.clone());
        if let Some(message) = &self.fail {
            return Err(TerminalError::Spawn {
                program: spec.program.display().to_string(),
                message: message.clone(),
            });
        }
        let (tx, rx) = sync_channel(16);
        let pipe = Arc::new(Pipe {
            tx: Mutex::new(Some(tx.clone())),
            stop: AtomicBool::new(false),
            exit: Mutex::new(None),
            line: Mutex::new(String::new()),
            log: self.log.clone(),
        });
        pipe.send(b"$ ".to_vec());
        if self.flood {
            let flood = pipe.clone();
            thread::spawn(move || {
                let chunk = "y\r\n".repeat(4096).into_bytes();
                while !flood.stop.load(Ordering::Relaxed) {
                    match tx.try_send(chunk.clone()) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => thread::sleep(Duration::from_millis(1)),
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            });
        } else {
            drop(tx);
        }
        Ok(Spawned {
            reader: Box::new(PipeReader {
                rx,
                pending: Vec::new(),
            }),
            writer: Box::new(EchoWriter(pipe.clone())),
            process: Box::new(FakeProcess(pipe)),
        })
    }

    fn screen(&self, size: TermSize, _scrollback: usize) -> Box<dyn Screen> {
        Box::new(FakeScreen::new(size))
    }
}

/// A plain-text grid: printable characters, CR, LF and backspace; answers
/// the cursor position query `ESC [6n`.
#[derive(Debug)]
pub struct FakeScreen {
    size: TermSize,
    rows: Vec<Vec<char>>,
    cursor: (u16, u16),
    dirty: BTreeSet<u16>,
    all: bool,
    history: u32,
    offset: u32,
    selection: Option<(CellPos, CellPos)>,
}

impl FakeScreen {
    /// An empty screen.
    pub fn new(size: TermSize) -> Self {
        Self {
            size,
            rows: vec![vec![' '; usize::from(size.cols)]; usize::from(size.rows)],
            cursor: (0, 0),
            dirty: BTreeSet::new(),
            all: true,
            history: 0,
            offset: 0,
            selection: None,
        }
    }

    fn newline(&mut self) {
        if self.cursor.0 + 1 < self.size.rows {
            self.cursor.0 += 1;
        } else {
            self.rows.remove(0);
            self.rows.push(vec![' '; usize::from(self.size.cols)]);
            self.history += 1;
            self.all = true;
        }
    }
}

impl Screen for FakeScreen {
    fn feed(&mut self, bytes: &[u8]) -> Vec<ScreenEvent> {
        let mut events = Vec::new();
        let text = String::from_utf8_lossy(bytes);
        if text.contains("\x1b[6n") {
            let (r, c) = self.cursor;
            events.push(ScreenEvent::Reply(
                format!("\x1b[{};{}R", r + 1, c + 1).into_bytes(),
            ));
        }
        for ch in text.replace("\x1b[6n", "").chars() {
            match ch {
                '\r' => self.cursor.1 = 0,
                '\n' => self.newline(),
                '\x08' => self.cursor.1 = self.cursor.1.saturating_sub(1),
                c if !c.is_control() => {
                    if self.cursor.1 >= self.size.cols {
                        self.cursor.1 = 0;
                        self.newline();
                    }
                    let (r, col) = (usize::from(self.cursor.0), usize::from(self.cursor.1));
                    self.rows[r][col] = c;
                    self.dirty.insert(self.cursor.0);
                    self.cursor.1 += 1;
                }
                _ => {}
            }
        }
        events
    }

    fn resize(&mut self, size: TermSize) {
        let cols = usize::from(size.cols);
        self.rows.resize(usize::from(size.rows), vec![' '; cols]);
        for row in &mut self.rows {
            row.resize(cols, ' ');
        }
        self.size = size;
        self.cursor = (
            self.cursor.0.min(size.rows - 1),
            self.cursor.1.min(size.cols - 1),
        );
        self.all = true;
    }

    fn take_dirty(&mut self) -> Dirty {
        let dirty = if self.all {
            Dirty::All
        } else {
            Dirty::Rows(self.dirty.iter().copied().collect())
        };
        self.all = false;
        self.dirty.clear();
        dirty
    }

    fn render_row(&self, row: u16) -> RenderRow {
        RenderRow {
            row,
            runs: vec![Run {
                text: self.row_text(row),
                style: Style::default(),
            }],
        }
    }

    fn row_text(&self, row: u16) -> String {
        self.rows
            .get(usize::from(row))
            .map(|r| r.iter().collect())
            .unwrap_or_default()
    }

    fn cursor(&self) -> Option<CursorPos> {
        Some(CursorPos {
            row: self.cursor.0,
            col: self.cursor.1,
        })
    }

    fn scroll(&mut self, lines: i32) {
        let target = i64::from(self.offset) + i64::from(lines);
        self.offset = u32::try_from(target.clamp(0, i64::from(self.history))).unwrap_or(0);
        self.all = true;
    }

    fn scroll_to_bottom(&mut self) {
        self.offset = 0;
        self.all = true;
    }

    fn scroll_state(&self) -> (u32, u32) {
        (self.offset, self.history)
    }

    fn modes(&self) -> Modes {
        Modes::default()
    }

    fn select(&mut self, anchor: CellPos, head: CellPos, _kind: SelectionKind) {
        self.selection = Some((anchor, head));
    }

    fn clear_selection(&mut self) {
        self.selection = None;
    }

    fn selection_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let (start, end) = if (a.row, a.col) <= (b.row, b.col) {
            (a, b)
        } else {
            (b, a)
        };
        let lines: Vec<String> = (start.row..=end.row)
            .map(|row| {
                let text: Vec<char> = self.row_text(row).chars().collect();
                let from = if row == start.row {
                    usize::from(start.col)
                } else {
                    0
                };
                let to = if row == end.row {
                    usize::from(end.col) + 1
                } else {
                    text.len()
                };
                text[from.min(text.len())..to.min(text.len())]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect();
        Some(lines.join("\n"))
    }
}
