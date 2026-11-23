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

fn kind_of(t: &PgType) -> CellKind {
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
        emit: &(dyn Fn(SqlRunEvent) + Sync),
    ) -> AppResult<()> {
        let started = Instant::now();
        // The whole script is looked at before any of it runs: a refused statement means nothing runs.
        for (i, p) in pieces.iter().enumerate() {
            let class = classify::classify(&p.text);
            if class != Class::Read {
                emit(SqlRunEvent::Refused {
                    index: i as u32,
                    class,
                    message: "This changes data or structure. The editor only runs reads for now; changes arrive with the confirmation dialogs in an update.".into(),
                });
                emit(SqlRunEvent::Finished { elapsed_ms: ms(started) });
                return Ok(());
            }
        }
        let _scope = self.scope().await;
        self.dirty.store(true, Ordering::SeqCst);
        self.client
            .batch_execute(&format!(
                "BEGIN READ ONLY; SET LOCAL statement_timeout = {}",
                opts.timeout_ms
            ))
            .await
            .map_err(|e| map_error(&e))?;
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
            if let Err((kind, message)) = self.run_one(i as u32, p, opts, cancel, emit).await {
                emit(SqlRunEvent::Failed {
                    index: i as u32,
                    kind,
                    message,
                });
                break;
            }
        }
        // Whatever happened, the transaction ends (an aborted one can only be rolled back).
        if self.client.batch_execute("ROLLBACK").await.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        emit(SqlRunEvent::Finished { elapsed_ms: ms(started) });
        Ok(())
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
            emit(SqlRunEvent::Columns { index, columns: cols });
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
        if !kinds.is_empty() && classify::is_query(&p.text) {
            // The text goes in on its own line: a trailing `-- comment` can't swallow what follows.
            // The extended protocol takes exactly one statement, whatever the two lexers think a `;` is.
            self.client
                .execute(&format!("DECLARE dalan_cur NO SCROLL CURSOR FOR\n{}", p.text), &[])
                .await
                .map_err(fail)?;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err((FailKind::Cancelled, "Stopped.".into()));
                }
                // One more than the limit tells whether the limit cut the result off.
                let want = if limit == 0 { BATCH } else { BATCH.min(limit.saturating_add(1) - sent) };
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
            self.client.batch_execute("CLOSE dalan_cur").await.map_err(fail)?;
        } else {
            let msgs = self.client.simple_query(&p.text).await.map_err(fail)?;
            let all: Vec<Vec<SqlCell>> = msgs
                .iter()
                .filter_map(|m| match m {
                    SimpleQueryMessage::Row(r) => Some(to_cells(r)),
                    _ => None,
                })
                .collect();
            let keep = if limit == 0 { all.len() } else { all.len().min(limit as usize) };
            more = keep < all.len();
            for chunk in all[..keep].chunks(BATCH as usize) {
                emit(SqlRunEvent::Rows { index, rows: chunk.to_vec() });
            }
            sent = keep as u32;
        }
        emit(SqlRunEvent::Done { index, rows: sent, more, elapsed_ms: ms(t) });
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
    let all = classify::split(text);
    if all.is_empty() {
        return Err(AppError::validation("There is nothing to run."));
    }
    Ok(match mode {
        RunMode::All | RunMode::Selection => all,
        // The editor counts UTF-16 units, the splitter counts characters: an emoji before the caret is two of one.
        RunMode::Current => classify::at_cursor(&all, utf16_to_char(text, cursor.unwrap_or(0) as usize))
            .cloned()
            .into_iter()
            .collect(),
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
