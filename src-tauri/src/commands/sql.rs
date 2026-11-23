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
use crate::sql::activity::{Activity, ActivityView};
use crate::sql::classify::{Class, Piece};
use crate::sql::run::{pieces_for_d, HeldRun, RunMode, RunOpts, SqlCatalog, SqlRunEvent, TxnState};
use crate::sql;
use crate::sql::diagram::Diagram;
use crate::sql::explain::ExplainPlan;
use crate::sql::grid::GridEdits;
use crate::sql::structure::{StructureOp, TableStructure};
use tauri_plugin_dialog::DialogExt;
use crate::sql::backup::{self, Tool};
use crate::sql::transfer::{ExportFormat, CsvPreview, TransferProgress};
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.objects(&path).await;
    }
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.table_info(&schema, &table).await;
    }
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.table_page(&request).await;
    }
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
    let n = if let Some(my) = state.sql.my(&conn_id).await? {
        my.count(&schema, &table, &filter).await?
    } else {
        state.sql.pg(&conn_id).await?.count(&schema, &table, &filter).await?
    };
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.cell(&schema, &table, &key, &column).await;
    }
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
    let dialect = state.sql.dialect(&conn_id).await?;
    let pieces = pieces_for_d(&text, mode, cursor, dialect)?;
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
    let classes: Vec<Class> = pieces.iter().map(|p| sql::classify::classify_for(&p.text, dialect)).collect();
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
    let c = if let Some(my) = state.sql.my(&conn_id).await? {
        my.catalog(refresh).await?
    } else {
        state.sql.pg(&conn_id).await?.catalog(refresh).await?
    };
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
    let stmts = if let Some(my) = state.sql.my(&conn_id).await? {
        my.grid_statements(&edits).await?
    } else {
        state.sql.pg(&conn_id).await?.grid_statements(&edits).await?
    };
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.grid_commit(&stmts).await;
    }
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.structure(&schema, &table).await;
    }
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
    let dialect = state.sql.dialect(&conn_id).await?;
    let (statements, warnings) = if let Some(my) = state.sql.my(&conn_id).await? {
        let rows = my.structure(&schema, &table).await?.stats.rows_estimate;
        let info = my.table_info(&schema, &table).await?;
        let ddl = my.structure(&schema, &table).await?.ddl;
        sql::myextra::statements_my(&schema, &table, &ops, rows, &info, &ddl)?
    } else {
        let pg = state.sql.pg(&conn_id).await?;
        let rows = pg.structure(&schema, &table).await?.stats.rows_estimate;
        sql::structure::statements(&schema, &table, &ops, rows)?
    };
    let class = statements.iter().map(|s| sql::classify::classify_for(s, dialect)).max().unwrap_or(Class::Schema);
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.structure_commit(&stmts).await;
    }
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
    let dialect = state.sql.dialect(&conn_id).await?;
    let pieces = pieces_for_d(&text, mode, cursor, dialect)?;
    let piece = pieces.first().ok_or_else(|| crate::error::AppError::validation("There is nothing to explain."))?;
    if analyze && sql::classify::classify_for(&piece.text, dialect) != Class::Read {
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
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.diagram(&start, steps).await;
    }
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

/// The activity views (S41): tiles always, plus sessions with blocking chains, or one of the server-wide tables.
#[tauri::command]
#[specta::specta]
pub async fn sql_activity(state: State<'_, AppState>, conn_id: String, view: ActivityView) -> AppResult<Activity> {
    state.require_unlocked()?;
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.activity(view).await;
    }
    state.sql.pg(&conn_id).await?.activity(view).await
}

#[derive(Debug, Clone, Copy, serde::Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionAction {
    Cancel,
    End,
}

#[derive(Serialize, Type)]
pub struct SessionPending {
    pub id: String,
    pub level: PendingLevel,
    pub db_name: String,
    pub environment: String,
    pub pid: i32,
    pub user: String,
    pub state: String,
    pub query: String,
    pub end: bool,
}

/// Cancelling another person's query or ending their session: refused in a Read-only database, held for a
/// confirmation, and on a production database the name must be typed.
#[tauri::command]
#[specta::specta]
pub async fn sql_session_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    pid: i32,
    action: SessionAction,
) -> AppResult<SessionPending> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    let production = target.environment == "production";
    let class = if production { Class::Dangerous } else { Class::Write };
    if decide(target.mode, class) == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let end = matches!(action, SessionAction::End);
    let info = if let Some(my) = state.sql.my(&conn_id).await? {
        my.session_info(u32::try_from(pid).map_err(|_| crate::error::AppError::validation("That isn't a session id."))?).await?
    } else {
        state.sql.pg(&conn_id).await?.session_info(pid).await?
    };
    let (user, st, query, started) = info
        .ok_or_else(|| crate::error::AppError::not_found("That session is gone, or it is one of Dalan's own."))?;
    let id = state.sql.session_pending.put(&conn_id, class, (pid, end, started))?;
    Ok(SessionPending {
        id,
        level: if production { PendingLevel::Typed } else { PendingLevel::Confirm },
        db_name: target.name,
        environment: target.environment,
        pid,
        user,
        state: st,
        query,
        end,
    })
}

/// "Cancel query" / "End session": the held action, once, after the mode and (on production) the name are checked.
/// `true` when the server did it; `false` when the session had gone.
#[tauri::command]
#[specta::specta]
pub async fn sql_session_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
) -> AppResult<bool> {
    state.require_unlocked()?;
    let class = state.sql.session_pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    let ((pid, end, started), _) = state.sql.session_pending.take(&conn_id, &pending_id)?;
    if let Some(my) = state.sql.my(&conn_id).await? {
        return Ok(my.signal_session(u32::try_from(pid).map_err(|_| crate::error::AppError::validation("That isn't a session id."))?, end, &started).await?.unwrap_or(false));
    }
    Ok(state.sql.pg(&conn_id).await?.signal_session(pid, end, &started).await?.unwrap_or(false))
}

#[tauri::command]
#[specta::specta]
pub fn sql_session_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.session_pending.take(&conn_id, &pending_id);
    Ok(())
}

// ---- Transfer (S43, F26)

pub(crate) async fn pick_save(app: &tauri::AppHandle, name: String) -> AppResult<Option<std::path::PathBuf>> {
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || dialog_app.dialog().file().set_file_name(&name).blocking_save_file())
        .await
        .map_err(|e| crate::error::AppError::internal(e.to_string()))?;
    picked
        .map(|p| p.into_path().map_err(|_| crate::error::AppError::validation("That location can't be used.")))
        .transpose()
}

async fn pick_open(app: &tauri::AppHandle, title: &'static str) -> AppResult<Option<std::path::PathBuf>> {
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || dialog_app.dialog().file().set_title(title).blocking_pick_file())
        .await
        .map_err(|e| crate::error::AppError::internal(e.to_string()))?;
    picked
        .map(|p| p.into_path().map_err(|_| crate::error::AppError::validation("That location can't be used.")))
        .transpose()
}

#[derive(Serialize, Type)]
pub struct TransferDone {
    pub path: String,
    pub rows: f64,
    pub bytes: f64,
}

/// Writes a table (or the rows matching `filter`) to a file the person picks. Rows stream to the file; progress
/// arrives on `progress`. `None` when the save dialog was cancelled.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn sql_export_table(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    transfer_id: String,
    schema: String,
    table: String,
    filter: String,
    format: ExportFormat,
    header: bool,
    progress: Channel<TransferProgress>,
) -> AppResult<Option<TransferDone>> {
    state.require_unlocked()?;
    let my = state.sql.my(&conn_id).await?;
    let pg = if my.is_some() { None } else { Some(state.sql.pg(&conn_id).await?) };
    let Some(path) = pick_save(&app, format!("{table}.{}", format.extension())).await? else {
        return Ok(None);
    };
    let stop = state.sql.begin_transfer(&transfer_id);
    let file = std::fs::File::create(&path).map_err(|e| crate::error::AppError::validation(format!("Couldn't create that file: {e}")))?;
    let mut out = std::io::BufWriter::new(file);
    let report = |rows, bytes| {
        let _ = progress.send(TransferProgress { rows, bytes });
    };
    let result = match (&my, &pg) {
        (Some(my), _) => my.export_table(&schema, &table, &filter, format, header, &mut out, &report, &stop).await,
        (None, Some(pg)) => pg.export_table(&schema, &table, &filter, format, header, &mut out, &report, &stop).await,
        _ => Err(crate::error::AppError::internal("no connection")),
    };
    state.sql.end_transfer(&transfer_id);
    match result {
        Ok((rows, bytes)) => Ok(Some(TransferDone { path: path.display().to_string(), rows, bytes })),
        Err(e) => {
            drop(out);
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// Stop for an export, a backup or a restore.
#[tauri::command]
#[specta::specta]
pub fn sql_transfer_stop(state: State<'_, AppState>, transfer_id: String) -> AppResult<()> {
    state.sql.stop_transfer(&transfer_id);
    Ok(())
}

#[derive(Serialize, Type)]
pub struct CsvFile {
    /// What the window sends back to choose this file; the path stays in Rust.
    pub id: String,
    pub name: String,
}

/// The open-file dialog for an import. Rust keeps the path; the window only shows the name.
#[tauri::command]
#[specta::specta]
pub async fn sql_import_pick(app: tauri::AppHandle, state: State<'_, AppState>, conn_id: String) -> AppResult<Option<CsvFile>> {
    state.require_unlocked()?;
    Ok(pick_open(&app, "Import a CSV").await?.map(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        CsvFile { id: state.sql.put_file(&conn_id, p), name }
    }))
}

fn delimiter(d: &str) -> char {
    match d {
        "tab" => '\t',
        ";" => ';',
        "|" => '|',
        _ => ',',
    }
}

/// The file's columns, its first rows and how they line up with the table's columns.
#[tauri::command]
#[specta::specta]
pub async fn sql_import_preview(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
    file_id: String,
    delimiter_name: String,
    header: bool,
) -> AppResult<CsvPreview> {
    state.require_unlocked()?;
    let path = state.sql.file(&conn_id, &file_id)?;
    let text = tokio::task::spawn_blocking(move || crate::sql::transfer::read_limited(&path)).await.map_err(|e| crate::error::AppError::internal(e.to_string()))??;
    if let Some(my) = state.sql.my(&conn_id).await? {
        return my.import_preview(&schema, &table, &text, delimiter(&delimiter_name), header).await;
    }
    state.sql.pg(&conn_id).await?.import_preview(&schema, &table, &text, delimiter(&delimiter_name), header).await
}

#[derive(Serialize, Type)]
pub struct CsvImportPending {
    pub id: String,
    pub db_name: String,
    pub environment: String,
    pub rows: f64,
    pub statement: String,
}

/// Maps the file onto the table and holds the load for the confirmation. Read-only databases refuse.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn sql_import_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
    table: String,
    file_id: String,
    delimiter_name: String,
    header: bool,
    mapping: Vec<Option<String>>,
) -> AppResult<CsvImportPending> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if decide(target.mode, Class::Write) == Decision::Refuse {
        return Err(gate::read_only_error(&target.name));
    }
    let path = state.sql.file(&conn_id, &file_id)?;
    let text = tokio::task::spawn_blocking(move || crate::sql::transfer::read_limited(&path)).await.map_err(|e| crate::error::AppError::internal(e.to_string()))??;
    let plan = if let Some(my) = state.sql.my(&conn_id).await? {
        my.import_plan(&schema, &table, &text, delimiter(&delimiter_name), header, &mapping).await?
    } else {
        state.sql.pg(&conn_id).await?.import_plan(&schema, &table, &text, delimiter(&delimiter_name), header, &mapping).await?
    };
    let (rows, statement) = (plan.rows as f64, plan.statement.clone());
    let id = state.sql.import_pending.put(&conn_id, Class::Write, plan)?;
    Ok(CsvImportPending { id, db_name: target.name, environment: target.environment, rows, statement })
}

/// Loads what `sql_import_prepare` held, in one transaction, after the mode is asked again. Returns the rows loaded.
#[tauri::command]
#[specta::specta]
pub async fn sql_import_commit(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<f64> {
    state.require_unlocked()?;
    let class = state.sql.import_pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    gate::approve(target.mode, class, &target.name, None)?;
    let (plan, _) = state.sql.import_pending.take(&conn_id, &pending_id)?;
    if let Some(my) = state.sql.my(&conn_id).await? {
        return Ok(my.import_commit(&plan).await? as f64);
    }
    Ok(state.sql.pg(&conn_id).await?.import_commit(&plan).await? as f64)
}

#[tauri::command]
#[specta::specta]
pub fn sql_import_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.import_pending.take(&conn_id, &pending_id);
    Ok(())
}

#[derive(Serialize, Type)]
pub struct BackupInfo {
    pub tool: String,
    pub runs_on: String,
    pub version: Option<String>,
    /// Why it can't run, in words (tool missing, engine not supported yet).
    pub problem: Option<String>,
}

/// Which dump tool would run, where, and its version (S43's "Tool on db-primary").
#[tauri::command]
#[specta::specta]
pub async fn sql_backup_check(state: State<'_, AppState>, conn_id: String) -> AppResult<BackupInfo> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let cfg = sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let runs_on = backup::runs_on(ssh.as_deref());
    let tool = match backup::tool_name(cfg.engine, Tool::Dump) {
        Ok(t) => t,
        Err(e) => return Ok(BackupInfo { tool: String::new(), runs_on, version: None, problem: Some(e.message) }),
    };
    // MariaDB's own tool names are tried before the MySQL ones.
    Ok(match backup::resolve_tool(ssh.as_deref(), cfg.engine, Tool::Dump).await {
        Ok((name, v)) => BackupInfo { tool: name, runs_on, version: Some(v), problem: None },
        Err(e) => BackupInfo { tool: tool.into(), runs_on, version: None, problem: Some(e.message) },
    })
}

/// Runs the dump tool and streams its output to a file the person picks. `None` when the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn sql_backup_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    transfer_id: String,
    progress: Channel<TransferProgress>,
) -> AppResult<Option<TransferDone>> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let cfg = sql::load(&state.store(), &db_id).await?;
    backup::tool_name(cfg.engine, Tool::Dump)?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let ext = if backup::is_my(cfg.engine) { "sql" } else if backup::is_mongo(cfg.engine) { "archive.gz" } else { "dump" };
    let name = format!("{}-{}.{ext}", cfg.database.clone().unwrap_or_else(|| cfg.name.clone()), chrono_free_date());
    let Some(path) = pick_save(&app, name).await? else {
        return Ok(None);
    };
    let stop = state.sql.begin_transfer(&transfer_id);
    let file = std::fs::File::create(&path).map_err(|e| crate::error::AppError::validation(format!("Couldn't create that file: {e}")))?;
    let mut out = std::io::BufWriter::new(file);
    let result = backup::run(backup::Run {
        cfg: &cfg,
        ssh: ssh.as_deref(),
        tool: Tool::Dump,
        stdout: Some(&mut out),
        stdin_file: None,
        progress: &|bytes| {
            let _ = progress.send(TransferProgress { rows: 0.0, bytes });
        },
        log: &|l| log::debug!("dump tool: {l}"),
        stop: &stop,
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    drop(out);
    match result {
        Ok(()) => {
            let bytes = std::fs::metadata(&path).map(|m| m.len() as f64).unwrap_or(0.0);
            Ok(Some(TransferDone { path: path.display().to_string(), rows: 0.0, bytes }))
        }
        Err(e) => {
            // A dump that did not finish is not a backup: it goes.
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// Today as YYYY-MM-DD in UTC, without a date crate.
fn chrono_free_date() -> String {
    let days = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Serialize, Type)]
pub struct RestorePending {
    pub id: String,
    pub db_name: String,
    pub environment: String,
    pub file_name: String,
    pub file_bytes: f64,
    pub runs_on: String,
}

/// Picks a dump file and holds the restore. Always typed; refused in Read-only, and on a production database unless it
/// has been switched to Open (which needs the master password).
#[tauri::command]
#[specta::specta]
pub async fn sql_restore_prepare(app: tauri::AppHandle, state: State<'_, AppState>, conn_id: String) -> AppResult<Option<RestorePending>> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if target.mode == RedisWriteMode::ReadOnly {
        return Err(gate::read_only_error(&target.name));
    }
    if target.environment == "production" && target.mode != RedisWriteMode::Open {
        return Err(crate::error::AppError::new(
            crate::error::ErrorCode::DbReadOnlyMode,
            format!("{} is a production database. To restore into it, switch it to Open first (that needs the master password).", target.name),
        ));
    }
    let cfg = sql::load(&state.store(), &db_id).await?;
    // The tool must exist before anything is asked.
    let ssh_for_check = state.sql.ssh(&conn_id).await?;
    if backup::is_my(cfg.engine) || backup::is_mongo(cfg.engine) {
        backup::resolve_tool(ssh_for_check.as_deref(), cfg.engine, Tool::Restore).await?;
    } else {
        backup::tool_name(cfg.engine, Tool::Restore)?;
    }
    let Some(path) = pick_open(&app, "Restore from a dump").await? else {
        return Ok(None);
    };
    let ssh = state.sql.ssh(&conn_id).await?;
    let file_bytes = std::fs::metadata(&path).map(|m| m.len() as f64).map_err(|e| crate::error::AppError::validation(format!("Couldn't read that file: {e}")))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let id = state.sql.restore_pending.put(&conn_id, Class::Dangerous, path)?;
    Ok(Some(RestorePending { id, db_name: target.name, environment: target.environment, file_name, file_bytes, runs_on: backup::runs_on(ssh.as_deref()) }))
}

/// Runs the restore tool on the held file after the database's exact name is typed and the mode asked again.
#[tauri::command]
#[specta::specta]
pub async fn sql_restore_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    transfer_id: String,
    progress: Channel<TransferProgress>,
) -> AppResult<()> {
    state.require_unlocked()?;
    let class = state.sql.restore_pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    if target.environment == "production" && target.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&target.name));
    }
    gate::approve(target.mode, class, &target.name, typed_name.as_deref())?;
    let (path, _) = state.sql.restore_pending.take(&conn_id, &pending_id)?;
    let cfg = sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let stop = state.sql.begin_transfer(&transfer_id);
    let result = backup::run(backup::Run {
        cfg: &cfg,
        ssh: ssh.as_deref(),
        tool: Tool::Restore,
        stdout: None,
        stdin_file: Some(&path),
        progress: &|bytes| {
            let _ = progress.send(TransferProgress { rows: 0.0, bytes });
        },
        log: &|l| log::debug!("restore tool: {l}"),
        stop: &stop,
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    result
}

#[tauri::command]
#[specta::specta]
pub fn sql_restore_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) -> AppResult<()> {
    let _ = state.sql.restore_pending.take(&conn_id, &pending_id);
    Ok(())
}
