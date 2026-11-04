//! `ferrule-perf`: external benchmarks for the performance budget in
//! `CLAUDE.md`. It generates a ~1000-file project, launches the release app
//! as a separate process and measures it only from the outside: screen
//! pixels (X11 `GetImage`), injected input (XTest) and the process tree's
//! CPU and memory (`/proc`). Linux/X11 only for now (`docs/ROADMAP.md`).
//!
//! Usage: `cargo run -p ferrule-perf --release -- [--app target/release/ferrule]
//! [--files 1000] [--keys 30]` with `DISPLAY` set (a real X server, XWayland,
//! or `xvfb-run`).

mod generate;
#[cfg(target_os = "linux")]
mod linux;

use std::path::PathBuf;

use anyhow::{Context, bail};

/// Command-line options.
#[derive(Debug, Clone)]
pub struct Args {
    /// The application binary (a release build).
    pub app: PathBuf,
    /// Number of files in the generated project.
    pub files: usize,
    /// Number of keystrokes for the typing-latency measurement.
    pub keys: usize,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        app: PathBuf::from("target/release/ferrule"),
        files: 1000,
        keys: 30,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--app" => args.app = it.next().context("--app needs a path")?.into(),
            "--files" => args.files = it.next().context("--files needs a number")?.parse()?,
            "--keys" => args.keys = it.next().context("--keys needs a number")?.parse()?,
            other => bail!("unknown argument {other:?}"),
        }
    }
    if !args.app.exists() {
        bail!(
            "{} does not exist; build it with `bun tauri build --no-bundle`",
            args.app.display()
        );
    }
    Ok(args)
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    #[cfg(target_os = "linux")]
    {
        linux::run(&args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (&args, generate::generate);
        bail!("ferrule-perf only supports Linux/X11 so far (see docs/ROADMAP.md)")
    }
}
