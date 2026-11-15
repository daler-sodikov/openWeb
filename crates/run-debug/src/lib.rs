//! Run configurations and test reports (`docs/ARCHITECTURE.md`, 4.10).
//!
//! Pure logic, no processes: the configuration model and its file
//! (`.ferrule/run.json`), detection from `package.json`, the exact command
//! a configuration runs, test filters, and parsing of the JSON reports of
//! Vitest, Jest and Playwright into one test tree. `ferrule-core` starts
//! and supervises the processes.

mod command;
mod config;
mod detect;
#[cfg(feature = "testing")]
pub mod fake;
mod report;

pub use command::{CommandError, CommandLine, Os, Tools, command_line, test_name_pattern};
pub use config::{
    ConfigError, PackageManager, RunConfig, RunKind, TestFramework, TestRef, TestScope,
    load_configs, save_configs,
};
pub use detect::{detect_configs, detect_package_manager};
pub use report::{
    Failure, NodeKind, StackFrame, TestNode, TestReport, TestStatus, parse_jest_report,
    parse_playwright_report,
};
