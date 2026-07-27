//! The resident service: one logged-in session, controlled over a socket.

/// Generates `docs/openrpc.json` from the wire types and checks it is current.
/// Test-only: the published document is served at runtime by `include_str!`.
#[cfg(test)]
mod openrpc;
pub mod proto;
