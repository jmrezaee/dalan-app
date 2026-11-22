//! Reaching a database of any engine through its route (direct, or through a saved host). D1 only proves the route
//! works (the "Test connection" of S35 for engines whose client arrives later); each engine's own client replaces
//! the last step when its phase lands (PostgreSQL in D2, MySQL and MariaDB in D7, MongoDB in D8).

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use specta::Type;

use super::transport::{self, Target};
use super::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::conn::{sign_in_to_host, Emit};
use crate::redis::store::RedisRoute;
use crate::ssh::Prompts;
use crate::state::Store;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DbTestResult {
    pub engine: Engine,
    /// Milliseconds to open the route and reach the port.
    pub latency_ms: u32,
    /// What the test proved, in plain words.
    pub detail: String,
}

/// Opens the route to the first address of `target` and closes it again. A host that needs an answer from the user
/// (a new host key, a password) is not asked here: the test says to open a terminal on it once.
pub async fn reach(
    store: &Store,
    engine: Engine,
    name: &str,
    route: RedisRoute,
    host_id: Option<&str>,
    target: &Target,
) -> AppResult<DbTestResult> {
    let started = Instant::now();
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let detail = match (route, host_id) {
        (RedisRoute::Host, Some(host_id)) => {
            let ssh = sign_in_to_host(store, host_id, 1, &prompts, &quiet)
                .await
                .map_err(|e| {
                    if e.code == ErrorCode::AuthFailed {
                        AppError::new(
                            ErrorCode::AuthFailed,
                            format!("The host for {name} needs an answer from you (a new host key or a password). Open a terminal on it once, then test again."),
                        )
                    } else {
                        e
                    }
                })?;
            let stream = transport::through_host_for(engine, &ssh, target).await?;
            drop(stream);
            format!(
                "Reached {} at {} from {}.",
                engine.label(),
                target.display(),
                ssh.label
            )
        }
        _ => {
            let stream = transport::direct_for(engine, target).await?;
            drop(stream);
            format!("Reached {} at {}.", engine.label(), target.display())
        }
    };
    Ok(DbTestResult {
        engine,
        latency_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        detail,
    })
}
