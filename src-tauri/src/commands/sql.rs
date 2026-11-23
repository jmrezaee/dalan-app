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
use crate::dbs::gate::{self, decide, strongest, Decision};
use crate::redis::store::RedisWriteMode;
use crate::sql::classify::{classify, Class, Piece};
use crate::sql::run::{pieces_for, HeldRun, RunMode, RunOpts, SqlCatalog, SqlRunEvent, TxnState};
use crate::sql;
use crate::sql::diagram::Diagram;
use crate::sql::explain::ExplainPlan;
use crate::sql::grid::GridEdits;
use crate::sql::structure::{StructureOp, TableStructure};
use tauri_plugin_dialog::DialogExt;
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

#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PendingLevel {
    Confirm,
    /// The database's exact name must be typed.
    Typed,
}

#[derive(Serialize, Type)]
pub struct PendingStatement {
    pub index: u32,
    pub text: String,
    pub class: Class,
    /// What the server says it would do ("This will change 3 rows."), when it could be asked.
    pub note: Option<String>,
}

/// A script that needs the person's answer first. `sql_run_commit` runs it, once.
#[derive(Serialize, Type)]
pub struct PendingRun {
    pub id: String,
    pub level: PendingLevel,
    pub db_name: String,
    pub environment: String,
    pub statements: Vec<PendingStatement>,
}

#[derive(Serialize, Type)]
pub struct SqlRunReply {
    /// `None`: the script ran (or was refused); the results arrived on `events`.
    pub pending: Option<PendingRun>,
}

/// Runs one held or allowed script and remembers it in history.
#[allow(clippy::too_many_arguments)]
async fn execute(
    state: &AppState,
    conn_id: &str,
    editor_id: &str,
    pieces: Vec<Piece>,
    limit: u32,
    timeout_ms: u32,
    allow_write: bool,
    events: &Channel<SqlRunEvent>,
) -> AppResult<()> {
    let editor = state.sql.editor(conn_id, editor_id).await?;
    editor.stop.store(false, std::sync::atomic::Ordering::SeqCst);
    let db_id = state.sql.db_id(conn_id)?;
    let ran = pieces.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(";\n");
    let _ = state
        .store()
        .with_db_session(move |c, s| {
            if history_enabled(c) {
                history::remember(c, s, &db_id, "sql", &ran)?;
            }
            Ok(())
        })
        .await;
    let opts = RunOpts { limit: limit.min(1_000_000), timeout_ms: timeout_ms.clamp(1000, 3_600_000) };
    editor.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let out = editor
        .conn
        .run_script(&pieces, &opts, &editor.stop, allow_write, &|e| {
            let _ = events.send(e);
        })
        .await;
    editor.running.store(false, std::sync::atomic::Ordering::SeqCst);
    out
}

/// Runs the editor's text: the statement at the cursor, the selection, or the whole script. Reads run at once and
/// stream their results over `events`. Anything else goes through the gate (one function, in Rust): refused in a
/// Read-only database, held for a confirmation (or the typed name) otherwise, run when the mode is Open.
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
) -> AppResult<SqlRunReply> {
    state.require_unlocked()?;
    let pieces = pieces_for(&text, mode, cursor)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if let Some(i) = pieces.iter().position(|p| sql::classify::is_txn_control(&p.text)) {
        let _ = events.send(SqlRunEvent::Refused {
            index: i as u32,
            class: Class::Write,
            message: "Transactions are started and ended with the Manual / Auto-commit button and the Commit and Roll back buttons, not with statements.".into(),
        });
        let _ = events.send(SqlRunEvent::Finished { elapsed_ms: 0 });
        return Ok(SqlRunReply { pending: None });
    }
    let classes: Vec<Class> = pieces.iter().map(|p| classify(&p.text)).collect();
    let decision = classes
        .iter()
        .fold(Decision::Run, |d, c| strongest(d, decide(target.mode, *c)));
    let class = classes.iter().copied().max().unwrap_or(Class::Read);
    match decision {
        Decision::Refuse => {
            let i = classes.iter().position(|c| *c != Class::Read).unwrap_or(0);
            let _ = events.send(SqlRunEvent::Refused {
                index: i as u32,
                class: classes[i],
                message: gate::read_only_error(&target.name).message,
            });
            let _ = events.send(SqlRunEvent::Finished { elapsed_ms: 0 });
            Ok(SqlRunReply { pending: None })
        }
        Decision::Run => {
            execute(&state, &conn_id, &editor_id, pieces, limit, timeout_ms, class != Class::Read, &events).await?;
            Ok(SqlRunReply { pending: None })
        }
        Decision::Confirm | Decision::Typed => {
            let editor = state.sql.editor(&conn_id, &editor_id).await?;
            let notes = editor.conn.preview(&pieces).await?;
            let statements = pieces
                .iter()
                .enumerate()
                .map(|(i, p)| PendingStatement { index: i as u32, text: p.text.clone(), class: classes[i], note: notes[i].clone() })
                .collect();
            let id = state.sql.pending.put(&conn_id, class, HeldRun { pieces, editor_id })?;
            Ok(SqlRunReply {
                pending: Some(PendingRun {
                    id,
                    level: if decision == Decision::Typed { PendingLevel::Typed } else { PendingLevel::Confirm },
                    db_name: target.name,
                    environment: target.environment,
                    statements,
                }),
            })
        }
    }
}

/// The answer to a held script: runs exactly what `sql_run` held, once, after the mode is asked again and (for the
/// dangerous class) the database's name is checked.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn sql_run_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    limit: u32,
    timeout_ms: u32,
    events: Channel<SqlRunEvent>,
) -> AppResult<()> {
    state.require_unlocked()?;
    // Checked before the id is used up: a mistyped name leaves the dialog open to try again.
    let class = state.sql.pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    let (held, _) = state.sql.pending.take(&conn_id, &pending_id)?;
    execute(&state, &conn_id, &held.editor_id, held.pieces, limit, timeout_ms, true, &events).await
}

/// "Cancel" on the dialog: the held script is dropped.
#[tauri::command]
#[specta::specta]
pub fn sql_run_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.pending.take(&conn_id, &pending_id);
    Ok(())
}

#[derive(Debug, Clone, Copy, serde::Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TxnAction {
    Begin,
    Commit,
    Rollback,
    State,
}

/// The editor's manual transaction (S37's bar). Beginning one needs a database that allows writes.
#[tauri::command]
#[specta::specta]
pub async fn sql_txn(
    state: State<'_, AppState>,
    conn_id: String,
    editor_id: String,
    action: TxnAction,
) -> AppResult<TxnState> {
    state.require_unlocked()?;
    if matches!(action, TxnAction::Begin) {
        let db_id = state.sql.db_id(&conn_id)?;
        let target = sql::gate_target(&state.store(), &db_id).await?;
        if target.mode == RedisWriteMode::ReadOnly {
            return Err(gate::read_only_error(&target.name));
        }
    }
    let editor = state.sql.editor(&conn_id, &editor_id).await?;
    match action {
        TxnAction::Begin => editor.conn.txn_begin().await?,
        TxnAction::Commit => editor.conn.txn_end(true).await?,
        TxnAction::Rollback => editor.conn.txn_end(false).await?,
        TxnAction::State => {}
    }
    editor.conn.txn_state().await
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

#[derive(Serialize, Type)]
pub struct GridPending {
    pub id: String,
    pub db_name: String,
    pub environment: String,
    /// The statements as readable SQL, for the commit review.
    pub statements: Vec<String>,
}

/// Turns the grid's edits into statements and holds them for the commit review. A Read-only database refuses here.
#[tauri::command]
#[specta::specta]
pub async fn sql_grid_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    edits: GridEdits,
) -> AppResult<GridPending> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if decide(target.mode, Class::Write) == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let stmts = state.sql.pg(&conn_id).await?.grid_statements(&edits).await?;
    let statements = stmts.iter().map(|s| s.shown.clone()).collect();
    let id = state.sql.grid_pending.put(&conn_id, Class::Write, stmts)?;
    Ok(GridPending { id, db_name: target.name, environment: target.environment, statements })
}

/// "Commit": runs what `sql_grid_prepare` held, in one transaction, after the mode is asked again.
#[tauri::command]
#[specta::specta]
pub async fn sql_grid_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
) -> AppResult<u32> {
    state.require_unlocked()?;
    let (stmts, class) = state.sql.grid_pending.take(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, None)?;
    state.sql.pg(&conn_id).await?.grid_commit(&stmts).await
}

#[tauri::command]
#[specta::specta]
pub fn sql_grid_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.grid_pending.take(&conn_id, &pending_id);
    Ok(())
}

/// A table's columns, constraints, indexes, triggers and DDL (S38).
#[tauri::command]
#[specta::specta]
pub async fn sql_structure(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
) -> AppResult<TableStructure> {
    state.require_unlocked()?;
    state.sql.pg(&conn_id).await?.structure(&schema, &table).await
}

#[derive(Serialize, Type)]
pub struct StructurePending {
    pub id: String,
    pub level: PendingLevel,
    pub db_name: String,
    pub environment: String,
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
}

/// Turns the operations into statements and holds them for the review. Read-only databases refuse here; dropping or
/// rewriting asks for the database's name.
#[tauri::command]
#[specta::specta]
pub async fn sql_structure_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
    ops: Vec<StructureOp>,
) -> AppResult<StructurePending> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if decide(target.mode, Class::Schema) == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let pg = state.sql.pg(&conn_id).await?;
    let rows = pg.structure(&schema, &table).await?.stats.rows_estimate;
    let (statements, warnings) = sql::structure::statements(&schema, &table, &ops, rows)?;
    let class = statements.iter().map(|s| classify(s)).max().unwrap_or(Class::Schema);
    let decision = decide(target.mode, class);
    let id = state.sql.structure_pending.put(&conn_id, class, statements.clone())?;
    Ok(StructurePending {
        id,
        level: if decision == Decision::Typed { PendingLevel::Typed } else { PendingLevel::Confirm },
        db_name: target.name,
        environment: target.environment,
        statements,
        warnings,
    })
}

/// "Apply": the held statements, in one transaction, after the mode and (if dangerous) the name are checked.
#[tauri::command]
#[specta::specta]
pub async fn sql_structure_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
) -> AppResult<()> {
    state.require_unlocked()?;
    let class = state.sql.structure_pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    let (stmts, _) = state.sql.structure_pending.take(&conn_id, &pending_id)?;
    state.sql.pg(&conn_id).await?.structure_commit(&stmts).await
}

#[tauri::command]
#[specta::specta]
pub fn sql_structure_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.structure_pending.take(&conn_id, &pending_id);
    Ok(())
}

/// The plan of the statement at the cursor (or the selection). `analyze` runs it, so it only works for reads.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn sql_explain(
    state: State<'_, AppState>,
    conn_id: String,
    editor_id: String,
    text: String,
    mode: RunMode,
    cursor: Option<u32>,
    analyze: bool,
    timeout_ms: u32,
) -> AppResult<ExplainPlan> {
    state.require_unlocked()?;
    let pieces = pieces_for(&text, mode, cursor)?;
    let piece = pieces.first().ok_or_else(|| crate::error::AppError::validation("There is nothing to explain."))?;
    if analyze && classify(&piece.text) != Class::Read {
        return Err(crate::error::AppError::validation(
            "Explain analyze runs the statement, so it only works for reads. Use plain Explain for a change.",
        ));
    }
    let editor = state.sql.editor(&conn_id, &editor_id).await?;
    editor.conn.explain(&piece.text, analyze, timeout_ms.clamp(1000, 3_600_000)).await
}

#[derive(serde::Deserialize, Type)]
pub struct TableRef {
    pub schema: String,
    pub table: String,
}

/// Tables and foreign keys, `steps` hops out from the tables given (S40).
#[tauri::command]
#[specta::specta]
pub async fn sql_diagram(
    state: State<'_, AppState>,
    conn_id: String,
    tables: Vec<TableRef>,
    steps: u32,
) -> AppResult<Diagram> {
    state.require_unlocked()?;
    let start: Vec<(String, String)> = tables.into_iter().map(|t| (t.schema, t.table)).collect();
    state.sql.pg(&conn_id).await?.diagram(&start, steps).await
}

/// Writes a file the person picks with the system's save dialog (the diagram's SVG or PNG; later exports). The
/// window never touches the file system: it hands Rust the bytes. Returns the path, or `None` when cancelled.
#[tauri::command]
#[specta::specta]
pub async fn sql_save_export(
    app: tauri::AppHandle,
    suggested_name: String,
    data: Vec<u8>,
) -> AppResult<Option<String>> {
    // A diagram is kilobytes; this is a ceiling against a runaway caller.
    if data.len() > 64 * 1024 * 1024 {
        return Err(crate::error::AppError::validation("That file is too big to export from here."));
    }
    let dialog_app = app.clone();
    let name = suggested_name.replace(['/', '\\'], "-");
    let picked = tokio::task::spawn_blocking(move || dialog_app.dialog().file().set_file_name(&name).blocking_save_file())
        .await
        .map_err(|e| crate::error::AppError::internal(e.to_string()))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|_| crate::error::AppError::validation("That location can't be used."))?;
    std::fs::write(&path, data).map_err(|e| crate::error::AppError::internal(format!("Couldn't write the file: {e}")))?;
    Ok(Some(path.display().to_string()))
}
