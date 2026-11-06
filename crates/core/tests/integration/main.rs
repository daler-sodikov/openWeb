//! Core integration tests in one test binary (each file is a module), so
//! the core is linked once instead of once per file. The real
//! TypeScript-server tests (`lsp_real*.rs`) stay separate binaries: they
//! are slow and timing-sensitive, and would otherwise run in parallel with
//! everything here.

mod git;
mod lsp_fake;
mod m2;
mod m2b;
mod session;
mod terminal;
