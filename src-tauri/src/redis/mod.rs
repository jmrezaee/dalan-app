//! Redis inside Dalan (replica/redis/spec.md). Everything Redis lives here, in Rust: the webview only sees typed
//! results (hard rule 2), and passwords never leave this side (hard rule 1).
//!
//! - `store`: the saved databases (S26, S27).
//! - `transport`: how a connection reaches its server, directly or through a saved host, with TLS inside.
//! - `conn`: one live connection per Redis tab: sign-in, server info, keep-alive, reconnect.
//! - `browse`: the browser's read side: `SCAN`, key details and values in pages, as bytes.
//!
//! The write gate, the console, Pub/Sub, bulk actions, Cluster and Sentinel arrive in R2 to R6.

pub mod browse;
pub mod conn;
pub mod store;
pub mod transport;

pub use conn::{RedisEvent, RedisManager};

#[cfg(test)]
mod tests;
