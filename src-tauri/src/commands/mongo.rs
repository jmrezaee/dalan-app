//! MongoDB commands (S44; F27). Thin: the logic is in `crate::mongo`. Everything needs an unlocked vault. A write to
//! a document is prepared (and held, bound to this tab's connection) and then committed once, after the database's
//! mode is asked again: the same gate shape as SQL.

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::dbs::gate::{self, decide, Decision};
use crate::error::AppResult;
use crate::mongo::docs::{self, MgCount, MgDocOp, MgFindRequest, MgNode, MgPage, MgWrite, Parsed};
use crate::sql;
use crate::sql::classify::Class;
use crate::state::AppState;

/// `[]` lists databases, `[db]` its collections and views.
#[tauri::command]
#[specta::specta]
pub async fn mg_tree(state: State<'_, AppState>, conn_id: String, path: Vec<String>) -> AppResult<Vec<MgNode>> {
    state.require_unlocked()?;
    docs::tree(&*state.sql.mongo(&conn_id).await?, &path).await
}

/// One page of documents.
#[tauri::command]
#[specta::specta]
pub async fn mg_find(state: State<'_, AppState>, conn_id: String, request: MgFindRequest) -> AppResult<MgPage> {
    state.require_unlocked()?;
    docs::find(&*state.sql.mongo(&conn_id).await?, &request).await
}

/// The collection's size (an estimate with no filter) or the exact number of matches.
#[tauri::command]
#[specta::specta]
pub async fn mg_count(state: State<'_, AppState>, conn_id: String, db: String, coll: String, filter: String) -> AppResult<MgCount> {
    state.require_unlocked()?;
    docs::count(&*state.sql.mongo(&conn_id).await?, &db, &coll, &filter).await
}

/// One whole document by `_id` in canonical extended JSON, for the editor.
#[tauri::command]
#[specta::specta]
pub async fn mg_doc_get(state: State<'_, AppState>, conn_id: String, db: String, coll: String, id: String) -> AppResult<String> {
    state.require_unlocked()?;
    docs::get(&*state.sql.mongo(&conn_id).await?, &db, &coll, &id).await
}

#[derive(Serialize, Type)]
pub struct MgPending {
    pub id: String,
    pub db_name: String,
    pub environment: String,
    /// What will happen, in words.
    pub summary: String,
    /// The document as it is now (canonical JSON), for a replace or delete: "Show sample".
    pub sample: Option<String>,
}

/// Parses and holds one document write. A Read-only database refuses here, before anything is held.
#[tauri::command]
#[specta::specta]
pub async fn mg_doc_prepare(state: State<'_, AppState>, conn_id: String, db: String, coll: String, op: MgDocOp) -> AppResult<MgPending> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if decide(target.mode, Class::Write) == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let write = MgWrite { db, coll, op: Parsed::parse(&op)? };
    let conn = state.sql.mongo(&conn_id).await?;
    let sample = docs::current(&conn, &write).await?.map(|d| docs::canonical(&d));
    let summary = docs::describe(&write);
    let id = state.sql.mongo_pending.put(&conn_id, Class::Write, write)?;
    Ok(MgPending { id, db_name: target.name, environment: target.environment, summary, sample })
}

/// Runs what `mg_doc_prepare` held, once, after the mode is asked again.
#[tauri::command]
#[specta::specta]
pub async fn mg_doc_commit(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<u32> {
    state.require_unlocked()?;
    let (write, class) = state.sql.mongo_pending.take(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, None)?;
    docs::run(&*state.sql.mongo(&conn_id).await?, &write).await
}

#[tauri::command]
#[specta::specta]
pub fn mg_doc_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.mongo_pending.take(&conn_id, &pending_id);
    Ok(())
}
