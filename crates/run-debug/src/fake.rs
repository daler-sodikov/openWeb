//! An in-memory run backend for UI and integration tests (feature
//! `testing`; never in the app). Test runs write the recorded reporter JSON
//! of `tests/fixtures` (paths pointing into the run's folder) and print a
//! summary; other runs print their command and a file link. Scripts named
//! `dev`, `serve` or `watch` keep running until stopped.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ferrule_terminal::fake::FakeScreen;
use ferrule_terminal::{Backend, Process, Screen, SpawnSpec, Spawned, TermSize, TerminalError};

const VITEST: &str = include_str!("../tests/fixtures/vitest.json");
const JEST: &str = include_str!("../tests/fixtures/jest.handwritten.json");
const PLAYWRIGHT: &str = include_str!("../tests/fixtures/playwright.json");

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The in-memory run backend.
#[derive(Debug, Default)]
pub struct FakeRunBackend;

struct Shared {
    /// Output still to be read; `None` once the program ended (EOF).
    out: Mutex<Option<SyncSender<Vec<u8>>>>,
    exit: Mutex<Option<Option<i32>>>,
}

struct Reader {
    rx: Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

impl Read for Reader {
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

struct Sink;

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FakeProcess(Arc<Shared>);

impl Process for FakeProcess {
    fn resize(&self, _size: TermSize) -> Result<(), TerminalError> {
        Ok(())
    }

    fn terminate(&self) {
        lock(&self.0.out).take();
        // Killed: no exit code (as after a signal).
        lock(&self.0.exit).get_or_insert(None);
    }

    fn exit_status(&self) -> Option<Option<i32>> {
        *lock(&self.0.exit)
    }
}

/// What the fake does for a command.
enum Plan {
    Report {
        path: String,
        json: &'static str,
        name: &'static str,
    },
    Wait,
    Print,
}

fn plan(spec: &SpawnSpec) -> Plan {
    let report_arg = spec.args.iter().find_map(|a| {
        a.strip_prefix("--outputFile.json=")
            .map(|p| (p, VITEST, "vitest"))
            .or_else(|| a.strip_prefix("--outputFile=").map(|p| (p, JEST, "jest")))
    });
    let playwright = spec
        .env
        .iter()
        .find(|(k, _)| k == "PLAYWRIGHT_JSON_OUTPUT_NAME")
        .map(|(_, p)| (p.as_str(), PLAYWRIGHT, "playwright"));
    if let Some((path, json, name)) = report_arg.or(playwright) {
        return Plan::Report {
            path: path.to_owned(),
            json,
            name,
        };
    }
    let script = spec
        .args
        .windows(2)
        .find(|w| w[0] == "run")
        .map(|w| w[1].as_str());
    if matches!(script, Some("dev" | "serve" | "watch")) {
        Plan::Wait
    } else {
        Plan::Print
    }
}

impl Backend for FakeRunBackend {
    fn spawn(&self, spec: &SpawnSpec, _size: TermSize) -> Result<Spawned, TerminalError> {
        let (tx, rx) = sync_channel(64);
        let shared = Arc::new(Shared {
            out: Mutex::new(Some(tx.clone())),
            exit: Mutex::new(None),
        });
        let command = std::iter::once(spec.program.to_string_lossy().into_owned())
            .chain(spec.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut lines = vec![format!("fake run: {command}")];
        match plan(spec) {
            Plan::Report { path, json, name } => {
                // Inside JSON strings: escaped (Windows paths have `\`).
                let quoted = serde_json::to_string(&spec.cwd.to_string_lossy())
                    .map_err(|e| TerminalError::Io(e.to_string()))?;
                let root = quoted.trim_matches('"');
                let report = json
                    .replace("__ROOT__", root)
                    .replace("__REPO__", root)
                    .replace("__NODE__", "node");
                if let Some(dir) = Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&path, &report).map_err(|e| TerminalError::Io(e.to_string()))?;
                let failed = report.contains("\"failed\"") || report.contains("\"unexpected\"");
                lines.push(format!(
                    "{name}: {}",
                    if failed {
                        "some tests failed"
                    } else {
                        "all tests passed"
                    }
                ));
                *lock(&shared.exit) = Some(Some(i32::from(failed)));
            }
            Plan::Print => {
                lines.push("started".to_owned());
                lines.push("see src/math.ts:5:10".to_owned());
                *lock(&shared.exit) = Some(Some(0));
            }
            Plan::Wait => {
                lines.push("serving until stopped".to_owned());
            }
        }
        let mut text = lines.join("\r\n");
        text.push_str("\r\n");
        let _ = tx.try_send(text.into_bytes());
        if lock(&shared.exit).is_some() {
            // Finished: end of output once it is read.
            lock(&shared.out).take();
        }
        drop(tx);
        Ok(Spawned {
            reader: Box::new(Reader {
                rx,
                pending: Vec::new(),
            }),
            writer: Box::new(Sink),
            process: Box::new(FakeProcess(shared)),
        })
    }

    fn screen(&self, size: TermSize, _scrollback: usize) -> Box<dyn Screen> {
        Box::new(FakeScreen::new(size))
    }
}
