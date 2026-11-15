//! Redis inside Dalan (replica/redis/spec.md). Everything Redis lives here, in Rust: the webview only sees typed
//! results (hard rule 2), and passwords never leave this side (hard rule 1).
//!
//! - `store`: the saved databases (S26, S27).
//! - `transport`: how a connection reaches its server, directly or through a saved host, with TLS inside.
//! - `conn`: one live connection per Redis tab: sign-in, server info, keep-alive, reconnect.
//! - `browse`: the browser's read side: `SCAN`, key details and values in pages, as bytes.
//! - `gate`: the write gate: one function classifies every command and decides by the database's mode.
//! - `write`: the editors' changes: prepare, confirm, commit once.
//!
//! - `console`: a typed line through the gate, replies printed in Rust, `MULTI` queued here, encrypted history.
//! - `server`: `INFO` and the slow log for the Server view.
//!
//! - `pubsub`: listening on its own connection, messages batched for the UI.
//! - `bulk`: preview, typed confirmation, batches of 500, stop.
//!
//! Cluster and Sentinel arrive in R6.

pub mod browse;
pub mod bulk;
pub mod conn;
pub mod console;
pub mod gate;
pub mod pubsub;
pub mod server;
pub mod store;
pub mod transport;
pub mod write;

pub use conn::{RedisEvent, RedisManager};

#[cfg(test)]
mod tests;
