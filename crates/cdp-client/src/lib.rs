//! Chrome DevTools Protocol client for Node's inspector.
//!
//! This crate is the transport only: finding the inspector's WebSocket URL
//! ([`discover`]), one connection task that correlates request ids with
//! answers and forwards events ([`CdpClient`], [`CdpEvents`]), and a scripted
//! fake server for tests (feature `testing`). It knows nothing about
//! breakpoints or the editor, and it depends on no other Ferrule crate.
//!
//! Everything stays on loopback: [`CdpClient::connect`] refuses any URL that
//! is not `ws://127.0.0.1:<port>/...`.

mod discover;
mod error;
mod transport;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;

pub use discover::discover;
pub use error::CdpError;
pub use transport::{CdpClient, CdpEvent, CdpEvents};
