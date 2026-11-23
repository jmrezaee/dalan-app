//! The MySQL and MariaDB side of a SQL tab (D7 part 2): the same operations `PgConn` offers the window (tree, table
//! pages, counts, cells, completion, running statements), written for MySQL's catalog and rules. Everything that only
//! reads runs inside `START TRANSACTION READ ONLY`, rolled back, so the server itself stops a write that sneaks in.
//! Values come as the server's own text (see `mysql.rs`), so nothing loses a digit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::classify::{self, Class, Dialect, Piece};
use super::grid::{GridEdits, GridStatement};
use super::mysql::{MyConn, MyResult};
use super::run::{TxnInfo, TxnState};
use super::pg::{ObjectKind, PageCursor, PageRequest, SqlColumn, SqlForeignKey, SqlObject, SqlPage, SqlTableInfo, DEFAULT_PAGE, MAX_PAGE};
use super::run::{CatalogTable, FailKind, RunColumn, RunOpts, SqlCatalog, SqlRunEvent};
use super::values::{CellKind, SqlCell};
use crate::error::{AppError, AppResult, ErrorCode};

pub(super) const SYSTEM: &str = "('mysql', 'information_schema', 'performance_schema', 'sys')";
const BATCH: usize = 500;

/// A name in backticks.
pub fn q(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// A string literal, safe to put in a statement (backslashes and quotes escaped; the session never runs with
/// NO_BACKSLASH_ESCAPES, see `MyConn::connect`).
pub fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
}

/// A key value for a comparison: a plain integer stays a number (a string would be compared as a double beyond
/// 2^53), everything else is a quoted literal.
fn key_lit(kind: CellKind, v: &str) -> String {
    let digits = v.strip_prefix('-').unwrap_or(v);
    if kind == CellKind::Int && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        v.to_string()
    } else {
        lit(v)
    }
}

/// Statements that end the open transaction by themselves in MySQL and MariaDB (read from the statement's own
/// tokens, so a leading comment doesn't hide it). `temporary` tables are the exception for create and drop.
fn implicit_commit(sql: &str) -> bool {
    let w = classify::words_for(sql, Dialect::My);
    let has = |x: &str| w.iter().any(|y| y == x);
    match w.first().map(String::as_str) {
        Some("CREATE" | "DROP") => !has("TEMPORARY"),
        Some("ALTER" | "TRUNCATE" | "RENAME" | "GRANT" | "REVOKE" | "ANALYZE" | "OPTIMIZE" | "REPAIR" | "LOCK" | "UNLOCK" | "FLUSH" | "BEGIN" | "START" | "XA" | "CHANGE" | "STOP" | "INSTALL" | "UNINSTALL" | "CACHE" | "LOAD") => true,
        Some("SET") => has("AUTOCOMMIT"),
        _ => false,
    }
}

fn ms(t: Instant) -> u32 {
    t.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

/// From `column_type` (`bigint unsigned`, `varchar(40)`, `enum('a','b')`).
fn kind_of_type(t: &str) -> CellKind {
    let t = t.to_ascii_lowercase();
    let base: String = t.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    match base.as_str() {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "year" => CellKind::Int,
        "decimal" | "numeric" => CellKind::Decimal,
        "float" | "double" | "real" => CellKind::Float,
        "date" | "datetime" | "timestamp" => CellKind::Datetime,
        "time" => CellKind::Interval,
        "json" => CellKind::Json,
        "enum" | "set" => CellKind::Enum,
        "bit" | "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "geometry" | "point" | "linestring" | "polygon" => CellKind::Bytes,
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" => CellKind::Text,
        _ => CellKind::Unknown,
    }
}

/// One expression and nothing else, in MySQL's grammar.
pub fn check_filter(filter: &str) -> AppResult<()> {
    use sqlparser::dialect::MySqlDialect;
    use sqlparser::parser::Parser;
    use sqlparser::tokenizer::Token;
    let bad = |m: String| AppError::validation(format!("That isn't a filter I can use: {m}"));
    // Comments are never needed in a condition, and MariaDB runs `/*M! */` as code while the parser sees a comment.
    if filter.contains("/*") || filter.contains('#') || filter.contains("-- ") {
        return Err(bad("leave comments out".into()));
    }
    if classify::calls_effectful_for(filter, Dialect::My) {
        return Err(bad("it calls a function that acts outside the query".into()));
    }
    let dialect = MySqlDialect {};
    let mut p = Parser::new(&dialect).try_with_sql(filter).map_err(|e| bad(e.to_string()))?;
    p.parse_expr().map_err(|e| bad(e.to_string()))?;
    if p.peek_token().token != Token::EOF {
        return Err(bad("write one condition, like status = 'open' and total > 100".into()));
    }
    Ok(())
}

pub struct MyDb {
    pub conn: MyConn,
    /// Connection ids of every connection Dalan holds on this server (this one and each editor's): activity shows them
    /// but never lets them be killed.
    pub(super) own: std::sync::Mutex<Vec<u32>>,
    /// One statement scope at a time, so a read-only transaction isn't interleaved with another's.
    pub(super) gate: tokio::sync::Mutex<()>,
    pub(super) catalog: tokio::sync::Mutex<Option<Arc<SqlCatalog>>>,
}

/// An editor tab's connection (its own, so a transaction in one never touches another).
pub struct MyEditor {
    pub conn: MyConn,
    pub(super) gate: tokio::sync::Mutex<()>,
    /// A manual transaction is open on this connection.
    txn: std::sync::Mutex<Option<TxnInfo>>,
}

impl MyEditor {
    pub fn new(conn: MyConn) -> MyEditor {
        MyEditor { conn, gate: tokio::sync::Mutex::new(()), txn: std::sync::Mutex::new(None) }
    }
}

impl MyDb {
    pub fn new(conn: MyConn) -> MyDb {
        let id = conn.connection_id;
        MyDb { conn, own: std::sync::Mutex::new(vec![id]), gate: tokio::sync::Mutex::new(()), catalog: tokio::sync::Mutex::new(None) }
    }

    /// Runs `f` inside a read-only transaction that is always rolled back.
    pub(super) async fn read_only<T>(&self, f: impl AsyncFnOnce(&MyConn) -> AppResult<T>) -> AppResult<T> {
        let _scope = self.gate.lock().await;
        self.conn.query("START TRANSACTION READ ONLY", 0).await?;
        let out = f(&self.conn).await;
        let _ = self.conn.query("ROLLBACK", 0).await;
        out
    }

    pub async fn objects(&self, path: &[String]) -> AppResult<Vec<SqlObject>> {
        self.read_only(async |c| match path {
            [] => Ok(c
                .query(&format!("select schema_name from information_schema.schemata where schema_name not in {SYSTEM} order by 1"), 0)
                .await?
                .rows
                .into_iter()
                .map(|r| SqlObject { name: r[0].clone().unwrap_or_default(), kind: ObjectKind::Schema, estimate: None })
                .collect()),
            [schema] => Ok(c
                .query(&format!("select table_name, table_type, table_rows from information_schema.tables where table_schema = {} order by 1", lit(schema)), 0)
                .await?
                .rows
                .into_iter()
                .map(|r| {
                    let view = r[1].as_deref().is_some_and(|t| t.contains("VIEW"));
                    SqlObject {
                        name: r[0].clone().unwrap_or_default(),
                        kind: if view { ObjectKind::View } else { ObjectKind::Table },
                        estimate: if view { None } else { r[2].as_deref().and_then(|n| n.parse().ok()) },
                    }
                })
                .collect()),
            _ => Ok(vec![]),
        })
        .await
    }

    pub async fn table_info(&self, schema: &str, table: &str) -> AppResult<SqlTableInfo> {
        self.read_only(async |c| {
            let cols = c
                .query(&format!("select column_name, column_type, is_nullable, column_default from information_schema.columns where table_schema = {} and table_name = {} order by ordinal_position", lit(schema), lit(table)), 0)
                .await?;
            if cols.rows.is_empty() {
                return Err(AppError::not_found(format!("There is no table {schema}.{table} (it may have been dropped).")));
            }
            let pk = c
                .query(&format!("select column_name from information_schema.key_column_usage where table_schema = {} and table_name = {} and constraint_name = 'PRIMARY' order by ordinal_position", lit(schema), lit(table)), 0)
                .await?
                .rows;
            let columns = cols
                .rows
                .iter()
                .map(|r| {
                    let name = r[0].clone().unwrap_or_default();
                    let ty = r[1].clone().unwrap_or_default();
                    SqlColumn {
                        pk_position: pk.iter().position(|p| p[0].as_deref() == Some(name.as_str())).map(|i| i as u32 + 1),
                        kind: kind_of_type(&ty),
                        type_name: ty,
                        nullable: r[2].as_deref() == Some("YES"),
                        default: r[3].clone(),
                        name,
                    }
                })
                .collect();
            let fks = |rows: Vec<Vec<Option<String>>>, mine: bool| -> Vec<SqlForeignKey> {
                let mut out: Vec<SqlForeignKey> = Vec::new();
                for r in rows {
                    let g = |i: usize| r[i].clone().unwrap_or_default();
                    // (constraint, column, ref schema, ref table, ref column, schema, table)
                    let (columns, target) = (g(1), g(4));
                    let (schema, table) = if mine { (g(2), g(3)) } else { (g(5), g(6)) };
                    match out.iter_mut().find(|k| k.name == g(0) && k.schema == schema && k.table == table) {
                        Some(k) => {
                            k.columns.push(columns);
                            k.target_columns.push(target);
                        }
                        None => out.push(SqlForeignKey { name: g(0), columns: vec![columns], schema, table, target_columns: vec![target] }),
                    }
                }
                out
            };
            let out_rows = c
                .query(&format!("select constraint_name, column_name, referenced_table_schema, referenced_table_name, referenced_column_name, table_schema, table_name from information_schema.key_column_usage where table_schema = {} and table_name = {} and referenced_table_name is not null order by constraint_name, ordinal_position", lit(schema), lit(table)), 0)
                .await?
                .rows;
            let in_rows = c
                .query(&format!("select constraint_name, column_name, referenced_table_schema, referenced_table_name, referenced_column_name, table_schema, table_name from information_schema.key_column_usage where referenced_table_schema = {} and referenced_table_name = {} order by table_schema, table_name, constraint_name, ordinal_position", lit(schema), lit(table)), 0)
                .await?
                .rows;
            Ok(SqlTableInfo { columns, references: fks(out_rows, true), referenced_by: fks(in_rows, false) })
        })
        .await
    }

    pub async fn table_page(&self, req: &PageRequest) -> AppResult<SqlPage> {
        let started = Instant::now();
        let info = self.table_info(&req.schema, &req.table).await?;
        let filter = req.filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        let size = req.page_size.clamp(1, MAX_PAGE).max(if req.page_size == 0 { DEFAULT_PAGE } else { 1 });
        let shown: Vec<&SqlColumn> = if req.columns.is_empty() {
            info.columns.iter().collect()
        } else {
            req.columns
                .iter()
                .map(|n| info.columns.iter().find(|c| &c.name == n).ok_or_else(|| AppError::validation(format!("There is no column {n} in this table."))))
                .collect::<AppResult<Vec<_>>>()?
        };
        let mut pk: Vec<&SqlColumn> = info.columns.iter().filter(|c| c.pk_position.is_some()).collect();
        pk.sort_by_key(|c| c.pk_position);
        let keyset = req.sort.is_empty() && !pk.is_empty();
        let mut select: Vec<String> = shown.iter().map(|c| q(&c.name)).collect();
        if keyset {
            select.extend(pk.iter().enumerate().map(|(i, c)| format!("{} as `__k{i}`", q(&c.name))));
        }
        let mut sql = format!("select {} from {}.{}", select.join(", "), q(&req.schema), q(&req.table));
        let mut conds = Vec::new();
        if !filter.is_empty() {
            conds.push(format!("({filter})"));
        }
        let mut offset = 0u64;
        if keyset {
            if let Some(PageCursor { keys: Some(keys), .. }) = &req.cursor {
                if keys.len() != pk.len() {
                    return Err(AppError::validation("That page position is out of date. Reload the table."));
                }
                let left = pk.iter().map(|c| q(&c.name)).collect::<Vec<_>>().join(", ");
                let right = keys.iter().zip(&pk).map(|(k, c)| key_lit(c.kind, k)).collect::<Vec<_>>().join(", ");
                conds.push(format!("({left}) > ({right})"));
            }
        } else if let Some(c) = &req.cursor {
            offset = c.offset;
        }
        if !conds.is_empty() {
            sql.push_str(&format!(" where {}", conds.join(" and ")));
        }
        let order: Vec<String> = if keyset {
            pk.iter().map(|c| q(&c.name)).collect()
        } else {
            let mut o = Vec::new();
            for k in &req.sort {
                if !info.columns.iter().any(|c| c.name == k.column) {
                    return Err(AppError::validation(format!("There is no column {} to sort by.", k.column)));
                }
                o.push(format!("{} {}", q(&k.column), if k.descending { "desc" } else { "asc" }));
            }
            o.extend(pk.iter().map(|c| q(&c.name)));
            o
        };
        if !order.is_empty() {
            sql.push_str(&format!(" order by {}", order.join(", ")));
        }
        sql.push_str(&format!(" limit {}", size + 1));
        if !keyset && offset > 0 {
            sql.push_str(&format!(" offset {offset}"));
        }
        let res = self.read_only(async |c| c.query(&sql, 0).await).await?;
        let more = res.rows.len() as u32 > size;
        let page_rows = &res.rows[..res.rows.len().min(size as usize)];
        let next = more.then(|| {
            if keyset {
                let last = &page_rows[page_rows.len() - 1];
                PageCursor { keys: Some((0..pk.len()).map(|i| last[shown.len() + i].clone().unwrap_or_default()).collect()), offset: 0 }
            } else {
                PageCursor { keys: None, offset: offset + u64::from(size) }
            }
        });
        let rows = page_rows.iter().map(|r| shown.iter().enumerate().map(|(i, c)| SqlCell::new(c.kind, r[i].clone())).collect()).collect();
        Ok(SqlPage { columns: shown.into_iter().cloned().collect(), rows, next, keyset, elapsed_ms: ms(started) })
    }

    pub async fn count(&self, schema: &str, table: &str, filter: &str) -> AppResult<i64> {
        let filter = filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        let mut sql = format!("select count(*) from {}.{}", q(schema), q(table));
        if !filter.is_empty() {
            sql.push_str(&format!(" where ({filter})"));
        }
        let r = self.read_only(async |c| c.query(&sql, 0).await).await?;
        r.rows.first().and_then(|r| r[0].as_deref()).and_then(|n| n.parse().ok()).ok_or_else(|| AppError::internal("The server gave no count."))
    }

    /// One whole cell, for the cell panel.
    pub async fn cell(&self, schema: &str, table: &str, key: &[(String, String)], column: &str) -> AppResult<SqlCell> {
        let info = self.table_info(schema, table).await?;
        let col = info.columns.iter().find(|c| c.name == column).ok_or_else(|| AppError::validation(format!("There is no column {column}.")))?;
        let pk_count = info.columns.iter().filter(|c| c.pk_position.is_some()).count();
        let distinct: std::collections::HashSet<&String> = key.iter().map(|(n, _)| n).collect();
        if key.is_empty() || key.len() != pk_count || distinct.len() != key.len() {
            return Err(AppError::validation("Pick the row with its whole primary key (this table has none, or the key given is partial)."));
        }
        let mut conds = Vec::new();
        for (name, value) in key {
            if !info.columns.iter().any(|c| &c.name == name && c.pk_position.is_some()) {
                return Err(AppError::validation(format!("{name} is not part of the primary key.")));
            }
            conds.push(format!("{} = {}", q(name), lit(value)));
        }
        let sql = format!("select {} from {}.{} where {}", q(column), q(schema), q(table), conds.join(" and "));
        let r = self.read_only(async |c| c.query(&sql, 0).await).await?;
        let row = r.rows.first().ok_or_else(|| AppError::not_found("That row is gone."))?;
        Ok(match &row[0] {
            None => SqlCell { kind: CellKind::Null, v: None, cut: false },
            Some(v) => SqlCell { kind: col.kind, v: Some(v.clone()), cut: false },
        })
    }

    pub async fn catalog(&self, refresh: bool) -> AppResult<Arc<SqlCatalog>> {
        let mut cached = self.catalog.lock().await;
        if let (false, Some(c)) = (refresh, cached.as_ref()) {
            return Ok(Arc::clone(c));
        }
        let r = self
            .read_only(async |c| c.query(&format!("select table_schema, table_name, column_name from information_schema.columns where table_schema not in {SYSTEM} order by 1, 2, ordinal_position limit 20000"), 0).await)
            .await?;
        let mut tables: Vec<CatalogTable> = Vec::new();
        for row in r.rows {
            let (s, t, c) = (row[0].clone().unwrap_or_default(), row[1].clone().unwrap_or_default(), row[2].clone().unwrap_or_default());
            match tables.last_mut() {
                Some(l) if l.schema == s && l.name == t => l.columns.push(c),
                _ => tables.push(CatalogTable { schema: s, name: t, columns: vec![c] }),
            }
        }
        let catalog = Arc::new(SqlCatalog { tables });
        *cached = Some(Arc::clone(&catalog));
        Ok(catalog)
    }
}

impl MyDb {
    /// A grid commit's statements for MySQL: values are literals (the text protocol has no parameters), each update says
    /// "where the key matches and the cell is still what you saw" (`<=>` is NULL-safe equality). Binary columns can't be
    /// edited as text.
    pub async fn grid_statements(&self, edits: &GridEdits) -> AppResult<Vec<GridStatement>> {
        use super::grid::RowEdit;
        if edits.rows.is_empty() {
            return Err(AppError::validation("There are no changes to commit."));
        }
        let info = self.table_info(&edits.schema, &edits.table).await?;
        let mut pk: Vec<&SqlColumn> = info.columns.iter().filter(|c| c.pk_position.is_some()).collect();
        pk.sort_by_key(|c| c.pk_position);
        if pk.is_empty() {
            return Err(AppError::new(ErrorCode::SqlNoKey, "This table has no key, so rows can't be matched safely. Use a query instead."));
        }
        let col = |n: &str| info.columns.iter().find(|c| c.name == n).ok_or_else(|| AppError::validation(format!("There is no column {n} in this table.")));
        let table = format!("{}.{}", q(&edits.schema), q(&edits.table));
        let key_where = |row: &RowEdit| -> AppResult<Vec<String>> {
            let mut names: Vec<&str> = row.key.iter().map(|k| k.column.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            if names.len() != row.key.len() || names.len() != pk.len() || !pk.iter().all(|c| names.contains(&c.name.as_str())) {
                return Err(AppError::validation("A row was sent without its whole primary key."));
            }
            // A text key is matched byte for byte too, so a `_ci` collation can't match a second row.
            row.key.iter().map(|k| { let kind = col(&k.column)?.kind; Ok(if kind == CellKind::Int { format!("{} = {}", q(&k.column), key_lit(kind, &k.value)) } else { format!("binary cast({} as char) = binary {}", q(&k.column), lit(&k.value)) }) }).collect()
        };
        let opt = |v: &Option<String>| v.as_deref().map_or_else(|| "NULL".to_string(), lit);
        // "The cell is still what the person saw": integers as numbers; everything else by its text form compared
        // byte for byte (a FLOAT against a string is compared as a double, and a `_ci` collation would call 'abc' and
        // 'ABC' equal). `<=>` is NULL-safe.
        let same = |c: &SqlColumn, old: &Option<String>| -> String {
            match (c.kind, old) {
                (CellKind::Int, Some(v)) => format!("{} <=> {}", q(&c.name), key_lit(CellKind::Int, v)),
                _ => format!("binary cast({} as char) <=> binary {}", q(&c.name), opt(old)),
            }
        };
        let mut out = Vec::new();
        for row in &edits.rows {
            let mut conds = key_where(row)?;
            if row.delete {
                let sql = format!("delete from {table} where {}", conds.join(" and "));
                out.push(GridStatement { shown: sql.clone(), sql, params: vec![] });
                continue;
            }
            if row.changes.is_empty() {
                return Err(AppError::validation("A row was sent with nothing changed."));
            }
            let mut sets = Vec::new();
            for ch in &row.changes {
                let c = col(&ch.column)?;
                if c.kind == CellKind::Bytes {
                    return Err(AppError::validation(format!("{} holds binary data and can't be edited as text. Use a query.", c.name)));
                }
                if c.pk_position.is_some() && ch.old != ch.new {
                    return Err(AppError::validation(format!("{} is part of the key. Change it with a query.", c.name)));
                }
                sets.push(format!("{} = {}", q(&c.name), opt(&ch.new)));
                conds.push(same(c, &ch.old));
            }
            let sql = format!("update {table} set {} where {}", sets.join(", "), conds.join(" and "));
            out.push(GridStatement { shown: sql.clone(), sql, params: vec![] });
        }
        Ok(out)
    }

    /// One transaction; every statement must match exactly one row (the connection asks for matched rows, so an
    /// update to the same value still counts), or everything is rolled back.
    pub async fn grid_commit(&self, stmts: &[GridStatement]) -> AppResult<u32> {
        let _scope = self.gate.lock().await;
        self.conn.query("START TRANSACTION", 0).await?;
        let mut failure = None;
        for (i, st) in stmts.iter().enumerate() {
            match self.conn.query(&st.sql, 0).await {
                Ok(r) if r.affected == 1 => {}
                Ok(_) => {
                    failure = Some(AppError::new(ErrorCode::SqlConflict, format!("Row {} was changed or removed by someone else since you loaded it. Nothing was written. Reload the table and try again.", i + 1)));
                    break;
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        let ended = self.conn.query(if failure.is_none() { "COMMIT" } else { "ROLLBACK" }, 0).await;
        if let Some(f) = failure {
            return Err(f);
        }
        ended?;
        Ok(stmts.len() as u32)
    }
}

impl MyEditor {
    pub async fn cancel_running(&self) -> AppResult<()> {
        self.conn.cancel_running().await
    }

    pub fn has_txn(&self) -> bool {
        self.txn.lock().map(|t| t.is_some()).unwrap_or(false)
    }

    pub async fn txn_begin(&self) -> AppResult<()> {
        let _scope = self.gate.lock().await;
        if self.has_txn() {
            return Err(AppError::new(ErrorCode::SqlInTransaction, "A transaction is already open in this editor."));
        }
        self.conn.query("START TRANSACTION", 0).await?;
        if let Ok(mut t) = self.txn.lock() {
            *t = Some(TxnInfo { since: Instant::now(), statements: 0, failed: false });
        }
        Ok(())
    }

    pub async fn txn_end(&self, commit: bool) -> AppResult<()> {
        let _scope = self.gate.lock().await;
        if !self.has_txn() {
            return Ok(());
        }
        let out = self.conn.query(if commit { "COMMIT" } else { "ROLLBACK" }, 0).await.map(|_| ());
        // Over either way: a COMMIT the server refused leaves nothing the person can still commit.
        if let Ok(mut t) = self.txn.lock() {
            *t = None;
        }
        out
    }

    /// The bar's numbers. The number of locked rows comes from `information_schema.innodb_trx`, which needs the PROCESS
    /// privilege: without it the count is just left out.
    pub async fn txn_state(&self) -> AppResult<TxnState> {
        let info = self.txn.lock().ok().and_then(|t| t.clone());
        let Some(info) = info else {
            return Ok(TxnState { active: false, statements: 0, age_s: 0, failed: false, locks: 0 });
        };
        let _scope = self.gate.lock().await;
        let locks = self
            .conn
            .query("select coalesce(max(trx_rows_locked), 0) from information_schema.innodb_trx where trx_mysql_thread_id = connection_id()", 1)
            .await
            .ok()
            .and_then(|r| r.rows.first().and_then(|r| r[0].as_deref().and_then(|n| n.parse::<u64>().ok())))
            .unwrap_or(0);
        Ok(TxnState { active: true, statements: info.statements, age_s: info.since.elapsed().as_secs().min(u64::from(u32::MAX)) as u32, failed: false, locks: locks.min(u64::from(u32::MAX)) as u32 })
    }

    /// What each data-changing statement would do: run in a transaction that is rolled back. InnoDB only; a
    /// statement that calls an effectful function is not tried.
    pub async fn preview(&self, pieces: &[Piece]) -> AppResult<Vec<Option<String>>> {
        let _scope = self.gate.lock().await;
        // A rollback only undoes InnoDB's work (MyISAM and Aria commit as they go, and a trigger may write to one):
        // with such tables around, nothing is tried out.
        let risky = self
            .conn
            .query(&format!("select count(*) from information_schema.tables where table_type = 'BASE TABLE' and engine not in ('InnoDB') and table_schema not in {SYSTEM}"), 0)
            .await
            .ok()
            .and_then(|r| r.rows.first().and_then(|r| r[0].as_deref().and_then(|n| n.parse::<u64>().ok())))
            .is_none_or(|n| n > 0);
        let _ = self.conn.set_timeout(10_000).await;
        let mut out = Vec::new();
        for p in pieces {
            let first = p.text.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
            if !matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "REPLACE") {
                out.push(None);
                continue;
            }
            if risky {
                out.push(Some("Not tried out: this server has tables that aren't InnoDB, where a rollback can't undo a change.".into()));
                continue;
            }
            if classify::calls_effectful_for(&p.text, Dialect::My) {
                out.push(Some("Not tried out: it calls a function that acts outside the transaction.".into()));
                continue;
            }
            // Inside the person's own transaction even a savepoint keeps the row locks until they commit: not tried.
            let manual = self.has_txn();
            if manual {
                out.push(Some("Not tried out inside a transaction: its row locks would stay until you commit.".into()));
                continue;
            }
            self.conn.query(if manual { "SAVEPOINT dalan_dry" } else { "START TRANSACTION" }, 0).await?;
            let r = self.conn.query(&p.text, 0).await;
            let _ = self.conn.query(if manual { "ROLLBACK TO SAVEPOINT dalan_dry" } else { "ROLLBACK" }, 0).await;
            if manual {
                let _ = self.conn.query("RELEASE SAVEPOINT dalan_dry", 0).await;
            }
            out.push(Some(match r {
                Ok(r) if r.affected == 1 => "This matches 1 row.".to_string(),
                Ok(r) => format!("This matches {} rows.", r.affected),
                Err(e) => format!("The server says it would fail: {}", e.message),
            }));
        }
        Ok(out)
    }

    /// Runs the statements in order and reports through `emit`, like `PgConn::run_script`: reads in one read-only
    /// transaction, writes in auto-commit (each statement stands alone; DDL commits by itself in MySQL).
    pub async fn run_script(&self, pieces: &[Piece], opts: &RunOpts, cancel: &AtomicBool, allow_write: bool, emit: &(dyn Fn(SqlRunEvent) + Sync)) -> AppResult<()> {
        let started = Instant::now();
        let all_reads = pieces.iter().all(|p| classify::classify_for(&p.text, Dialect::My) == Class::Read);
        if !all_reads && !allow_write {
            let (i, class) = pieces.iter().enumerate().map(|(i, p)| (i, classify::classify_for(&p.text, Dialect::My))).find(|(_, c)| *c != Class::Read).unwrap_or((0, Class::Dangerous));
            emit(SqlRunEvent::Refused { index: i as u32, class, message: "This changes data or structure and hasn't been confirmed. Nothing was sent.".into() });
            emit(SqlRunEvent::Finished { elapsed_ms: ms(started) });
            return Ok(());
        }
        let _scope = self.gate.lock().await;
        let _ = self.conn.set_timeout(opts.timeout_ms).await;
        // A manual transaction is the person's: nothing is wrapped around it.
        let manual = self.has_txn();
        if all_reads && !manual {
            self.conn.query("START TRANSACTION READ ONLY", 0).await?;
        }
        for (i, p) in pieces.iter().enumerate() {
            let index = i as u32;
            if cancel.load(Ordering::SeqCst) {
                emit(SqlRunEvent::Failed { index, kind: FailKind::Cancelled, message: "Stopped.".into() });
                break;
            }
            emit(SqlRunEvent::Started { index, statement: p.text.clone(), start: p.start as u32, end: p.end as u32 });
            let t = Instant::now();
            let r: AppResult<MyResult> = self.conn.query(&p.text, opts.limit as usize).await;
            // DDL and its kin commit the open transaction by themselves; a connection that dropped has lost it.
            if manual && (r.is_ok() && implicit_commit(&p.text) || matches!(&r, Err(e) if e.code == ErrorCode::DbUnreachable)) {
                if let Ok(mut g) = self.txn.lock() {
                    *g = None;
                }
            } else if manual {
                if let Ok(mut g) = self.txn.lock() {
                    if let Some(x) = g.as_mut() {
                        x.statements += 1;
                    }
                }
            }
            match r {
                // A killed `sleep()` answers normally: the stop flag says what happened.
                Ok(_) if cancel.load(Ordering::SeqCst) && all_reads => {
                    emit(SqlRunEvent::Failed { index, kind: FailKind::Cancelled, message: "Stopped.".into() });
                    break;
                }
                Ok(res) => {
                    if !res.columns.is_empty() {
                        emit(SqlRunEvent::Columns { index, columns: res.columns.iter().map(|c| RunColumn { name: c.name.clone(), type_name: c.type_name.clone(), kind: c.kind }).collect() });
                        for chunk in res.rows.chunks(BATCH) {
                            emit(SqlRunEvent::Rows { index, rows: chunk.iter().map(|r| res.columns.iter().zip(r).map(|(c, v)| SqlCell::new(c.kind, v.clone())).collect()).collect() });
                        }
                    }
                    let affected = res.columns.is_empty().then_some(res.affected as f64);
                    emit(SqlRunEvent::Done { index, rows: res.rows.len() as u32, more: res.more, elapsed_ms: ms(t), affected });
                }
                Err(e) => {
                    let m = e.message.to_ascii_lowercase();
                    let kind = if cancel.load(Ordering::SeqCst) || m.contains("query execution was interrupted") && !m.contains("max") {
                        FailKind::Cancelled
                    } else if m.contains("maximum statement execution time") || m.contains("max_statement_time") {
                        FailKind::Timeout
                    } else {
                        FailKind::Error
                    };
                    emit(SqlRunEvent::Failed { index, kind, message: e.message.clone() });
                    if e.code == ErrorCode::DbUnreachable {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        if all_reads && !manual {
            let _ = self.conn.query("ROLLBACK", 0).await;
        }
        emit(SqlRunEvent::Finished { elapsed_ms: ms(started) });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_strings_are_quoted_for_mysql() {
        assert_eq!(q("order`s"), "`order``s`");
        assert_eq!(lit("it's a \\ test"), "'it''s a \\\\ test'");
    }

    #[test]
    fn column_types_map_to_kinds() {
        assert_eq!(kind_of_type("bigint unsigned"), CellKind::Int);
        assert_eq!(kind_of_type("DECIMAL(12,2)"), CellKind::Decimal);
        assert_eq!(kind_of_type("enum('a','b')"), CellKind::Enum);
        assert_eq!(kind_of_type("longblob"), CellKind::Bytes);
        assert_eq!(kind_of_type("varchar(40)"), CellKind::Text);
        assert_eq!(kind_of_type("json"), CellKind::Json);
    }

    #[test]
    fn a_filter_is_one_expression_in_mysql_too() {
        assert!(check_filter("status = 'open' and total > 100").is_ok());
        assert!(check_filter("`order` in (select id from refunds)").is_ok());
        assert!(check_filter("1 = 1; drop table orders").is_err());
        assert!(check_filter("status = ").is_err());
        for bad in ["a = 1 /*M! into outfile 'x' */", "a = 1 # x", "a = 1 -- x", "get_lock('a', 1) = 1", "load_file('/etc/passwd') is not null", "sys_exec('x') = 0"] {
            assert!(check_filter(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_are_compared_as_numbers_when_the_column_is_numeric() {
        assert_eq!(key_lit(CellKind::Int, "9007199254740993"), "9007199254740993");
        assert_eq!(key_lit(CellKind::Int, "1; drop"), "'1; drop'");
        assert_eq!(key_lit(CellKind::Text, "007"), "'007'");
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target, TlsMode};
    use crate::dbs::Engine;
    use super::super::pg::{Reopen, SortKey};
    use super::super::run::RunMode;
    use std::sync::Mutex;
    use zeroize::Zeroizing;

    const SEED: &str = r#"
drop view if exists d7_open;
drop table if exists d7_items, d7_orders, d7_customers, d7_nokey, d7_nums, d7_cmp;
create table d7_nums (n int primary key);
insert into d7_nums values (1);
insert into d7_nums select n + 1 from d7_nums;
insert into d7_nums select n + 2 from d7_nums;
insert into d7_nums select n + 4 from d7_nums;
insert into d7_nums select n + 8 from d7_nums;
insert into d7_nums select n + 16 from d7_nums;
insert into d7_nums select n + 32 from d7_nums;
insert into d7_nums select n + 64 from d7_nums;
insert into d7_nums select n + 128 from d7_nums;
insert into d7_nums select n + 256 from d7_nums;
insert into d7_nums select n + 512 from d7_nums;
insert into d7_nums select n + 1024 from d7_nums;
create table d7_customers (id bigint primary key, name varchar(60) not null, long_text longtext);
insert into d7_customers select n, concat('customer ', n), if(n = 1, repeat('x', 70000), null) from d7_nums where n <= 50;
create table d7_orders (id bigint primary key, customer_id bigint not null, status enum('open','paid','void') not null, total decimal(12,2), placed_at datetime, note text,
  constraint d7_orders_customer foreign key (customer_id) references d7_customers(id));
insert into d7_orders select n, 1 + n % 50, elt(1 + n % 3, 'open', 'paid', 'void'), n * 1.25, timestamp('2026-01-01 00:00:00') + interval n hour, if(n % 10 = 0, null, concat('note ', n)) from d7_nums where n <= 1200;
create table d7_items (order_id bigint, line_no int, sku varchar(40), primary key (order_id, line_no));
insert into d7_items select o.n, l.n, concat('sku-', o.n, '-', l.n) from d7_nums o, d7_nums l where o.n <= 30 and l.n <= 4;
create table d7_nokey (a int, b text);
insert into d7_nokey values (1, 'a'), (2, 'b');
create table d7_cmp (id int primary key, f float, d decimal(10,2), s varchar(20));
insert into d7_cmp values (1, 0.1, 1.50, 'abc');
create view d7_open as select * from d7_orders where status = 'open';
"#;

    /// The two tests share the seeded tables: one at a time.
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn seed(container: &str, root_pw: &str) {
        // MariaDB's image ships `mariadb`, not `mysql`.
        let client = if container.contains("mariadb") { "mariadb" } else { "mysql" };
        let mut child = std::process::Command::new("docker")
            .args(["exec", "-i", container, client, "-uroot", &format!("-p{root_pw}"), "shop"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("docker");
        use std::io::Write;
        child.stdin.take().unwrap().write_all(SEED.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success(), "seeding {container} failed");
    }

    fn reopen_to(port: u16) -> Reopen {
        std::sync::Arc::new(move || {
            Box::pin(async move {
                let t = Target { address: "127.0.0.1".into(), port, tls: false, tls_server_name: None };
                transport::direct_for(Engine::Mysql, &t).await
            })
        })
    }

    async fn conn(port: u16, pw: &str) -> MyConn {
        conn_as(port, "app", pw).await
    }

    async fn conn_as(port: u16, user: &str, pw: &str) -> MyConn {
        let stream = (reopen_to(port))().await.unwrap();
        let login = super::super::mysql::MyLogin { user: user.into(), password: Some(Zeroizing::new(pw.into())), database: Some("shop".into()), tls: TlsMode::Off, server_name: "127.0.0.1".into() };
        // MySQL 8 without TLS needs a warm cache for caching_sha2: a first sign-in over TLS warms it.
        let warm = super::super::mysql::MyLogin { tls: TlsMode::Require, ..login.clone() };
        if let Ok((c, _)) = MyConn::connect((reopen_to(port))().await.unwrap(), warm, reopen_to(port)).await {
            c.close().await;
        }
        MyConn::connect(stream, login, reopen_to(port)).await.unwrap().0
    }

    /// `(port, app password, container, root password)` for MySQL and MariaDB.
    const SERVERS: [(u16, &str, &str, &str); 2] = [(13306, "test-mysql", "dalan-test-mysql", "test-mysql"), (13307, "test-mariadb", "dalan-test-mariadb", "test-mariadb")];

    fn page(table: &str) -> PageRequest {
        PageRequest { schema: "shop".into(), table: table.into(), filter: String::new(), sort: vec![], columns: vec![], cursor: None, page_size: 500 }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_browse_the_tree_keys_pages_counts_and_whole_cells_on_both_servers() {
        let _one = ONE_AT_A_TIME.lock().await;
        for (port, pw, container, root) in SERVERS {
            seed(container, root);
            let db = MyDb::new(conn(port, pw).await);
            assert!(db.objects(&[]).await.unwrap().iter().any(|o| o.name == "shop" && o.kind == ObjectKind::Schema));
            let objs = db.objects(&["shop".into()]).await.unwrap();
            assert!(objs.iter().any(|o| o.name == "d7_orders" && o.kind == ObjectKind::Table && o.estimate.is_some()));
            assert!(objs.iter().any(|o| o.name == "d7_open" && o.kind == ObjectKind::View));
            let info = db.table_info("shop", "d7_orders").await.unwrap();
            assert!(info.columns.iter().any(|c| c.name == "id" && c.pk_position == Some(1) && c.kind == CellKind::Int));
            assert!(info.columns.iter().any(|c| c.name == "status" && c.kind == CellKind::Enum) && info.columns.iter().any(|c| c.name == "total" && c.kind == CellKind::Decimal));
            assert_eq!(info.references[0].table, "d7_customers");
            assert_eq!((info.references[0].columns.clone(), info.references[0].target_columns.clone()), (vec!["customer_id".to_string()], vec!["id".to_string()]));
            let cust = db.table_info("shop", "d7_customers").await.unwrap();
            assert!(cust.referenced_by.iter().any(|k| k.table == "d7_orders" && k.columns == ["customer_id"] && k.target_columns == ["id"]));
            let items = db.table_info("shop", "d7_items").await.unwrap();
            assert_eq!(items.columns.iter().filter_map(|c| c.pk_position).max(), Some(2));
            assert_eq!(db.table_info("shop", "nope_not_here").await.unwrap_err().code, ErrorCode::NotFound);
            // Keyset paging walks every row once.
            let (mut seen, mut cursor, mut pages) = (std::collections::HashSet::new(), None, 0);
            loop {
                let mut r = page("d7_orders");
                r.cursor = cursor.clone();
                let p = db.table_page(&r).await.unwrap();
                assert!(p.keyset);
                for row in &p.rows {
                    assert!(seen.insert(row[0].v.clone().unwrap()));
                }
                pages += 1;
                match p.next {
                    Some(n) => cursor = Some(n),
                    None => break,
                }
            }
            assert_eq!((seen.len(), pages), (1200, 3));
            // Composite keys, a sort (offset paging), a filter, a column subset, exact counts.
            let mut r = page("d7_items");
            r.page_size = 50;
            let p1 = db.table_page(&r).await.unwrap();
            r.cursor = p1.next.clone();
            let p2 = db.table_page(&r).await.unwrap();
            assert_eq!((p1.rows.len(), p2.rows.len()), (50, 50));
            let mut r = page("d7_orders");
            r.sort = vec![SortKey { column: "total".into(), descending: true }];
            r.page_size = 10;
            let p = db.table_page(&r).await.unwrap();
            assert!(!p.keyset);
            assert_eq!(p.rows[0][0].v.as_deref(), Some("1200"));
            let mut r = page("d7_orders");
            r.filter = "status = 'open' and id <= 30".into();
            r.columns = vec!["id".into(), "status".into()];
            let p = db.table_page(&r).await.unwrap();
            assert_eq!((p.rows.len(), p.columns.len()), (10, 2));
            assert_eq!(db.count("shop", "d7_orders", "status = 'open'").await.unwrap(), 400);
            assert_eq!(db.count("shop", "d7_nokey", "").await.unwrap(), 2);
            // A filter is one expression and nothing else; nothing was dropped.
            let mut r = page("d7_orders");
            r.filter = "1 = 1; drop table d7_orders".into();
            assert_eq!(db.table_page(&r).await.unwrap_err().code, ErrorCode::Validation);
            assert_eq!(db.count("shop", "d7_orders", "").await.unwrap(), 1200);
            // A table without a key: offset paging. A long text arrives cut, and whole by key.
            assert!(!db.table_page(&page("d7_nokey")).await.unwrap().keyset);
            let p = db.table_page(&page("d7_customers")).await.unwrap();
            let long = &p.rows[0][2];
            assert!(long.cut && long.v.as_ref().unwrap().chars().count() == 65_536);
            let whole = db.cell("shop", "d7_customers", &[("id".into(), "1".into())], "long_text").await.unwrap();
            assert_eq!((whole.cut, whole.v.map(|v| v.len())), (false, Some(70_000)));
            assert_eq!(db.cell("shop", "d7_items", &[("order_id".into(), "1".into())], "sku").await.unwrap_err().code, ErrorCode::Validation);
            // Completion sees the tables; a write can't happen in the read-only transaction.
            let cat = db.catalog(false).await.unwrap();
            assert!(cat.tables.iter().any(|t| t.schema == "shop" && t.name == "d7_orders" && t.columns[0] == "id"));
            let e = db.read_only(async |c| c.query("insert into d7_nokey values (9, 'x')", 0).await).await.unwrap_err();
            assert!(e.message.to_lowercase().contains("read only") || e.message.to_lowercase().contains("read-only"), "{}", e.message);
            assert_eq!(db.count("shop", "d7_nokey", "").await.unwrap(), 2);
        }
    }

    /// A customer's name as `c` sees it.
    async fn name(c: &MyConn, id: u32) -> String {
        c.query(&format!("select name from d7_customers where id = {id}"), 0).await.unwrap().rows[0][0].clone().unwrap()
    }

    /// A column with a string default that looks like a date, a comment and NOT NULL: the type changes and the rest stays.
    async fn e_modify(db: &MyDb) {
        use super::super::structure::StructureOp;
        db.conn.query("drop table if exists d7_mod", 0).await.unwrap();
        db.conn.query("create table d7_mod (id int primary key auto_increment, d varchar(20) not null default '2024-01-01' comment 'when', n int default 5)", 0).await.unwrap();
        let info = db.table_info("shop", "d7_mod").await.unwrap();
        let ddl = db.structure("shop", "d7_mod").await.unwrap().ddl;
        let (stmts, _) = super::super::myextra::statements_my("shop", "d7_mod", &[StructureOp::ChangeType { column: "d".into(), type_name: "varchar(40)".into() }, StructureOp::SetNullable { column: "n".into(), nullable: false }], None, &info, &ddl).unwrap();
        db.structure_commit(&stmts).await.unwrap();
        let after = db.structure("shop", "d7_mod").await.unwrap().ddl;
        assert!(after.contains("`d` varchar(40) NOT NULL DEFAULT '2024-01-01' COMMENT 'when'"), "{after}");
        assert!(after.lines().any(|l| l.trim_start().starts_with("`n` int") && l.contains("NOT NULL DEFAULT") && l.contains('5')), "{after}");
        assert!(after.contains("AUTO_INCREMENT"), "{after}");
        db.conn.query("drop table d7_mod", 0).await.unwrap();
    }

    async fn run(e: &MyEditor, sql: &str, limit: u32, allow: bool, cancel: &AtomicBool) -> Vec<SqlRunEvent> {
        let pieces = super::super::run::pieces_for_d(sql, RunMode::All, None, Dialect::My).unwrap();
        let out = Mutex::new(Vec::new());
        e.run_script(&pieces, &RunOpts { limit, timeout_ms: 30_000 }, cancel, allow, &|ev| out.lock().unwrap().push(ev)).await.unwrap();
        out.into_inner().unwrap()
    }
    fn rows(ev: &[SqlRunEvent], i: u32) -> usize {
        ev.iter().map(|e| match e { SqlRunEvent::Rows { index, rows } if *index == i => rows.len(), _ => 0 }).sum()
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_the_editor_runs_reads_refuses_writes_without_the_gate_and_stops_a_long_one() {
        let _one = ONE_AT_A_TIME.lock().await;
        for (port, pw, container, root) in SERVERS {
            seed(container, root);
            let no = AtomicBool::new(false);
            let e = MyEditor::new(conn(port, pw).await);
            let ev = run(&e, "select id, status, total from d7_orders order by id; select 1 as one, null as nothing", 0, false, &no).await;
            assert_eq!((rows(&ev, 0), rows(&ev, 1)), (1200, 1));
            let ev = run(&e, "select id from d7_orders order by id", 100, false, &no).await;
            assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Done { rows: 100, more: true, .. })));
            // Nothing but a read runs without the gate's yes; one write in a script and not even the reads run.
            for sql in ["insert into d7_nokey values (9, 'x')", "delete from d7_nokey", "drop table d7_nokey", "select 1; update d7_nokey set a = 1 where b = 'a'", "kill 1"] {
                let ev = run(&e, sql, 0, false, &no).await;
                assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Refused { .. })), "{sql}");
                assert!(!ev.iter().any(|x| matches!(x, SqlRunEvent::Started { .. })), "{sql}");
            }
            assert_eq!(e.conn.query("select count(*) from d7_nokey", 0).await.unwrap().rows[0][0].as_deref(), Some("2"));
            // Allowed: a write reports its rows; a failing statement ends the script with the server's words.
            let ev = run(&e, "insert into d7_nokey values (3, 'c'), (4, 'd'); delete from d7_nokey where a >= 3", 0, true, &no).await;
            assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Done { index: 0, affected: Some(a), .. } if *a == 2.0)));
            assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Done { index: 1, affected: Some(a), .. } if *a == 2.0)));
            let ev = run(&e, "select 1; select * from nope_not_here; select 3", 0, false, &no).await;
            assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Failed { index: 1, kind: FailKind::Error, message } if message.contains("nope_not_here"))));
            assert!(!ev.iter().any(|x| matches!(x, SqlRunEvent::Started { index: 2, .. })));
            // The dry run says what a change would do and changes nothing.
            let p = super::super::run::pieces_for_d("update d7_orders set note = 'x' where status = 'open'; delete from d7_nokey", RunMode::All, None, Dialect::My).unwrap();
            let notes = e.preview(&p).await.unwrap();
            assert_eq!(notes[0].as_deref(), Some("This matches 400 rows."));
            assert_eq!(e.conn.query("select count(*) from d7_orders where note = 'x'", 0).await.unwrap().rows[0][0].as_deref(), Some("0"));
            // With a table the rollback can't undo, nothing is tried out; with only InnoDB tables it is.
            e.conn.query("create table d7_myisam (a int) engine = MyISAM", 0).await.unwrap();
            let notes = e.preview(&p).await.unwrap();
            e.conn.query("drop table d7_myisam", 0).await.unwrap();
            assert!(notes[0].as_deref().unwrap().starts_with("Not tried out"), "{notes:?}");
            assert_eq!(e.preview(&p).await.unwrap()[0].as_deref(), Some("This matches 400 rows."));
            // Stop: a long statement ends within seconds and the connection works afterwards.
            let e = std::sync::Arc::new(e);
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let (e2, s2) = (e.clone(), stop.clone());
            let started = Instant::now();
            let job = tokio::spawn(async move { run(&e2, "select sleep(30)", 0, false, &s2).await });
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            stop.store(true, Ordering::SeqCst);
            e.cancel_running().await.unwrap();
            let ev = job.await.unwrap();
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            assert!(ev.iter().any(|x| matches!(x, SqlRunEvent::Failed { kind: FailKind::Cancelled, .. })), "{ev:?}");
            assert_eq!(rows(&run(&e, "select 1", 0, false, &no).await, 0), 1);
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_manual_transactions_and_grid_edits_work_on_both_servers() {
        use super::super::grid::{CellChange, KeyValue, RowEdit};
        let _one = ONE_AT_A_TIME.lock().await;
        for (port, pw, container, root) in SERVERS {
            seed(container, root);
            let no = AtomicBool::new(false);
            let e = MyEditor::new(conn(port, pw).await);
            let other = conn(port, pw).await;
            // A manual transaction holds its change until Commit or Roll back.
            e.txn_begin().await.unwrap();
            assert!(e.txn_begin().await.is_err());
            run(&e, "update d7_customers set name = 'changed' where id = 10", 0, true, &no).await;
            let st = e.txn_state().await.unwrap();
            // (The locked-row count needs the PROCESS privilege, which this user lacks.)
            assert!(st.active && st.statements == 1, "{st:?}");
            assert_eq!(name(&other, 10).await, "customer 10");
            assert_eq!(name(&e.conn, 10).await, "changed");
            // Explaining or previewing inside it leaves it open.
            let p = super::super::run::pieces_for_d("update d7_customers set name = 'x' where id = 11", RunMode::All, None, Dialect::My).unwrap();
            assert_eq!(e.preview(&p).await.unwrap()[0].as_deref(), Some("Not tried out inside a transaction: its row locks would stay until you commit."));
            assert!(e.txn_state().await.unwrap().active);
            e.txn_end(false).await.unwrap();
            assert_eq!(name(&other, 10).await, "customer 10");
            e.txn_begin().await.unwrap();
            run(&e, "update d7_customers set name = 'kept' where id = 10", 0, true, &no).await;
            e.txn_end(true).await.unwrap();
            assert_eq!(name(&other, 10).await, "kept");
            // A DDL statement commits by itself: the transaction is over and the state says so.
            e.txn_begin().await.unwrap();
            run(&e, "update d7_customers set name = 'before ddl' where id = 12", 0, true, &no).await;
            run(&e, "create table d7_tmp_ddl (a int)", 0, true, &no).await;
            assert!(!e.txn_state().await.unwrap().active);
            assert_eq!(name(&other, 12).await, "before ddl");
            e.conn.query("drop table d7_tmp_ddl", 0).await.unwrap();

            // Grid edits: updates and a delete commit together; a stale row stops everything.
            let db = MyDb::new(conn(port, pw).await);
            let edit = |id: &str, col: &str, old: Option<&str>, new: Option<&str>| RowEdit {
                key: vec![KeyValue { column: "id".into(), value: id.into() }],
                changes: vec![CellChange { column: col.into(), old: old.map(Into::into), new: new.map(Into::into) }],
                delete: false,
            };
            let g = |rows| GridEdits { schema: "shop".into(), table: "d7_customers".into(), rows };
            let stmts = db.grid_statements(&g(vec![edit("20", "name", Some("customer 20"), Some("it's \\ ok")), edit("21", "long_text", None, Some("now text")), edit("22", "name", Some("customer 22"), Some("customer 22"))])).await.unwrap();
            assert!(stmts[0].shown.contains("<=> binary 'customer 20'") && stmts[0].shown.contains("'it''s \\\\ ok'"), "{}", stmts[0].shown);
            assert_eq!(db.grid_commit(&stmts).await.unwrap(), 3);
            assert_eq!(name(&other, 20).await, "it's \\ ok");
            assert_eq!(other.query("select long_text from d7_customers where id = 21", 0).await.unwrap().rows[0][0].as_deref(), Some("now text"));
            // Someone else changed row 31 after it was loaded: nothing from this commit stays.
            other.query("update d7_customers set name = 'theirs' where id = 31", 0).await.unwrap();
            let stmts = db.grid_statements(&g(vec![edit("30", "name", Some("customer 30"), Some("mine 30")), edit("31", "name", Some("customer 31"), Some("mine 31"))])).await.unwrap();
            assert_eq!(db.grid_commit(&stmts).await.unwrap_err().code, ErrorCode::SqlConflict);
            assert_eq!(name(&other, 30).await, "customer 30");
            assert_eq!(name(&other, 31).await, "theirs");
            // A delete removes one row; again it finds none and is a conflict. No key, a changed key and a binary column are refused.
            let del = RowEdit { key: vec![KeyValue { column: "n".into(), value: "1500".into() }], changes: vec![], delete: true };
            let stmts = db.grid_statements(&GridEdits { schema: "shop".into(), table: "d7_nums".into(), rows: vec![del] }).await.unwrap();
            assert_eq!(db.grid_commit(&stmts).await.unwrap(), 1);
            assert_eq!(db.grid_commit(&stmts).await.unwrap_err().code, ErrorCode::SqlConflict);
            let nokey = GridEdits { schema: "shop".into(), table: "d7_nokey".into(), rows: vec![edit("1", "a", Some("1"), Some("2"))] };
            assert_eq!(db.grid_statements(&nokey).await.unwrap_err().code, ErrorCode::SqlNoKey);
            assert_eq!(db.grid_statements(&g(vec![edit("1", "id", Some("1"), Some("9"))])).await.unwrap_err().code, ErrorCode::Validation);
            // Floats and decimals are matched by their text, and a case-only change by someone else is a conflict.
            let cmp = |rows| GridEdits { schema: "shop".into(), table: "d7_cmp".into(), rows };
            let ok = db.grid_statements(&cmp(vec![edit("1", "f", Some("0.1"), Some("0.2")), edit("1", "d", Some("1.50"), Some("2.50"))])).await.unwrap();
            assert_eq!(db.grid_commit(&ok).await.unwrap(), 2);
            other.query("update d7_cmp set s = 'ABC' where id = 1", 0).await.unwrap();
            let stale = db.grid_statements(&cmp(vec![edit("1", "s", Some("abc"), Some("new"))])).await.unwrap();
            assert_eq!(db.grid_commit(&stale).await.unwrap_err().code, ErrorCode::SqlConflict, "{}", stale[0].shown);
            // A leading comment doesn't hide a DDL statement from the transaction bookkeeping.
            assert!(implicit_commit("/* x */ create table t (a int)") && !implicit_commit("create temporary table t (a int)") && implicit_commit("set autocommit = 1") && !implicit_commit("set @a = 1"));
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_structure_explain_and_diagram_on_both_servers() {
        use super::super::structure::StructureOp;
        let _one = ONE_AT_A_TIME.lock().await;
        for (port, pw, container, root) in SERVERS {
            seed(container, root);
            let db = MyDb::new(conn(port, pw).await);
            let s = db.structure("shop", "d7_orders").await.unwrap();
            assert!(s.columns.iter().any(|c| c.name == "total" && c.type_name.starts_with("decimal(12,2)") && c.nullable));
            assert!(s.constraints.iter().any(|c| c.kind == "p" && c.columns == ["id"]));
            let fk = s.constraints.iter().find(|c| c.kind == "f").expect("a foreign key");
            assert!(fk.definition.contains("REFERENCES shop.d7_customers(id)"));
            assert!(s.indexes.iter().any(|i| i.primary && i.unique) && s.ddl.to_lowercase().starts_with("create table"));
            assert!(s.stats.table_bytes > 0);
            // Apply: statements one by one, then the structure shows them; a failing second statement keeps the first.
            let info = db.table_info("shop", "d7_nokey").await.unwrap();
            let add = |n: &str| StructureOp::AddColumn { name: n.into(), type_name: "int".into(), nullable: true, default: Some("0".into()) };
            let (stmts, _) = super::super::myextra::statements_my("shop", "d7_nokey", &[add("extra"), StructureOp::CreateIndex { name: "d7_extra_idx".into(), columns: vec!["extra".into()], unique: false }], None, &info, &db.structure("shop", "d7_nokey").await.unwrap().ddl).unwrap();
            db.structure_commit(&stmts).await.unwrap();
            let s = db.structure("shop", "d7_nokey").await.unwrap();
            assert!(s.columns.iter().any(|c| c.name == "extra") && s.indexes.iter().any(|i| i.name == "d7_extra_idx"));
            let (stmts, w) = super::super::myextra::statements_my("shop", "d7_nokey", &[add("second"), add("extra")], None, &info, &db.structure("shop", "d7_nokey").await.unwrap().ddl).unwrap();
            assert!(w[0].contains("can't undo"));
            let err = db.structure_commit(&stmts).await.unwrap_err();
            assert!(err.message.contains("Statement 2 failed") && err.message.contains("already applied"), "{}", err.message);
            assert!(db.structure("shop", "d7_nokey").await.unwrap().columns.iter().any(|c| c.name == "second"));
            let info = db.table_info("shop", "d7_nokey").await.unwrap();
            let (stmts, _) = super::super::myextra::statements_my("shop", "d7_nokey", &[StructureOp::DropIndex { name: "d7_extra_idx".into() }, StructureOp::DropColumn { name: "extra".into() }, StructureOp::DropColumn { name: "second".into() }], None, &info, &db.structure("shop", "d7_nokey").await.unwrap().ddl).unwrap();
            db.structure_commit(&stmts).await.unwrap();
            assert!(!db.structure("shop", "d7_nokey").await.unwrap().columns.iter().any(|c| c.name == "extra"));

            // `modify` keeps what the column already has: change the type and nullability of a column with a default and a comment.
            e_modify(&db).await;
            // Explain: a plan; with timings on MySQL; MariaDB says so.
            let e = MyEditor::new(conn(port, pw).await);
            let plain = e.explain("select o.id, c.name from d7_orders o join d7_customers c on c.id = o.customer_id where o.total > 100", false, 30_000).await.unwrap();
            assert!(!plain.analyzed && !plain.root.children.is_empty(), "{plain:?}");
            if port == 13306 {
                let timed = e.explain("select * from d7_orders where note like 'note 1%'", true, 30_000).await.unwrap();
                assert!(timed.analyzed && timed.root.actual_rows.is_some());
            } else {
                assert!(e.explain("select 1", true, 30_000).await.is_err());
            }
            assert_eq!(e.conn.query("select 1", 0).await.unwrap().rows[0][0].as_deref(), Some("1"));

            // Diagram: foreign keys out from orders.
            let d = db.diagram(&[("shop".into(), "d7_orders".into())], 1).await.unwrap();
            let names: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
            assert!(names.contains(&"d7_orders") && names.contains(&"d7_customers"), "{names:?}");
            assert!(d.edges.iter().any(|e| e.from_table == "d7_orders" && e.to_table == "d7_customers" && e.from_columns == ["customer_id"]));
            assert!(d.tables.iter().find(|t| t.name == "d7_orders").unwrap().columns.iter().any(|c| c.name == "customer_id" && c.fk));
            assert!(d.total_tables >= 4);
            assert!(db.diagram(&[("shop".into(), "nope_not_here".into())], 1).await.is_err());
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_activity_lists_sessions_and_ends_another_clients_session() {
        use super::super::activity::ActivityView;
        let _one = ONE_AT_A_TIME.lock().await;
        for (port, _pw, container, root) in SERVERS {
            seed(container, root);
            // The PROCESS privilege shows other people's sessions: root has it.
            let db = MyDb::new(conn_as(port, "root", root).await);
            let a = db.activity(ActivityView::Sessions).await.unwrap();
            assert!(a.tiles.max_connections > 0 && a.tiles.connections >= 1);
            assert!(a.sessions.iter().any(|s| s.mine), "Dalan's own connection is marked");
            for v in [ActivityView::Locks, ActivityView::Slow, ActivityView::Tables, ActivityView::Roles, ActivityView::Replication] {
                let t = db.activity(v).await.unwrap().table.unwrap();
                assert!(!t.columns.is_empty() && t.rows.iter().all(|r| r.len() == t.columns.len()), "{v:?}");
            }
            let client = if container.contains("mariadb") { "mariadb" } else { "mysql" };
            let mut child = std::process::Command::new("docker")
                .args(["exec", container, client, "-uroot", &format!("-p{root}"), "-e", "select sleep(60)"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let mut found = None;
            for _ in 0..30 {
                let a = db.activity(ActivityView::Sessions).await.unwrap();
                found = a.sessions.iter().find(|s| s.query.contains("sleep(60)") && !s.mine).map(|s| s.pid);
                if found.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            let id = found.expect("the other client showed up") as u32;
            let (user, _, query, owner) = db.session_info(id).await.unwrap().unwrap();
            assert_eq!(user, "root");
            assert!(query.contains("sleep(60)"));
            // Dalan's own connections can't be touched, and a connection that now belongs to someone else is left alone.
            assert!(db.session_info(db.conn.connection_id).await.unwrap().is_none());
            assert_eq!(db.signal_session(id, true, "someone@else").await.unwrap(), None);
            assert_eq!(db.signal_session(id, true, &owner).await.unwrap(), Some(true));
            assert!(!child.wait().unwrap().success(), "the session was ended");
            assert_eq!(db.signal_session(id, true, &owner).await.unwrap(), None, "gone now");
        }
    }
}
