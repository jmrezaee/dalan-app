//! Editing rows in a table's grid (F22, S36 edits). The window sends what changed (a row's key and the old and new
//! text of each changed cell, or a row to delete); Rust turns it into statements, shows them, and on commit runs
//! them in one transaction. Each update says "where the key matches and the cell is still what you saw" (the
//! optimistic check): if a statement changes anything other than exactly one row, everything is rolled back.
//! A table with no primary key can't be edited here: its rows can't be matched safely.

use std::sync::atomic::Ordering;

use serde::Deserialize;
use specta::Type;

use super::pg::{map_error, quote, PgConn};
use crate::error::{AppError, AppResult, ErrorCode};

#[derive(Debug, Clone, Deserialize, Type)]
pub struct KeyValue {
    pub column: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct CellChange {
    pub column: String,
    /// The text the person saw; `None` was NULL.
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct RowEdit {
    pub key: Vec<KeyValue>,
    /// Empty and `delete` set: the row goes. Otherwise the cells to change.
    pub changes: Vec<CellChange>,
    pub delete: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct GridEdits {
    pub schema: String,
    pub table: String,
    pub rows: Vec<RowEdit>,
}

/// One statement of a commit: the SQL with parameters (all text, cast on the server), and the same as readable text.
#[derive(Debug, Clone)]
pub struct GridStatement {
    pub sql: String,
    pub params: Vec<Option<String>>,
    pub shown: String,
}

fn lit(v: &Option<String>) -> String {
    match v {
        None => "NULL".to_string(),
        Some(t) => format!("'{}'", t.replace('\'', "''")),
    }
}

impl PgConn {
    /// Checks the edits against the table (whole key, real columns) and builds the statements.
    pub async fn grid_statements(&self, edits: &GridEdits) -> AppResult<Vec<GridStatement>> {
        if edits.rows.is_empty() {
            return Err(AppError::validation("There are no changes to commit."));
        }
        let info = self.table_info(&edits.schema, &edits.table).await?;
        let mut pk: Vec<_> = info
            .columns
            .iter()
            .filter(|c| c.pk_position.is_some())
            .collect();
        pk.sort_by_key(|c| c.pk_position);
        if pk.is_empty() {
            return Err(AppError::new(
                ErrorCode::SqlNoKey,
                "This table has no key, so rows can't be matched safely. Use a query instead.",
            ));
        }
        let col = |name: &str| {
            info.columns.iter().find(|c| c.name == name).ok_or_else(|| {
                AppError::validation(format!("There is no column {name} in this table."))
            })
        };
        let table = format!("{}.{}", quote(&edits.schema), quote(&edits.table));
        let mut out = Vec::new();
        for row in &edits.rows {
            // The whole key, each column once: a partial key would match more than one row.
            let mut names: Vec<&str> = row.key.iter().map(|k| k.column.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            if names.len() != row.key.len()
                || names.len() != pk.len()
                || !pk.iter().all(|c| names.contains(&c.name.as_str()))
            {
                return Err(AppError::validation(
                    "A row was sent without its whole primary key.",
                ));
            }
            let mut params: Vec<Option<String>> = Vec::new();
            let mut shown_where: Vec<String> = Vec::new();
            let mut where_sql: Vec<String> = Vec::new();
            let add =
                |c: &super::pg::SqlColumn, v: Option<String>, params: &mut Vec<Option<String>>| {
                    params.push(v);
                    // The type name comes from the catalog (`format_type`), never from the window.
                    format!("(${})::text::{}", params.len(), c.type_name)
                };
            if row.delete {
                for k in &row.key {
                    let c = col(&k.column)?;
                    where_sql.push(format!(
                        "{} = {}",
                        quote(&c.name),
                        add(c, Some(k.value.clone()), &mut params)
                    ));
                    shown_where.push(format!(
                        "{} = {}",
                        quote(&c.name),
                        lit(&Some(k.value.clone()))
                    ));
                }
                out.push(GridStatement {
                    sql: format!("delete from {table} where {}", where_sql.join(" and ")),
                    shown: format!("delete from {table} where {}", shown_where.join(" and ")),
                    params,
                });
                continue;
            }
            if row.changes.is_empty() {
                return Err(AppError::validation("A row was sent with nothing changed."));
            }
            let mut set_sql = Vec::new();
            let mut set_shown = Vec::new();
            for ch in &row.changes {
                let c = col(&ch.column)?;
                if c.pk_position.is_some() && ch.old != ch.new {
                    // Changing a key moves the row's identity: not from a grid.
                    return Err(AppError::validation(format!(
                        "{} is part of the key. Change it with a query.",
                        c.name
                    )));
                }
                set_sql.push(format!(
                    "{} = {}",
                    quote(&c.name),
                    add(c, ch.new.clone(), &mut params)
                ));
                set_shown.push(format!("{} = {}", quote(&c.name), lit(&ch.new)));
            }
            for k in &row.key {
                let c = col(&k.column)?;
                where_sql.push(format!(
                    "{} = {}",
                    quote(&c.name),
                    add(c, Some(k.value.clone()), &mut params)
                ));
                shown_where.push(format!(
                    "{} = {}",
                    quote(&c.name),
                    lit(&Some(k.value.clone()))
                ));
            }
            // The optimistic check: the cell must still be what the person saw. Types with no equality operator
            // (json, xml, geometric ones) are compared by their text form.
            for ch in &row.changes {
                let c = col(&ch.column)?;
                let no_eq = matches!(
                    c.type_name.as_str(),
                    "json"
                        | "xml"
                        | "point"
                        | "line"
                        | "lseg"
                        | "box"
                        | "path"
                        | "polygon"
                        | "circle"
                );
                if no_eq {
                    params.push(ch.old.clone());
                    where_sql.push(format!(
                        "{}::text is not distinct from (${})::text",
                        quote(&c.name),
                        params.len()
                    ));
                } else {
                    where_sql.push(format!(
                        "{} is not distinct from {}",
                        quote(&c.name),
                        add(c, ch.old.clone(), &mut params)
                    ));
                }
                shown_where.push(format!(
                    "{} is not distinct from {}",
                    quote(&c.name),
                    lit(&ch.old)
                ));
            }
            out.push(GridStatement {
                sql: format!(
                    "update {table} set {} where {}",
                    set_sql.join(", "),
                    where_sql.join(" and ")
                ),
                shown: format!(
                    "update {table} set {} where {}",
                    set_shown.join(", "),
                    shown_where.join(" and ")
                ),
                params,
            });
        }
        Ok(out)
    }

    /// Runs the statements in one transaction. Every one must change exactly one row, or nothing is written.
    pub async fn grid_commit(&self, stmts: &[GridStatement]) -> AppResult<u32> {
        let _scope = self.scope().await;
        self.dirty.store(true, Ordering::SeqCst);
        self.client
            .batch_execute("BEGIN; SET LOCAL lock_timeout = 5000")
            .await
            .map_err(|e| map_error(&e))?;
        let mut failure: Option<AppError> = None;
        for (i, st) in stmts.iter().enumerate() {
            let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = st
                .params
                .iter()
                .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            match self.client.execute(&st.sql, &refs).await {
                Ok(1) => {}
                Ok(_) => {
                    failure = Some(AppError::new(
                        ErrorCode::SqlConflict,
                        format!("Row {} was changed or removed by someone else since you loaded it. Nothing was written. Reload the table and try again.", i + 1),
                    ));
                    break;
                }
                Err(e) => {
                    failure = Some(map_error(&e));
                    break;
                }
            }
        }
        let end = if failure.is_none() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        let ended = self.client.batch_execute(end).await;
        if ended.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        if let Some(f) = failure {
            return Err(f);
        }
        ended.map_err(|e| map_error(&e))?;
        Ok(stmts.len() as u32)
    }
}
