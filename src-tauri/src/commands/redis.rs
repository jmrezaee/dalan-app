//! Redis commands (replica/redis/spec.md section 3; F13; S26, S27, S33). Thin: the logic is in `crate::redis`.
//! Everything needs an unlocked vault. A password goes UI -> Rust once and is never returned (`has_password`).

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::browse::{self, RedisBytes, RedisKeyInfo, RedisScanPage, RedisValuePage};
use crate::redis::bulk::{
    self, RedisBulkAction, RedisBulkEvent, RedisBulkPreview, RedisBulkResult,
};
use crate::redis::cluster::RedisNode;
use crate::redis::conn::{self, Config, RedisDbKeys, RedisTestResult};
use crate::redis::console::{self, RedisCommandInfo, RedisConsoleOutcome, RedisConsoleReply};
use crate::redis::gate;
use crate::redis::pubsub::{RedisSubEvent, SubEmit};
use crate::redis::server::{self, RedisServerStats, RedisSlowEntry};
use crate::redis::store::{
    self, RedisDatabase, RedisDatabaseInput, RedisEnvironment, RedisWriteMode,
};
use crate::redis::transport::Target;
use crate::redis::write::{self, RedisWriteOp, RedisWriteOutcome};
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

/// Checks the master password for a change that makes a production database less protected. Five wrong tries
/// start the same cooldown as unlocking. The password is zeroized when it is dropped.
pub(crate) async fn check_master(state: &AppState, password: Option<String>) -> AppResult<()> {
    let password = password.map(Zeroizing::new);
    state.check_cooldown()?;
    let Some(pw) = password.filter(|p| !p.is_empty()) else {
        return Err(AppError::new(
            ErrorCode::WrongPassword,
            "Type the master password to make a production database less protected.",
        ));
    };
    // `unlock` derives the key and checks it; the session it builds is dropped (and zeroized) right away.
    match state
        .with_db(move |c| crate::vault::unlock(c, &pw).map(|_| ()))
        .await
    {
        Ok(()) => {
            state.reset_failures();
            Ok(())
        }
        Err(e) => {
            if e.code == ErrorCode::WrongPassword {
                state.record_failure();
            }
            Err(e)
        }
    }
}

/// S27: creates or updates a database. The password, if any, goes straight into the vault.
/// A save that makes a production database less protected (a looser write mode, or no longer marked
/// production) needs the master password, checked here.
#[tauri::command]
#[specta::specta]
pub async fn redis_db_save(
    state: State<'_, AppState>,
    input: RedisDatabaseInput,
    master_password: Option<String>,
) -> AppResult<RedisDatabase> {
    state.require_unlocked()?;
    // Read the saved row first: only a save that lowers protection asks for (and checks) the password.
    let (lowers, input) = state
        .with_db_session(move |c, s| Ok((store::lowers_protection(c, &s.vault_id, &input)?, input)))
        .await?;
    if lowers {
        check_master(&state, master_password).await?;
        log::info!(
            "redis: {} was made less protected from the editor",
            input.name.trim()
        );
    }
    state
        .with_db_session(move |c, s| store::save_with_master(c, s, input, lowers))
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
        sentinel_master: input
            .sentinel_master
            .clone()
            .filter(|m| !m.trim().is_empty()),
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

/// F14: one page of keys (`SCAN`, never `KEYS`), each with its type and time left. `cursor` is `""` to start;
/// the page's own cursor continues, and `"0"` means the whole keyspace has been walked.
#[tauri::command]
#[specta::specta]
pub async fn redis_scan(
    state: State<'_, AppState>,
    conn_id: String,
    cursor: String,
    pattern: String,
    key_type: Option<String>,
    count: u32,
) -> AppResult<RedisScanPage> {
    state.require_unlocked()?;
    let mut conn = state.redis.handle(&conn_id).await?;
    browse::scan(&mut conn, &cursor, &pattern, key_type.as_deref(), count).await
}

/// The key header: type, time left, size, memory, encoding (and a stream's groups).
#[tauri::command]
#[specta::specta]
pub async fn redis_key_info(
    state: State<'_, AppState>,
    conn_id: String,
    key: RedisBytes,
) -> AppResult<RedisKeyInfo> {
    state.require_unlocked()?;
    let mut conn = state.redis.handle(&conn_id).await?;
    browse::key_info(&mut conn, &key).await
}

/// One page of the value for its type (500 items, or a string). `full` loads a big string whole.
#[tauri::command]
#[specta::specta]
pub async fn redis_value_page(
    state: State<'_, AppState>,
    conn_id: String,
    key: RedisBytes,
    cursor: String,
    full: bool,
) -> AppResult<RedisValuePage> {
    state.require_unlocked()?;
    let mut conn = state.redis.handle(&conn_id).await?;
    browse::value_page(&mut conn, &key, &cursor, full).await
}

/// The database a connection belongs to, as the write gate needs it. Read fresh for every write, so a mode
/// change applies to tabs that are already open.
async fn write_target(state: &AppState, w: &conn::Writer) -> AppResult<write::Target> {
    let id = w.db_id.clone();
    let db = state
        .with_db_session(move |c, s| store::get(c, &s.vault_id, &id))
        .await?;
    Ok(write::Target {
        name: db.name,
        mode: db.write_mode,
        replica: w.replica,
    })
}

/// F15, step one: an editor's change. In Open mode it runs; in Read-only it is refused; otherwise the answer says
/// what to confirm, and the exact command waits two minutes for `redis_write_commit`.
#[tauri::command]
#[specta::specta]
pub async fn redis_write_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    op: RedisWriteOp,
) -> AppResult<RedisWriteOutcome> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let target = write_target(&state, &w).await?;
    write::prepare(&mut w.conn, &w.table, &w.pending, &target, op).await
}

/// F15, step two: runs a prepared write, once. `typed_name` is what the person typed in the "type the database's
/// name" dialog; Rust compares it.
#[tauri::command]
#[specta::specta]
pub async fn redis_write_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
) -> AppResult<RedisWriteOutcome> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let target = write_target(&state, &w).await?;
    write::commit(
        &mut w.conn,
        &w.pending,
        &target,
        &pending_id,
        typed_name.as_deref(),
    )
    .await
}

/// Changes a database's write mode. Making a production database less protected needs the master password,
/// checked here (five wrong tries start the same cooldown as unlocking).
#[tauri::command]
#[specta::specta]
pub async fn redis_set_mode(
    state: State<'_, AppState>,
    db_id: String,
    mode: RedisWriteMode,
    master_password: Option<String>,
) -> AppResult<RedisDatabase> {
    state.require_unlocked()?;
    let id = db_id.clone();
    let db = state
        .with_db_session(move |c, s| store::get(c, &s.vault_id, &id))
        .await?;
    let loosens = gate::protection(mode) < gate::protection(db.write_mode);
    if loosens && db.environment == RedisEnvironment::Production {
        check_master(&state, master_password).await?;
    }
    log::info!("redis: write mode of {} is now {}", db.name, mode.as_str());
    state
        .with_db_session(move |c, s| store::set_mode(c, &s.vault_id, &db_id, mode))
        .await
}

/// F16: one console line. It is parsed, shown to the write gate, then run, refused, or held for "Send" / "Run
/// anyway" / the typed-name dialog. Lines that carry a password are never saved to history.
#[tauri::command]
#[specta::specta]
pub async fn redis_console_exec(
    state: State<'_, AppState>,
    conn_id: String,
    line: String,
) -> AppResult<RedisConsoleOutcome> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let target = write_target(&state, &w).await?;
    let (db_id, kept) = (w.db_id.clone(), line.clone());
    // History first: a line is worth finding again even when the server refused it. Failing to save is not an error.
    let _ = state
        .with_db_session(move |c, s| {
            if console::history_enabled(c) {
                console::remember(c, s, &db_id, &kept)?;
            }
            Ok(())
        })
        .await;
    let outcome = console::exec(
        &mut w.conn,
        &w.table,
        &w.pending,
        &w.console,
        &target,
        &line,
    )
    .await?;
    if let RedisConsoleOutcome::Reply { reply } = &outcome {
        if let (Some(index), false) = (reply.selected, reply.error) {
            state.redis.note_selected(&conn_id, index);
        }
    }
    Ok(outcome)
}

/// Runs what `redis_console_exec` held, once.
#[tauri::command]
#[specta::specta]
pub async fn redis_console_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
) -> AppResult<RedisConsoleReply> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let target = write_target(&state, &w).await?;
    let reply = console::commit(
        &mut w.conn,
        &w.pending,
        &target,
        &pending_id,
        typed_name.as_deref(),
    )
    .await?;
    if let (Some(index), false) = (reply.selected, reply.error) {
        state.redis.note_selected(&conn_id, index);
    }
    Ok(reply)
}

/// The database's console history, newest first (kept encrypted).
#[tauri::command]
#[specta::specta]
pub async fn redis_console_history(
    state: State<'_, AppState>,
    db_id: String,
) -> AppResult<Vec<String>> {
    state
        .with_db_session(move |c, s| console::history(c, s, &db_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn redis_console_history_clear(
    state: State<'_, AppState>,
    db_id: String,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| console::clear_history(c, s, &db_id))
        .await
}

/// The server's commands with their syntax, for Tab completion.
#[tauri::command]
#[specta::specta]
pub async fn redis_commands(
    state: State<'_, AppState>,
    conn_id: String,
) -> AppResult<Vec<RedisCommandInfo>> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    Ok(console::commands(&mut w.conn, &w.table).await)
}

/// S31: the tiles and "Keys by database", from `INFO`.
#[tauri::command]
#[specta::specta]
pub async fn redis_info(
    state: State<'_, AppState>,
    conn_id: String,
) -> AppResult<RedisServerStats> {
    state.require_unlocked()?;
    let mut conn = state.redis.handle(&conn_id).await?;
    server::info(&mut conn).await
}

/// S31: the server's own slow log, newest first.
#[tauri::command]
#[specta::specta]
pub async fn redis_slowlog(
    state: State<'_, AppState>,
    conn_id: String,
) -> AppResult<Vec<RedisSlowEntry>> {
    state.require_unlocked()?;
    let mut conn = state.redis.handle(&conn_id).await?;
    server::slowlog(&mut conn).await
}

/// F17: listens to a channel, or to a pattern such as `orders.*`. Listening uses its own connection through the
/// same route; messages arrive on `messages` in batches. Reading is allowed in every write mode.
#[tauri::command]
#[specta::specta]
pub async fn redis_sub_start(
    state: State<'_, AppState>,
    conn_id: String,
    name: RedisBytes,
    pattern: bool,
    messages: Channel<RedisSubEvent>,
) -> AppResult<()> {
    state.require_unlocked()?;
    let emit: SubEmit = Arc::new(move |e| {
        let _ = messages.send(e);
    });
    state.redis.sub_start(&conn_id, &name, pattern, emit).await
}

#[tauri::command]
#[specta::specta]
pub async fn redis_sub_stop(
    state: State<'_, AppState>,
    conn_id: String,
    name: RedisBytes,
    pattern: bool,
) -> AppResult<()> {
    state.redis.sub_stop(&conn_id, &name, pattern).await
}

/// F18, step one: walks the whole keyspace (`SCAN`) and answers with how many keys match and a sample of 20.
/// The matching keys stay in Rust for `redis_bulk_run`. `redis_bulk_stop` ends the walk.
#[tauri::command]
#[specta::specta]
pub async fn redis_bulk_preview(
    state: State<'_, AppState>,
    conn_id: String,
    pattern: String,
    key_type: Option<String>,
    progress: Channel<RedisBulkEvent>,
) -> AppResult<RedisBulkPreview> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let emit: bulk::BulkEmit = Arc::new(move |e| {
        let _ = progress.send(e);
    });
    bulk::preview(
        &mut w.conn,
        &w.bulk,
        pattern.trim(),
        key_type.as_deref(),
        &emit,
    )
    .await
}

/// F18, step two: runs the action on the previewed keys, 500 at a time. Refused in Read-only; in the other modes
/// `typed_name` must be the database's name. Runs once per preview.
#[tauri::command]
#[specta::specta]
pub async fn redis_bulk_run(
    state: State<'_, AppState>,
    conn_id: String,
    preview_id: String,
    action: RedisBulkAction,
    typed_name: Option<String>,
    progress: Channel<RedisBulkEvent>,
) -> AppResult<RedisBulkResult> {
    state.require_unlocked()?;
    let mut w = state.redis.writer(&conn_id).await?;
    let target = write_target(&state, &w).await?;
    let emit: bulk::BulkEmit = Arc::new(move |e| {
        let _ = progress.send(e);
    });
    // The gate first: refused in Read-only, and the typed name must be the database's.
    bulk::approve(&target, action, typed_name.as_deref())?;
    bulk::run(
        &mut w.conn,
        &w.table,
        &w.bulk,
        &target,
        &preview_id,
        action,
        &emit,
    )
    .await
}

/// "Stop": the scan or the run ends after its current batch. What is already done stays done.
#[tauri::command]
#[specta::specta]
pub fn redis_bulk_stop(state: State<'_, AppState>, conn_id: String) {
    state.redis.bulk_stop(&conn_id);
}

/// S31 "Nodes": a cluster's primaries and replicas with their slots, or a Sentinel set (primary, replicas,
/// sentinels). Empty for a single server.
#[tauri::command]
#[specta::specta]
pub async fn redis_nodes(state: State<'_, AppState>, conn_id: String) -> AppResult<Vec<RedisNode>> {
    state.require_unlocked()?;
    state.redis.nodes(&conn_id).await
}
