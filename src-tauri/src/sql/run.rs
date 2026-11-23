//! The query editor's side of a PostgreSQL connection (D3, S37, F23): run a script statement by statement and stream
//! the answers, cancel from a second connection, and read the catalog for completion.
//!
//! Every statement runs inside `BEGIN READ ONLY` that is always rolled back, and `classify` has already refused
//! anything that isn't a read, so the editor can't change data before the gate's dialogs exist (D4).
//! A plain query is read through a cursor (`DECLARE ... CURSOR`, then `FETCH` in batches): the rows arrive as text
//! from the server, so no value loses a digit, and stopping at the row limit or on cancel costs nothing more.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::Serialize;
use specta::Type;
use tokio_postgres::types::{Kind, Type as PgType};
use tokio_postgres::{NoTls, SimpleQueryMessage};

use super::classify::{self, Class, Piece};
use super::pg::{map_error, start_tls, PgConn};
use super::values::{CellKind, SqlCell};
use crate::dbs::transport::TlsMode;
use crate::error::{AppError, AppResult};

/// Rows sent to the window at a time.
const BATCH: u32 = 500;

#[derive(Debug, Clone, Serialize, Type)]
pub struct RunColumn {
    pub name: String,
    pub type_name: String,
    pub kind: CellKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FailKind {
    Error,
    Timeout,
    Cancelled,
}

/// What a run tells the window. `index` is the statement's place in the script.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqlRunEvent {
    /// A statement begins. `start` and `end` are its characters in the editor's text.
    Started {
        index: u32,
        statement: String,
        start: u32,
        end: u32,
    },
    Columns {
        index: u32,
        columns: Vec<RunColumn>,
    },
    Rows {
        index: u32,
        rows: Vec<Vec<SqlCell>>,
    },
    /// `rows` is what was sent; `more` says the limit cut the result off.
    Done {
        index: u32,
        rows: u32,
        more: bool,
        elapsed_ms: u32,
        /// For a statement that returns no rows: how many rows it changed.
        affected: Option<f64>,
    },
    Failed {
        index: u32,
        kind: FailKind,
        message: String,
    },
    /// Not run: not a read. Nothing of the script ran.
    Refused {
        index: u32,
        class: Class,
        message: String,
    },
    Finished {
        elapsed_ms: u32,
    },
}

/// A manual transaction (S37's bar): what the person has done in it and for how long.
#[derive(Debug, Clone)]
pub struct TxnInfo {
    pub since: Instant,
    pub statements: u32,
    /// A statement failed: the server only accepts ROLLBACK now.
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TxnState {
    pub active: bool,
    pub statements: u32,
    pub age_s: u32,
    pub failed: bool,
    /// Locks this transaction holds on tables and rows (`pg_locks`).
    pub locks: u32,
}

/// A script held for the person's answer (the gate's "Confirm" and "Typed" levels).
#[derive(Debug, Clone)]
pub struct HeldRun {
    pub pieces: Vec<Piece>,
    pub editor_id: String,
}

pub struct RunOpts {
    /// Rows per result; 0 is no limit.
    pub limit: u32,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CatalogTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlCatalog {
    pub tables: Vec<CatalogTable>,
}

pub(super) fn kind_of(t: &PgType) -> CellKind {
    match t.kind() {
        Kind::Array(_) => CellKind::Array,
        Kind::Enum(_) => CellKind::Enum,
        _ => {
            let category = match t.name() {
                "date" | "time" | "timetz" | "timestamp" | "timestamptz" => "D",
                "interval" => "T",
                "text" | "varchar" | "bpchar" | "name" | "char" | "citext" => "S",
                _ => "",
            };
            CellKind::of_pg(t.name(), category)
        }
    }
}

fn ms(t: Instant) -> u32 {
    t.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

impl PgConn {
    /// Runs the statements in order and reports through `emit`. A statement that fails ends the run (the rest would
    /// run on a different footing than the person wrote). Returns `Err` only when the connection itself fails.
    pub async fn run_script(
        &self,
        pieces: &[Piece],
        opts: &RunOpts,
        cancel: &AtomicBool,
        allow_write: bool,
        emit: &(dyn Fn(SqlRunEvent) + Sync),
    ) -> AppResult<()> {
        let started = Instant::now();
        // The whole script is looked at before any of it runs. Statements that aren't reads need the gate's yes,
        // which the caller passes as `allow_write`; without it, a refused statement means nothing runs.
        let all_reads = pieces
            .iter()
            .all(|p| classify::classify(&p.text) == Class::Read);
        if !all_reads && !allow_write {
            let (i, class) = pieces
                .iter()
                .enumerate()
                .map(|(i, p)| (i, classify::classify(&p.text)))
                .find(|(_, c)| *c != Class::Read)
                .unwrap_or((0, Class::Dangerous));
            emit(SqlRunEvent::Refused {
                index: i as u32,
                class,
                message:
                    "This changes data or structure and hasn't been confirmed. Nothing was sent."
                        .into(),
            });
            emit(SqlRunEvent::Finished {
                elapsed_ms: ms(started),
            });
            return Ok(());
        }
        let _scope = self.scope().await;
        let manual = self.txn.lock().map(|t| t.is_some()).unwrap_or(false);
        let timeout = opts.timeout_ms;
        // Three footings: a manual transaction (nothing is wrapped; it is the person's), reads only (one read-only
        // transaction, always rolled back), and writes in auto-commit (each statement stands alone).
        if manual {
            let _ = self
                .client
                .batch_execute(&format!("SET LOCAL statement_timeout = {timeout}"))
                .await;
        } else if all_reads {
            self.dirty.store(true, Ordering::SeqCst);
            self.client
                .batch_execute(&format!(
                    "BEGIN READ ONLY; SET LOCAL statement_timeout = {timeout}"
                ))
                .await
                .map_err(|e| map_error(&e))?;
        } else {
            self.client
                .batch_execute(&format!("SET statement_timeout = {timeout}"))
                .await
                .map_err(|e| map_error(&e))?;
        }
        for (i, p) in pieces.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                emit(SqlRunEvent::Failed {
                    index: i as u32,
                    kind: FailKind::Cancelled,
                    message: "Stopped.".into(),
                });
                break;
            }
            emit(SqlRunEvent::Started {
                index: i as u32,
                statement: p.text.clone(),
                start: p.start as u32,
                end: p.end as u32,
            });
            // A query in auto-commit needs a transaction for its cursor; it commits (or rolls back) with it.
            let wrap = !manual && !all_reads && classify::is_query(&p.text);
            if wrap {
                self.client
                    .batch_execute("BEGIN")
                    .await
                    .map_err(|e| map_error(&e))?;
                self.dirty.store(true, Ordering::SeqCst);
            }
            let out = self.run_one(i as u32, p, opts, cancel, emit).await;
            if wrap
                && self
                    .client
                    .batch_execute(if out.is_ok() { "COMMIT" } else { "ROLLBACK" })
                    .await
                    .is_ok()
            {
                self.dirty.store(false, Ordering::SeqCst);
            }
            if manual {
                if let Ok(mut t) = self.txn.lock() {
                    if let Some(t) = t.as_mut() {
                        t.statements += 1;
                        t.failed |= out.is_err();
                    }
                }
            }
            if let Err((kind, message)) = out {
                emit(SqlRunEvent::Failed {
                    index: i as u32,
                    kind,
                    message,
                });
                break;
            }
        }
        // Whatever happened, a read-only transaction ends (an aborted one can only be rolled back).
        if !manual && all_reads && self.client.batch_execute("ROLLBACK").await.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        emit(SqlRunEvent::Finished {
            elapsed_ms: ms(started),
        });
        Ok(())
    }

    /// What each data-changing statement would do, for the confirmation dialog: it is run for real inside a
    /// transaction that is rolled back (a savepoint when a manual transaction is open), and the server's row count
    /// is reported. A table too big for ten seconds says so instead. Statements that aren't inserts, updates,
    /// deletes or merges get no note. Sequences and triggers' outside effects are not undone by the rollback.
    pub async fn preview(&self, pieces: &[Piece]) -> AppResult<Vec<Option<String>>> {
        let _scope = self.scope().await;
        let manual = self.txn.lock().map(|t| t.is_some()).unwrap_or(false);
        let mut out = Vec::new();
        for p in pieces {
            let first = p
                .text
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            if !matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE") {
                out.push(None);
                continue;
            }
            // Trying it out would do for real what a rollback can't undo (a session killed, a remote call made).
            if classify::calls_effectful(&p.text) {
                out.push(Some(
                    "Not tried out: it calls a function that acts outside the transaction."
                        .to_string(),
                ));
                continue;
            }
            let (open, close) = if manual {
                ("SAVEPOINT dalan_dry", "ROLLBACK TO SAVEPOINT dalan_dry")
            } else {
                ("BEGIN", "ROLLBACK")
            };
            self.dirty.store(!manual, Ordering::SeqCst);
            self.client
                .batch_execute(open)
                .await
                .map_err(|e| map_error(&e))?;
            let _ = self
                .client
                .batch_execute("SET LOCAL statement_timeout = 10000")
                .await;
            let r = self.client.execute(&p.text, &[]).await;
            let _ = self.client.batch_execute(close).await;
            if manual {
                let _ = self
                    .client
                    .batch_execute("RELEASE SAVEPOINT dalan_dry")
                    .await;
            } else {
                self.dirty.store(false, Ordering::SeqCst);
            }
            out.push(Some(match r {
                Ok(1) => "This will change 1 row.".to_string(),
                Ok(n) => format!("This will change {n} rows."),
                Err(e)
                    if e.as_db_error()
                        .is_some_and(|d| d.message().contains("statement timeout")) =>
                {
                    "Too big to count here: it ran for more than ten seconds.".to_string()
                }
                Err(e) => format!(
                    "The server says it would fail: {}",
                    e.as_db_error()
                        .map_or_else(|| e.to_string(), |d| d.message().to_string())
                ),
            }));
        }
        Ok(out)
    }

    pub fn has_txn(&self) -> bool {
        self.txn.lock().map(|t| t.is_some()).unwrap_or(false)
    }

    /// Begin a manual transaction on this connection.
    pub async fn txn_begin(&self) -> AppResult<()> {
        let _scope = self.scope().await;
        if self.txn.lock().map(|t| t.is_some()).unwrap_or(true) {
            return Err(AppError::new(
                crate::error::ErrorCode::SqlInTransaction,
                "A transaction is already open in this editor.",
            ));
        }
        self.client
            .batch_execute("BEGIN")
            .await
            .map_err(|e| map_error(&e))?;
        if let Ok(mut t) = self.txn.lock() {
            *t = Some(TxnInfo {
                since: Instant::now(),
                statements: 0,
                failed: false,
            });
        }
        Ok(())
    }

    /// Commit or roll back the manual transaction. The transaction is over either way, even if the server complains.
    pub async fn txn_end(&self, commit: bool) -> AppResult<()> {
        let _scope = self.scope().await;
        let was = self.txn.lock().ok().and_then(|mut t| t.take());
        let Some(was) = was else {
            return Ok(());
        };
        if commit && was.failed {
            // The server turns COMMIT into ROLLBACK for an aborted transaction; say so instead of pretending.
            let _ = self.client.batch_execute("ROLLBACK").await;
            return Err(AppError::validation(
                "The transaction had failed, so it was rolled back instead of committed.",
            ));
        }
        self.client
            .batch_execute(if commit { "COMMIT" } else { "ROLLBACK" })
            .await
            .map_err(|e| map_error(&e))
    }

    pub async fn txn_state(&self) -> AppResult<TxnState> {
        let info = self.txn.lock().ok().and_then(|t| t.clone());
        let Some(info) = info else {
            return Ok(TxnState {
                active: false,
                statements: 0,
                age_s: 0,
                failed: false,
                locks: 0,
            });
        };
        let mut locks = 0u32;
        if !info.failed {
            let _scope = self.scope().await;
            if let Ok(row) = self
                .client
                .query_one(
                    "select count(*)::int from pg_locks where pid = pg_backend_pid() and granted and locktype not in ('virtualxid', 'transactionid')",
                    &[],
                )
                .await
            {
                locks = row.get::<_, i32>(0).max(0) as u32;
            }
        }
        Ok(TxnState {
            active: true,
            statements: info.statements,
            age_s: info.since.elapsed().as_secs().min(u64::from(u32::MAX)) as u32,
            failed: info.failed,
            locks,
        })
    }

    async fn run_one(
        &self,
        index: u32,
        p: &Piece,
        opts: &RunOpts,
        cancel: &AtomicBool,
        emit: &(dyn Fn(SqlRunEvent) + Sync),
    ) -> Result<(), (FailKind, String)> {
        let fail = |e: tokio_postgres::Error| {
            let message = e
                .as_db_error()
                .map_or_else(|| map_error(&e).message, |d| d.message().to_string());
            let kind = if cancel.load(Ordering::SeqCst) {
                FailKind::Cancelled
            } else if message.contains("statement timeout") {
                FailKind::Timeout
            } else {
                FailKind::Error
            };
            (kind, message)
        };
        let t = Instant::now();
        let prepared = self.client.prepare(&p.text).await.map_err(fail)?;
        let cols: Vec<RunColumn> = prepared
            .columns()
            .iter()
            .map(|c| RunColumn {
                name: c.name().to_string(),
                type_name: c.type_().name().to_string(),
                kind: kind_of(c.type_()),
            })
            .collect();
        let kinds: Vec<CellKind> = cols.iter().map(|c| c.kind).collect();
        if !cols.is_empty() {
            emit(SqlRunEvent::Columns {
                index,
                columns: cols,
            });
        }
        let limit = opts.limit;
        let mut sent = 0u32;
        let mut more = false;
        let to_cells = |row: &tokio_postgres::SimpleQueryRow| -> Vec<SqlCell> {
            kinds
                .iter()
                .enumerate()
                .map(|(i, k)| SqlCell::new(*k, row.get(i).map(str::to_string)))
                .collect()
        };
        if kinds.is_empty() && !classify::is_query(&p.text) {
            // Changes something and returns no rows: the server's own count says how many it touched.
            let n = self.client.execute(&p.text, &[]).await.map_err(fail)?;
            emit(SqlRunEvent::Done {
                index,
                rows: 0,
                more: false,
                elapsed_ms: ms(t),
                affected: Some(n as f64),
            });
            return Ok(());
        }
        if !kinds.is_empty() && classify::is_query(&p.text) {
            // The text goes in on its own line: a trailing `-- comment` can't swallow what follows.
            // The extended protocol takes exactly one statement, whatever the two lexers think a `;` is.
            self.client
                .execute(
                    &format!("DECLARE dalan_cur NO SCROLL CURSOR FOR\n{}", p.text),
                    &[],
                )
                .await
                .map_err(fail)?;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err((FailKind::Cancelled, "Stopped.".into()));
                }
                // One more than the limit tells whether the limit cut the result off.
                let want = if limit == 0 {
                    BATCH
                } else {
                    BATCH.min(limit.saturating_add(1) - sent)
                };
                let msgs = self
                    .client
                    .simple_query(&format!("FETCH FORWARD {want} FROM dalan_cur"))
                    .await
                    .map_err(fail)?;
                let mut rows: Vec<Vec<SqlCell>> = msgs
                    .iter()
                    .filter_map(|m| match m {
                        SimpleQueryMessage::Row(r) => Some(to_cells(r)),
                        _ => None,
                    })
                    .collect();
                let got = rows.len() as u32;
                if limit != 0 && sent + got > limit {
                    rows.truncate((limit - sent) as usize);
                    more = true;
                }
                sent += rows.len() as u32;
                if !rows.is_empty() {
                    emit(SqlRunEvent::Rows { index, rows });
                }
                if more || got < want {
                    break;
                }
            }
            self.client
                .batch_execute("CLOSE dalan_cur")
                .await
                .map_err(fail)?;
        } else {
            let msgs = self.client.simple_query(&p.text).await.map_err(fail)?;
            let all: Vec<Vec<SqlCell>> = msgs
                .iter()
                .filter_map(|m| match m {
                    SimpleQueryMessage::Row(r) => Some(to_cells(r)),
                    _ => None,
                })
                .collect();
            let keep = if limit == 0 {
                all.len()
            } else {
                all.len().min(limit as usize)
            };
            more = keep < all.len();
            for chunk in all[..keep].chunks(BATCH as usize) {
                emit(SqlRunEvent::Rows {
                    index,
                    rows: chunk.to_vec(),
                });
            }
            sent = keep as u32;
        }
        emit(SqlRunEvent::Done {
            index,
            rows: sent,
            more,
            elapsed_ms: ms(t),
            affected: None,
        });
        Ok(())
    }

    /// Asks the server to stop what this connection is running, from a second connection to the same server through
    /// the same route (the server's cancel request is its own tiny protocol on a new socket).
    pub async fn cancel_running(&self) -> AppResult<()> {
        let stream = (self.reopen)().await?;
        let stream = if self.tls == TlsMode::Off {
            stream
        } else {
            start_tls(stream, self.tls, &self.server_name).await?
        };
        self.cancel
            .cancel_query_raw(stream, NoTls)
            .await
            .map_err(|e| map_error(&e))
    }

    /// Tables and columns for completion. Cached; `refresh` reads them again.
    pub async fn catalog(&self, refresh: bool) -> AppResult<std::sync::Arc<SqlCatalog>> {
        let mut cached = self.catalog.lock().await;
        if let (false, Some(c)) = (refresh, cached.as_ref()) {
            return Ok(std::sync::Arc::clone(c));
        }
        let rows = self
            .read_only(async |c| {
                c.query(
                    "select n.nspname::text, c.relname::text, array_agg(a.attname::text order by a.attnum)
                     from pg_class c
                     join pg_namespace n on n.oid = c.relnamespace
                     join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped
                     where c.relkind in ('r', 'v', 'm', 'p', 'f')
                       and n.nspname not in ('pg_catalog', 'information_schema', 'pg_toast')
                       and n.nspname not like 'pg_temp%'
                     group by 1, 2 order by 1, 2 limit 5000",
                    &[],
                )
                .await
            })
            .await?;
        let catalog = std::sync::Arc::new(SqlCatalog {
            tables: rows
                .iter()
                .map(|r| CatalogTable {
                    schema: r.get(0),
                    name: r.get(1),
                    columns: r.get(2),
                })
                .collect(),
        });
        *cached = Some(std::sync::Arc::clone(&catalog));
        Ok(catalog)
    }
}

/// Where a statement's position is for the window: `Piece` characters are what the editor counts.
pub fn pieces_for(text: &str, mode: RunMode, cursor: Option<u32>) -> AppResult<Vec<Piece>> {
    pieces_for_d(text, mode, cursor, classify::Dialect::Pg)
}

pub fn pieces_for_d(
    text: &str,
    mode: RunMode,
    cursor: Option<u32>,
    d: classify::Dialect,
) -> AppResult<Vec<Piece>> {
    let all = classify::split_for(text, d);
    if all.is_empty() {
        return Err(AppError::validation("There is nothing to run."));
    }
    Ok(match mode {
        RunMode::All | RunMode::Selection => all,
        // The editor counts UTF-16 units, the splitter counts characters: an emoji before the caret is two of one.
        RunMode::Current => {
            classify::at_cursor(&all, utf16_to_char(text, cursor.unwrap_or(0) as usize))
                .cloned()
                .into_iter()
                .collect()
        }
    })
}

fn utf16_to_char(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16();
    }
    text.chars().count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Current,
    Selection,
    All,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_pick_their_statements() {
        let sql = "select 1;\nselect 2;\nselect 3";
        assert_eq!(pieces_for(sql, RunMode::All, None).unwrap().len(), 3);
        assert_eq!(pieces_for(sql, RunMode::Selection, None).unwrap().len(), 3);
        let one = pieces_for(sql, RunMode::Current, Some(14)).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].text, "select 2");
        assert!(pieces_for("  -- only a comment", RunMode::All, None).is_err());
        // "😀" is two UTF-16 units: the caret at unit 14 is in the second statement.
        let e = pieces_for("select '😀';\nselect 2", RunMode::Current, Some(16)).unwrap();
        assert_eq!(e[0].text, "select 2");
    }

    #[test]
    fn kinds_follow_the_type() {
        assert_eq!(kind_of(&PgType::INT8), CellKind::Int);
        assert_eq!(kind_of(&PgType::TIMESTAMPTZ), CellKind::Datetime);
        assert_eq!(kind_of(&PgType::TEXT_ARRAY), CellKind::Array);
        assert_eq!(kind_of(&PgType::JSONB), CellKind::Json);
        assert_eq!(kind_of(&PgType::TEXT), CellKind::Text);
    }
}
