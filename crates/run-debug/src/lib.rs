//! Run configurations and test reports (`docs/ARCHITECTURE.md`, 4.10).
//!
//! Pure logic, no processes: the configuration model and its file
//! (`.ferrule/run.json`), detection from `package.json`, the exact command
//! a configuration runs, test filters, and parsing of the JSON reports of
//! Vitest, Jest and Playwright into one test tree. `ferrule-core` starts
//! and supervises the processes.
//!
//! Debugging (M5b): the protocol-neutral session model ([`DebugState`],
//! [`PauseInfo`], [`Frame`], [`Variable`]), the [`BreakpointStore`], log
//! points ([`logpoint_condition`]) and, with the `testing` feature, a
//! scripted `fake_debug` backend. Still no process and no async runtime.

mod breakpoints;
mod command;
mod config;
mod debug;
mod detect;
#[cfg(feature = "testing")]
pub mod fake;
#[cfg(feature = "testing")]
pub mod fake_debug;
mod logpoint;
pub mod paths;
mod report;
mod sourcemap;
mod urls;

pub use breakpoints::{
    Breakpoint, BreakpointEdit, BreakpointError, BreakpointStore, Shifted, Toggled,
};
pub use command::{CommandError, CommandLine, Os, Tools, command_line, test_name_pattern};
pub use config::{
    ConfigError, PackageManager, RunConfig, RunKind, TestFramework, TestRef, TestScope,
    load_configs, save_configs,
};
pub use debug::{
    BreakpointState, DebugCommand, DebugError, DebugEvent, DebugState, Frame, Location,
    MAX_PREVIEW_CHARS, PAGE_SIZE, PauseInfo, PauseReason, Position, Scope, ScopeKind, Variable,
    VariablesPage, cut_preview,
};
pub use detect::{detect_configs, detect_package_manager};
pub use logpoint::logpoint_condition;
pub use report::{
    Failure, NodeKind, StackFrame, TestNode, TestReport, TestStatus, parse_jest_report,
    parse_playwright_report,
};
pub use sourcemap::{Generated, Original, SourceMap, SourceMapError, SourcePosition};
pub use urls::{Host, file_url_to_path, path_to_file_url, same_file};
