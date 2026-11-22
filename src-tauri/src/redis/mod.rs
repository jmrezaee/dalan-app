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
//! - `cluster`: the slot map, key hashing, one connection per node, `MOVED` / `ASK`; `Conn` is one server or a cluster.
//! - `sentinel`: asking the sentinels where the primary is.

pub mod browse;
pub mod bulk;
pub mod cluster;
pub mod conn;
pub mod console;
pub mod gate;
pub mod pubsub;
pub mod sentinel;
pub mod server;
pub mod store;
// The transport is shared by every engine now (D1); Redis keeps this path for its own code.
pub use crate::dbs::transport;
pub mod write;

pub use conn::{RedisEvent, RedisManager};

#[cfg(test)]
mod tests;
