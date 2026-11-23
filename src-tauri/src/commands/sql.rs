//! SQL commands (spec section 3 "Commands"; F21; S36). Thin: the logic is in `crate::sql`. Everything needs an
//! unlocked vault. Table pages come back as one value (at most 1,000 rows); streaming over a `Channel` arrives with
//! the query editor in D3, where results can be huge.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppResult;
use crate::dbs::history;
use crate::redis::console::history_enabled;
use crate::sql::pg::{PageRequest, SqlObject, SqlPage, SqlTableInfo};
use crate::sql::run::{pieces_for, RunMode, RunOpts, SqlCatalog, SqlRunEvent};
use crate::sql::values::SqlCell;
use crate::sql::{DbEvent, Emit};
use crate::ssh::PromptAnswer;
use crate::state::AppState;

#[derive(Serialize, Type)]
pub struct DbOpened {
    pub conn_id: String,
}

/// Opens a SQL tab's connection. Returns at once; the steps, the host's questions and the result arrive on `events`.
#[tauri::command]
#[specta::specta]
pub async fn db_open(
    state: State<'_, AppState>,
    db_id: String,
    events: Channel<DbEvent>,
) -> AppResult<DbOpened> {
    state.require_unlocked()?;
    let emit: Emit = Arc::new(move |e| {
        let _ = events.send(e);
    });
    let conn_id = state.sql.open(state.store(), db_id, emit)?;
    Ok(DbOpened { conn_id })
}

/// The answer to a host's question asked while a SQL tab opens (host key, password, passphrase, 2FA).
#[tauri::command]
#[specta::specta]
pub fn db_respond(
    state: State<'_, AppState>,
    conn_id: String,
    prompt_id: String,
    answer: PromptAnswer,
) -> AppResult<()> {
    state.sql.respond(&conn_id, &prompt_id, answer)
}

/// Closes the tab's connection, its questions and the SSH connection it ran through.
#[tauri::command]
#[specta::specta]
pub fn db_close(state: State<'_, AppState>, conn_id: String) {
    state.sql.close(&conn_id);
}

/// The lazy object tree: `[]` lists schemas, `[schema]` lists its tables and views.
#[tauri::command]
#[specta::specta]
pub async fn sql_objects(
    state: State<'_, AppState>,
    conn_id: String,
    path: Vec<String>,
) -> AppResult<Vec<SqlObject>> {
    state.require_unlocked()?;
    state.sql.pg(&conn_id).await?.objects(&path).await
}

/// A table's columns, primary key and foreign keys (both directions).
#[tauri::command]
#[specta::specta]
pub async fn sql_table_info(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
) -> AppResult<SqlTableInfo> {
    state.require_unlocked()?;
    state
        .sql
        .pg(&conn_id)
        .await?
        .table_info(&schema, &table)
        .await
}

/// One page of a table: filter, sort, columns and the cursor of the page before.
#[tauri::command]
#[specta::specta]
pub async fn sql_table_page(
    state: State<'_, AppState>,
    conn_id: String,
    request: PageRequest,
) -> AppResult<SqlPage> {
    state.require_unlocked()?;
    state.sql.pg(&conn_id).await?.table_page(&request).await
}

/// "Count exactly": `count(*)` with the same filter.
#[tauri::command]
#[specta::specta]
pub async fn sql_count(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
    filter: String,
) -> AppResult<f64> {
    state.require_unlocked()?;
    let n = state
        .sql
        .pg(&conn_id)
        .await?
        .count(&schema, &table, &filter)
        .await?;
    // A count far below 2^53, so a JS number holds it exactly.
    Ok(n as f64)
}

#[derive(Deserialize, Type)]
pub struct KeyPart {
    pub column: String,
    pub value: String,
}

/// One whole cell (the page cut it), for the cell panel.
#[tauri::command]
#[specta::specta]
pub async fn sql_cell(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
    key: Vec<KeyPart>,
    column: String,
) -> AppResult<SqlCell> {
    state.require_unlocked()?;
    let key: Vec<(String, String)> = key.into_iter().map(|k| (k.column, k.value)).collect();
    state
        .sql
        .pg(&conn_id)
        .await?
        .cell(&schema, &table, &key, &column)
        .await
}

/// Runs the editor's text: the statement at the cursor, the selection, or the whole script. Results stream over
/// `events` as they come (rows in batches); the command itself answers when the run is over. D3 runs reads only.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn sql_run(
    state: State<'_, AppState>,
    conn_id: String,
    editor_id: String,
    text: String,
    mode: RunMode,
    cursor: Option<u32>,
    limit: u32,
    timeout_ms: u32,
    events: Channel<SqlRunEvent>,
) -> AppResult<()> {
    state.require_unlocked()?;
    let pieces = pieces_for(&text, mode, cursor)?;
    let editor = state.sql.editor(&conn_id, &editor_id).await?;
    editor.stop.store(false, std::sync::atomic::Ordering::SeqCst);
    // What was typed is remembered once it is known to be allowed to run; the password filter is inside.
    let db_id = state.sql.db_id(&conn_id)?;
    let ran = pieces
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(";\n");
    let refused = pieces
        .iter()
        .any(|p| crate::sql::classify::classify(&p.text) != crate::sql::classify::Class::Read);
    if !refused {
        let _ = state
            .store()
            .with_db_session(move |c, s| {
                if history_enabled(c) {
                    history::remember(c, s, &db_id, "sql", &ran)?;
                }
                Ok(())
            })
            .await;
    }
    let opts = RunOpts {
        limit: limit.min(1_000_000),
        timeout_ms: timeout_ms.clamp(1000, 3_600_000),
    };
    editor.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let out = editor
        .conn
        .run_script(&pieces, &opts, &editor.stop, &|e| {
            let _ = events.send(e);
        })
        .await;
    editor.running.store(false, std::sync::atomic::Ordering::SeqCst);
    out
}

/// Stop: the running statement is cancelled by the server (a second connection), and no further statement starts.
#[tauri::command]
#[specta::specta]
pub async fn sql_cancel(
    state: State<'_, AppState>,
    conn_id: String,
    editor_id: String,
) -> AppResult<()> {
    state.require_unlocked()?;
    let editor = state.sql.editor(&conn_id, &editor_id).await?;
    // Nothing running: a cancel now would land on whatever the connection does next.
    if !editor.running.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    editor.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    editor.conn.cancel_running().await
}

/// The editor tab closed.
#[tauri::command]
#[specta::specta]
pub async fn sql_editor_close(
    state: State<'_, AppState>,
    conn_id: String,
    editor_id: String,
) -> AppResult<()> {
    state.sql.close_editor(&conn_id, &editor_id).await;
    Ok(())
}

/// Tables and columns for completion; `refresh` reads them again (after a schema change).
#[tauri::command]
#[specta::specta]
pub async fn sql_catalog(
    state: State<'_, AppState>,
    conn_id: String,
    refresh: bool,
) -> AppResult<SqlCatalog> {
    state.require_unlocked()?;
    let c = state.sql.pg(&conn_id).await?.catalog(refresh).await?;
    Ok((*c).clone())
}

/// Lays the statements out on lines. Comments and anything unreadable are left as typed.
#[tauri::command]
#[specta::specta]
pub fn sql_format(text: String) -> String {
    crate::sql::classify::format(&text)
}

/// What was run on this database, newest first.
#[tauri::command]
#[specta::specta]
pub async fn sql_history(state: State<'_, AppState>, db_id: String) -> AppResult<Vec<String>> {
    state.require_unlocked()?;
    state
        .store()
        .with_db_session(move |c, s| history::list(c, s, &db_id, "sql"))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn sql_history_clear(state: State<'_, AppState>, db_id: String) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .store()
        .with_db_session(move |c, s| history::clear(c, s, &db_id, "sql"))
        .await
}
