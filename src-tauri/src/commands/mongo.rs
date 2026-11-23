//! MongoDB commands (S44; F27). Thin: the logic is in `crate::mongo`. Everything needs an unlocked vault. A write to
//! a document is prepared (and held, bound to this tab's connection) and then committed once, after the database's
//! mode is asked again: the same gate shape as SQL.

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::dbs::gate::{self, decide, Decision};
use crate::error::AppResult;
use crate::mongo::aggregate::{self, CodeLanguage, MgAggRequest, MgAggResult, MgStage};
use crate::mongo::schema::{self, MgIndex, MgIndexOp, MgSchema, MgValidation, MgValidationCheck};
use crate::mongo::docs::{self, MgCount, MgDocOp, MgFindRequest, MgNode, MgPage, MgWrite, Parsed};
use crate::sql;
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
    /// `confirm`, or `typed` when the database's name must be typed first (a dangerous change).
    pub level: String,
}

/// Holds a parsed write under the gate: refused in a Read-only database, otherwise kept for one confirmation.
async fn hold(state: &AppState, conn_id: &str, write: MgWrite) -> AppResult<MgPending> {
    let db_id = state.sql.db_id(conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    let class = write.op.class();
    let decision = decide(target.mode, class);
    if decision == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let conn = state.sql.mongo(conn_id).await?;
    let sample = docs::current(&conn, &write).await?.map(|d| docs::canonical(&d));
    let summary = docs::describe(&write);
    let id = state.sql.mongo_pending.put(conn_id, class, write)?;
    Ok(MgPending { id, db_name: target.name, environment: target.environment, summary, sample, level: if decision == Decision::Typed { "typed".into() } else { "confirm".into() } })
}

/// Parses and holds one document write. A Read-only database refuses here, before anything is held.
#[tauri::command]
#[specta::specta]
pub async fn mg_doc_prepare(state: State<'_, AppState>, conn_id: String, db: String, coll: String, op: MgDocOp) -> AppResult<MgPending> {
    state.require_unlocked()?;
    hold(&state, &conn_id, MgWrite { db, coll, op: Parsed::parse(&op)? }).await
}

/// Runs what `mg_doc_prepare` held, once, after the mode is asked again.
#[tauri::command]
#[specta::specta]
pub async fn mg_doc_commit(state: State<'_, AppState>, conn_id: String, pending_id: String, typed_name: Option<String>) -> AppResult<u32> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    // The typed name is checked before the held change is used up: a typo keeps the dialog open.
    let class = state.sql.mongo_pending.peek_class(&conn_id, &pending_id)?;
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    let (write, _) = state.sql.mongo_pending.take(&conn_id, &pending_id)?;
    // The mode is asked again after the hold is used up: it may have changed while the dialog was open.
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    docs::run(&*state.sql.mongo(&conn_id).await?, &write).await
}

#[tauri::command]
#[specta::specta]
pub fn mg_doc_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.mongo_pending.take(&conn_id, &pending_id);
    Ok(())
}

// ---- Aggregation (S45)

/// The output of the pipeline up to a stage. Reads only: a `$out` or `$merge` ends the run before it.
#[tauri::command]
#[specta::specta]
pub async fn mg_aggregate(state: State<'_, AppState>, conn_id: String, request: MgAggRequest) -> AppResult<MgAggResult> {
    state.require_unlocked()?;
    aggregate::run(&*state.sql.mongo(&conn_id).await?, &request).await
}

/// Holds a pipeline that ends in `$out` or `$merge` (a write) for its confirmation.
#[tauri::command]
#[specta::specta]
pub async fn mg_agg_prepare(state: State<'_, AppState>, conn_id: String, request: MgAggRequest) -> AppResult<MgPending> {
    state.require_unlocked()?;
    let op = aggregate::write_command(&request)?;
    hold(&state, &conn_id, MgWrite { db: request.db, coll: request.coll, op }).await
}

/// Holds `createView` for the pipeline.
#[tauri::command]
#[specta::specta]
pub async fn mg_view_prepare(state: State<'_, AppState>, conn_id: String, db: String, coll: String, name: String, stages: Vec<MgStage>) -> AppResult<MgPending> {
    state.require_unlocked()?;
    let op = aggregate::view_command(&db, &coll, &name, &stages)?;
    hold(&state, &conn_id, MgWrite { db, coll, op }).await
}

/// The pipeline as code to paste (mongosh, Node.js, Python) or as JSON.
#[tauri::command]
#[specta::specta]
pub fn mg_pipeline_code(coll: String, stages: Vec<MgStage>, language: CodeLanguage) -> AppResult<String> {
    aggregate::code(&coll, &stages, language)
}

// ---- Schema, indexes, validation (S46)

#[tauri::command]
#[specta::specta]
pub async fn mg_schema(state: State<'_, AppState>, conn_id: String, db: String, coll: String, sample: u32) -> AppResult<MgSchema> {
    state.require_unlocked()?;
    schema::sample(&*state.sql.mongo(&conn_id).await?, &db, &coll, sample).await
}

#[tauri::command]
#[specta::specta]
pub async fn mg_indexes(state: State<'_, AppState>, conn_id: String, db: String, coll: String) -> AppResult<Vec<MgIndex>> {
    state.require_unlocked()?;
    schema::indexes(&*state.sql.mongo(&conn_id).await?, &db, &coll).await
}

/// Holds an index change: creating asks, dropping asks for the database's name.
#[tauri::command]
#[specta::specta]
pub async fn mg_index_prepare(state: State<'_, AppState>, conn_id: String, db: String, coll: String, op: MgIndexOp) -> AppResult<MgPending> {
    state.require_unlocked()?;
    let held = schema::index_command(&coll, &op)?;
    hold(&state, &conn_id, MgWrite { db, coll, op: held }).await
}

#[tauri::command]
#[specta::specta]
pub async fn mg_validation(state: State<'_, AppState>, conn_id: String, db: String, coll: String) -> AppResult<MgValidation> {
    state.require_unlocked()?;
    schema::validation(&*state.sql.mongo(&conn_id).await?, &db, &coll).await
}

/// How many existing documents break these rules.
#[tauri::command]
#[specta::specta]
pub async fn mg_validation_check(state: State<'_, AppState>, conn_id: String, db: String, coll: String, validator: String) -> AppResult<MgValidationCheck> {
    state.require_unlocked()?;
    schema::check(&*state.sql.mongo(&conn_id).await?, &db, &coll, &validator).await
}

#[tauri::command]
#[specta::specta]
pub async fn mg_validation_prepare(state: State<'_, AppState>, conn_id: String, db: String, coll: String, validator: String, level: String, action: String) -> AppResult<MgPending> {
    state.require_unlocked()?;
    let op = schema::validation_command(&coll, &validator, &level, &action)?;
    hold(&state, &conn_id, MgWrite { db, coll, op }).await
}
