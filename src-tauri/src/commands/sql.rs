//! SQL commands (spec section 3 "Commands"; F21; S36). Thin: the logic is in `crate::sql`. Everything needs an
//! unlocked vault. Table pages come back as one value (at most 1,000 rows); streaming over a `Channel` arrives with
//! the query editor in D3, where results can be huge.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppResult;
use crate::sql::pg::{PageRequest, SqlObject, SqlPage, SqlTableInfo};
use crate::sql::values::Cell;
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
) -> AppResult<Cell> {
    state.require_unlocked()?;
    let key: Vec<(String, String)> = key.into_iter().map(|k| (k.column, k.value)).collect();
    state
        .sql
        .pg(&conn_id)
        .await?
        .cell(&schema, &table, &key, &column)
        .await
}
