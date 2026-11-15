//! Redis commands (replica/redis/spec.md section 3; F13; S26, S27, S33). Thin: the logic is in `crate::redis`.
//! Everything needs an unlocked vault. A password goes UI -> Rust once and is never returned (`has_password`).

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use zeroize::Zeroizing;

use crate::error::AppResult;
use crate::redis::conn::{self, Config, RedisDbKeys, RedisTestResult};
use crate::redis::store::{self, RedisDatabase, RedisDatabaseInput};
use crate::redis::transport::Target;
use crate::redis::RedisEvent;
use crate::ssh::PromptAnswer;
use crate::state::AppState;

/// S26: the saved databases, by name.
#[tauri::command]
#[specta::specta]
pub async fn redis_db_list(state: State<'_, AppState>) -> AppResult<Vec<RedisDatabase>> {
    state
        .with_db_session(|c, s| store::list(c, &s.vault_id))
        .await
}

/// S27: creates or updates a database. The password, if any, goes straight into the vault.
#[tauri::command]
#[specta::specta]
pub async fn redis_db_save(
    state: State<'_, AppState>,
    input: RedisDatabaseInput,
) -> AppResult<RedisDatabase> {
    state
        .with_db_session(move |c, s| store::save(c, s, input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn redis_db_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| store::delete(c, s, &id))
        .await
}

/// S27 "Test connection": connects with the form's values, reads version, mode and latency, then closes.
/// An empty password on a saved database means "the saved one".
#[tauri::command]
#[specta::specta]
pub async fn redis_db_test(
    state: State<'_, AppState>,
    input: RedisDatabaseInput,
) -> AppResult<RedisTestResult> {
    state.require_unlocked()?;
    store::validate(&input)?;
    let typed = input
        .password
        .clone()
        .filter(|p| !p.is_empty())
        .map(Zeroizing::new);
    let saved_id = input.id.clone();
    let clear = input.clear_password;
    let saved = match (&typed, saved_id, clear) {
        (None, Some(id), false) => {
            state
                .with_db_session(move |c, s| store::password(c, s, &id))
                .await?
        }
        _ => None,
    };
    let cfg = Config {
        name: input.name.trim().to_string(),
        route: input.route,
        host_id: input.host_id.clone().filter(|h| !h.is_empty()),
        kind: input.kind,
        target: Target {
            address: input.address.trim().to_string(),
            port: input.port,
            tls: input.tls,
            tls_server_name: input.tls_server_name.clone(),
        },
        username: input.username.clone().filter(|u| !u.trim().is_empty()),
        password: typed.or(saved),
        db: input.default_db,
    };
    conn::test(&state.store(), cfg).await
}

#[derive(Serialize, Type)]
pub struct RedisOpened {
    pub conn_id: String,
}

/// Opens a Redis tab's connection. Returns at once; the steps, the host's questions and the result arrive on `events`.
#[tauri::command]
#[specta::specta]
pub async fn redis_open(
    state: State<'_, AppState>,
    db_id: String,
    events: Channel<RedisEvent>,
) -> AppResult<RedisOpened> {
    state.require_unlocked()?;
    let emit: conn::Emit = Arc::new(move |e| {
        let _ = events.send(e);
    });
    let conn_id = state.redis.open(state.store(), db_id, emit)?;
    Ok(RedisOpened { conn_id })
}

/// The answer to a host's question asked while a Redis tab opens (host key, password, passphrase, 2FA).
#[tauri::command]
#[specta::specta]
pub fn redis_respond(
    state: State<'_, AppState>,
    conn_id: String,
    prompt_id: String,
    answer: PromptAnswer,
) -> AppResult<()> {
    state.redis.respond(&conn_id, &prompt_id, answer)
}

/// The logical database picker (`db0`, `db1`, ...): switches and answers with the key count.
#[tauri::command]
#[specta::specta]
pub async fn redis_select(
    state: State<'_, AppState>,
    conn_id: String,
    index: u32,
) -> AppResult<RedisDbKeys> {
    state.require_unlocked()?;
    state.redis.select(&conn_id, index).await
}

/// Closes the tab's connection, its questions and the SSH connection it ran through.
#[tauri::command]
#[specta::specta]
pub fn redis_close(state: State<'_, AppState>, conn_id: String) {
    state.redis.close(&conn_id);
}
