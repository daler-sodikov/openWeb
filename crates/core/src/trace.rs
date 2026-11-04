//! Typing-path timing (`FERRULE_TYPING_TRACE=<file>`, `docs/ARCHITECTURE.md`
//! 4.8): Rust's own stage durations are buffered in memory and written,
//! together with the view's samples, when the view flushes. Lines are
//! `stage<TAB>milliseconds`; `ferrule-perf` reads them.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ferrule_ipc::TraceSample;

pub(crate) struct TypingTrace {
    path: PathBuf,
    rust: Mutex<Vec<(&'static str, f64)>>,
}

impl TypingTrace {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            rust: Mutex::new(Vec::new()),
        }
    }

    /// Records how long a Rust stage took.
    pub fn record(&self, stage: &'static str, took: Duration) {
        if let Ok(mut rust) = self.rust.lock() {
            rust.push((stage, took.as_secs_f64() * 1000.0));
        }
    }

    /// Appends the view's samples and the buffered Rust ones to the file.
    pub fn flush(&self, view: &[TraceSample]) -> std::io::Result<()> {
        let rust = self
            .rust
            .lock()
            .map(|mut rust| std::mem::take(&mut *rust))
            .unwrap_or_default();
        if view.is_empty() && rust.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for sample in view {
            text.push_str(&format!("{}\t{:.3}\n", sample.stage, sample.ms));
        }
        for (stage, ms) in rust {
            text.push_str(&format!("{stage}\t{ms:.3}\n"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(text.as_bytes())
    }
}
