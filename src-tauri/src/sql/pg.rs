//! PostgreSQL (D2, S36 read side): the connection, the catalog and a table's pages. Everything here only reads, and
//! every table page runs inside `BEGIN READ ONLY`, so a filter that smuggles in a function that writes is stopped by
//! the server itself (spec section 3, "second lock").

use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Client, NoTls};
use tokio_rustls::rustls;
use zeroize::Zeroizing;

use super::values::{CellKind, SqlCell};
use crate::dbs::transport::{tls_client_config, BoxStream, TlsMode};
use crate::error::{AppError, AppResult, ErrorCode};

/// A statement that runs longer than this is stopped by the server.
const STATEMENT_TIMEOUT_MS: u32 = 30_000;
/// Rows per page unless the window asks for another size.
pub const DEFAULT_PAGE: u32 = 200;
pub const MAX_PAGE: u32 = 1000;

pub struct PgConn {
    client: Client,
    /// One statement scope at a time: a table page's read-only transaction must not interleave with another's.
    gate: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlServerInfo {
    pub engine: String,
    pub version: String,
    pub database: String,
    pub user: String,
    pub latency_ms: u32,
}

/// Turns the driver's error into ours. The server's own words are kept as written.
pub fn map_error(e: &tokio_postgres::Error) -> AppError {
    if let Some(db) = e.as_db_error() {
        let code = db.code().code();
        if code == "28P01" || code == "28000" {
            return AppError::new(
                ErrorCode::DbAuthFailed,
                "Wrong username or password. PostgreSQL refused the sign-in.",
            );
        }
        return AppError::new(ErrorCode::DbRefused, db.message().to_string());
    }
    let text = format!("{e:?}");
    if text.contains("UnknownIssuer")
        || text.contains("NotValidForName")
        || text.contains("Expired")
        || text.contains("invalid peer certificate")
    {
        return AppError::new(
            ErrorCode::DbTlsUntrusted,
            "This server's certificate isn't trusted by this computer, or is for another name.",
        );
    }
    AppError::new(
        ErrorCode::DbUnreachable,
        "The connection to PostgreSQL closed before it answered. Is this really a PostgreSQL port?",
    )
}

pub struct Login {
    pub user: String,
    pub password: Option<Zeroizing<String>>,
    pub database: Option<String>,
    pub tls: TlsMode,
    /// The name the certificate must be for (Verify).
    pub server_name: String,
}

impl PgConn {
    /// Signs in over `stream` (direct or through a host). `on_close` runs when the connection ends.
    pub async fn connect(
        stream: BoxStream,
        login: Login,
        on_close: impl FnOnce(Option<String>) + Send + 'static,
    ) -> AppResult<(PgConn, SqlServerInfo)> {
        let started = Instant::now();
        let mut cfg = tokio_postgres::Config::new();
        cfg.user(&login.user).application_name("Dalan");
        if let Some(p) = &login.password {
            cfg.password(p.as_bytes());
        }
        if let Some(d) = login.database.as_deref().filter(|d| !d.is_empty()) {
            cfg.dbname(d);
        }
        let client = match login.tls {
            TlsMode::Off => {
                cfg.ssl_mode(SslMode::Disable);
                let (client, connection) = cfg
                    .connect_raw(stream, NoTls)
                    .await
                    .map_err(|e| map_error(&e))?;
                spawn_connection(connection, on_close);
                client
            }
            mode => {
                // PostgreSQL starts TLS inside its own protocol: ask first, then handshake on the same stream.
                let tls = start_tls(stream, mode, &login.server_name).await?;
                cfg.ssl_mode(SslMode::Disable);
                let (client, connection) = cfg
                    .connect_raw(tls, NoTls)
                    .await
                    .map_err(|e| map_error(&e))?;
                spawn_connection(connection, on_close);
                client
            }
        };
        let row = client
            .query_one(
                "select version(), current_database()::text, current_user::text",
                &[],
            )
            .await
            .map_err(|e| map_error(&e))?;
        let info = SqlServerInfo {
            engine: "postgres".into(),
            version: row.get(0),
            database: row.get(1),
            user: row.get(2),
            latency_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        };
        Ok((
            PgConn {
                client,
                gate: tokio::sync::Mutex::new(()),
            },
            info,
        ))
    }

    /// Runs `body` inside a read-only transaction that is always rolled back.
    async fn read_only<T>(
        &self,
        body: impl AsyncFnOnce(&Client) -> Result<T, tokio_postgres::Error>,
    ) -> AppResult<T> {
        let _scope = self.gate.lock().await;
        self.client
            .batch_execute(&format!(
                "BEGIN READ ONLY; SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MS}"
            ))
            .await
            .map_err(|e| map_error(&e))?;
        let out = body(&self.client).await;
        // Whatever happened, the transaction ends (an aborted one can only be rolled back).
        let _ = self.client.batch_execute("ROLLBACK").await;
        out.map_err(|e| map_error(&e))
    }
}

/// `SSLRequest`, the server's one-byte answer, then the TLS handshake. Done here (not by the driver's TLS glue) so the
/// certificate problems come out as our own codes.
async fn start_tls(
    mut stream: BoxStream,
    mode: TlsMode,
    server_name: &str,
) -> AppResult<BoxStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let unreachable = |what: &str| AppError::new(ErrorCode::DbUnreachable, what.to_string());
    stream
        .write_all(&[0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f])
        .await
        .map_err(|_| unreachable("The connection to PostgreSQL closed before it answered."))?;
    let mut answer = [0u8; 1];
    stream
        .read_exact(&mut answer)
        .await
        .map_err(|_| unreachable("The connection to PostgreSQL closed before it answered."))?;
    if answer[0] != b'S' {
        return Err(AppError::new(
            ErrorCode::DbRefused,
            "This server doesn't offer TLS. Turn TLS off for this database, or ask for it to be enabled.",
        ));
    }
    let name = rustls::pki_types::ServerName::try_from(server_name.to_string()).map_err(|_| {
        AppError::validation(format!("{server_name} can't be used as a TLS server name."))
    })?;
    let connector = tokio_rustls::TlsConnector::from(tls_client_config(mode)?);
    let tls = tokio::time::timeout(
        crate::dbs::transport::CONNECT_TIMEOUT,
        connector.connect(name, stream),
    )
    .await
    .map_err(|_| unreachable("The TLS handshake took too long. Is this port really TLS?"))?
    .map_err(|e| crate::dbs::transport::tls_error(crate::dbs::Engine::Postgres, &e, server_name))?;
    Ok(Box::new(tls))
}

fn spawn_connection<S, T>(
    connection: tokio_postgres::Connection<S, T>,
    on_close: impl FnOnce(Option<String>) + Send + 'static,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio_postgres::tls::TlsStream + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        on_close(connection.await.err().map(|e| e.to_string()));
    });
}

/// A name in double quotes, doubled inside: safe to put in a statement.
pub fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

// ---- The catalog

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Schema,
    Table,
    View,
    Matview,
    Foreign,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlObject {
    pub name: String,
    pub kind: ObjectKind,
    /// The planner's row estimate (tables and materialized views); `None` when the server has none yet.
    #[specta(type = Option<f64>)]
    pub estimate: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlColumn {
    pub name: String,
    /// `format_type`: `character varying(40)`, `timestamp with time zone`.
    pub type_name: String,
    pub kind: CellKind,
    pub nullable: bool,
    pub default: Option<String>,
    /// Position in the primary key, from 1; `None` when the column is not in it.
    pub pk_position: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub schema: String,
    pub table: String,
    pub target_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlTableInfo {
    pub columns: Vec<SqlColumn>,
    /// Foreign keys of this table, pointing out.
    pub references: Vec<SqlForeignKey>,
    /// Foreign keys of other tables that point here (`columns` are theirs, `target_columns` ours).
    pub referenced_by: Vec<SqlForeignKey>,
}

impl PgConn {
    /// The lazy tree: `[]` lists schemas, `[schema]` lists its tables and views.
    pub async fn objects(&self, path: &[String]) -> AppResult<Vec<SqlObject>> {
        match path {
            [] => {
                let rows = self
                    .client
                    .query(
                        "select nspname::text from pg_namespace
                         where nspname not like 'pg\\_%' and nspname <> 'information_schema' order by 1",
                        &[],
                    )
                    .await
                    .map_err(|e| map_error(&e))?;
                Ok(rows
                    .iter()
                    .map(|r| SqlObject {
                        name: r.get(0),
                        kind: ObjectKind::Schema,
                        estimate: None,
                    })
                    .collect())
            }
            [schema] => {
                let rows = self
                    .client
                    .query(
                        "select c.relname::text, c.relkind::text, c.reltuples::float8
                         from pg_class c join pg_namespace n on n.oid = c.relnamespace
                         where n.nspname = $1 and c.relkind in ('r','p','v','m','f') order by 1",
                        &[schema],
                    )
                    .await
                    .map_err(|e| map_error(&e))?;
                Ok(rows
                    .iter()
                    .map(|r| {
                        let kind = match r.get::<_, String>(1).as_str() {
                            "v" => ObjectKind::View,
                            "m" => ObjectKind::Matview,
                            "f" => ObjectKind::Foreign,
                            _ => ObjectKind::Table,
                        };
                        let est: f64 = r.get(2);
                        SqlObject {
                            name: r.get(0),
                            kind,
                            estimate: (est >= 0.0
                                && matches!(kind, ObjectKind::Table | ObjectKind::Matview))
                            .then_some(est as i64),
                        }
                    })
                    .collect())
            }
            _ => Ok(Vec::new()),
        }
    }

    pub async fn table_info(&self, schema: &str, table: &str) -> AppResult<SqlTableInfo> {
        let rows = self
            .client
            .query(
                "select a.attname::text, format_type(a.atttypid, a.atttypmod), t.typname::text, t.typcategory::text,
                        not a.attnotnull, pg_get_expr(d.adbin, d.adrelid),
                        array_position(i.indkey::int2[], a.attnum)::int4
                 from pg_attribute a
                 join pg_class c on c.oid = a.attrelid
                 join pg_namespace n on n.oid = c.relnamespace
                 join pg_type t on t.oid = a.atttypid
                 left join pg_attrdef d on d.adrelid = a.attrelid and d.adnum = a.attnum
                 left join pg_index i on i.indrelid = c.oid and i.indisprimary
                 where n.nspname = $1 and c.relname = $2 and a.attnum > 0 and not a.attisdropped
                 order by a.attnum",
                &[&schema, &table],
            )
            .await
            .map_err(|e| map_error(&e))?;
        if rows.is_empty() {
            return Err(AppError::not_found(format!(
                "There is no table {schema}.{table} (it may have been dropped)."
            )));
        }
        let mut columns: Vec<SqlColumn> = rows
            .iter()
            .map(|r| {
                let pos: Option<i32> = r.get(6);
                SqlColumn {
                    name: r.get(0),
                    type_name: r.get(1),
                    kind: CellKind::of_pg(&r.get::<_, String>(2), &r.get::<_, String>(3)),
                    nullable: r.get(4),
                    default: r.get(5),
                    // `indkey` counts from 0 as an array; ours counts from 1.
                    pk_position: pos.map(|p| p.max(0) as u32 + 1),
                }
            })
            .collect();
        // Primary key columns in key order, ranks 1..n whatever the array's base was.
        let mut keyed: Vec<usize> = (0..columns.len())
            .filter(|i| columns[*i].pk_position.is_some())
            .collect();
        keyed.sort_by_key(|i| columns[*i].pk_position);
        for (rank, i) in keyed.into_iter().enumerate() {
            columns[i].pk_position = Some(rank as u32 + 1);
        }
        let references = self.foreign_keys(schema, table, true).await?;
        let referenced_by = self.foreign_keys(schema, table, false).await?;
        Ok(SqlTableInfo {
            columns,
            references,
            referenced_by,
        })
    }

    async fn foreign_keys(
        &self,
        schema: &str,
        table: &str,
        outgoing: bool,
    ) -> AppResult<Vec<SqlForeignKey>> {
        // `outgoing`: this table's own constraints. Otherwise the constraints of tables that point here.
        let filter = if outgoing {
            "n.nspname = $1 and c.relname = $2"
        } else {
            "fn.nspname = $1 and fc.relname = $2"
        };
        let sql = format!(
            "select con.conname::text, n.nspname::text, c.relname::text, fn.nspname::text, fc.relname::text,
                    (select array_agg(a.attname::text order by k.ord) from unnest(con.conkey) with ordinality k(attnum, ord)
                       join pg_attribute a on a.attrelid = con.conrelid and a.attnum = k.attnum),
                    (select array_agg(a.attname::text order by k.ord) from unnest(con.confkey) with ordinality k(attnum, ord)
                       join pg_attribute a on a.attrelid = con.confrelid and a.attnum = k.attnum)
             from pg_constraint con
             join pg_class c on c.oid = con.conrelid join pg_namespace n on n.oid = c.relnamespace
             join pg_class fc on fc.oid = con.confrelid join pg_namespace fn on fn.oid = fc.relnamespace
             where con.contype = 'f' and {filter} order by 1"
        );
        let rows = self
            .client
            .query(&sql, &[&schema, &table])
            .await
            .map_err(|e| map_error(&e))?;
        Ok(rows
            .iter()
            .map(|r| {
                let (own_schema, own_table): (String, String) = (r.get(1), r.get(2));
                let (to_schema, to_table): (String, String) = (r.get(3), r.get(4));
                let (cols, target): (Vec<String>, Vec<String>) = (r.get(5), r.get(6));
                if outgoing {
                    SqlForeignKey {
                        name: r.get(0),
                        columns: cols,
                        schema: to_schema,
                        table: to_table,
                        target_columns: target,
                    }
                } else {
                    SqlForeignKey {
                        name: r.get(0),
                        columns: cols,
                        schema: own_schema,
                        table: own_table,
                        target_columns: target,
                    }
                }
            })
            .collect())
    }
}

// ---- A table's pages

#[derive(Debug, Clone, Deserialize, Type)]
pub struct SortKey {
    pub column: String,
    pub descending: bool,
}

/// Where the next page starts. Opaque to the window: it hands back what the last page gave it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PageCursor {
    /// Keyset paging: the primary key of the last row, as text.
    pub keys: Option<Vec<String>>,
    /// Offset paging (a sort was chosen, or the table has no primary key).
    #[specta(type = f64)]
    pub offset: u64,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct PageRequest {
    pub schema: String,
    pub table: String,
    /// A SQL expression for `where`, typed by the user. Empty: no filter.
    pub filter: String,
    pub sort: Vec<SortKey>,
    /// The columns to show; empty: all.
    pub columns: Vec<String>,
    pub cursor: Option<PageCursor>,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SqlPage {
    pub columns: Vec<SqlColumn>,
    pub rows: Vec<Vec<SqlCell>>,
    /// `None` on the last page.
    pub next: Option<PageCursor>,
    /// True when rows are matched by primary key (stable while rows change).
    pub keyset: bool,
    pub elapsed_ms: u32,
}

/// Checks the filter is one expression and nothing else (no second statement, no trailing text).
pub fn check_filter(filter: &str) -> AppResult<()> {
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;
    use sqlparser::tokenizer::Token;
    let dialect = PostgreSqlDialect {};
    let bad = |m: String| AppError::validation(format!("That isn't a filter I can use: {m}"));
    let mut p = Parser::new(&dialect)
        .try_with_sql(filter)
        .map_err(|e| bad(e.to_string()))?;
    p.parse_expr().map_err(|e| bad(e.to_string()))?;
    if p.peek_token().token != Token::EOF {
        return Err(bad(
            "write one condition, like status = 'open' and total > 100".into(),
        ));
    }
    Ok(())
}

impl PgConn {
    pub async fn table_page(&self, req: &PageRequest) -> AppResult<SqlPage> {
        let started = Instant::now();
        let info = self.table_info(&req.schema, &req.table).await?;
        let filter = req.filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        let size = req.page_size.clamp(1, MAX_PAGE);
        let shown: Vec<&SqlColumn> = if req.columns.is_empty() {
            info.columns.iter().collect()
        } else {
            let mut v = Vec::new();
            for name in &req.columns {
                v.push(
                    info.columns
                        .iter()
                        .find(|c| &c.name == name)
                        .ok_or_else(|| {
                            AppError::validation(format!(
                                "There is no column {name} in this table."
                            ))
                        })?,
                );
            }
            v
        };
        let mut pk: Vec<&SqlColumn> = info
            .columns
            .iter()
            .filter(|c| c.pk_position.is_some())
            .collect();
        pk.sort_by_key(|c| c.pk_position);
        let keyset = req.sort.is_empty() && !pk.is_empty();

        // The select list: each shown column as text (cut), then the key columns for the next cursor.
        let mut select: Vec<String> = shown
            .iter()
            .enumerate()
            .map(|(i, c)| {
                // Aliased c0, c1, ...: an output column named like a table column would win in ORDER BY, and the
                // page would be sorted by the text form (1, 10, 100, 2, ...) instead of by the column.
                format!(
                    "left({}::text, {}) as \"c{i}\"",
                    quote(&c.name),
                    super::values::CUT_AT + 1
                )
            })
            .collect();
        if keyset {
            for (i, c) in pk.iter().enumerate() {
                select.push(format!("{}::text as \"__k{i}\"", quote(&c.name)));
            }
        }
        let mut sql = format!(
            "select {} from {}.{}",
            select.join(", "),
            quote(&req.schema),
            quote(&req.table)
        );
        let mut conds: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();
        if !filter.is_empty() {
            conds.push(format!("({filter})"));
        }
        let mut offset = 0u64;
        if keyset {
            if let Some(PageCursor {
                keys: Some(keys), ..
            }) = &req.cursor
            {
                if keys.len() != pk.len() {
                    return Err(AppError::validation(
                        "That page position is out of date. Reload the table.",
                    ));
                }
                let left: Vec<String> = pk.iter().map(|c| quote(&c.name)).collect();
                let right: Vec<String> = pk
                    .iter()
                    .enumerate()
                    // The type name comes from the catalog (`format_type`), not from the window.
                    .map(|(i, c)| format!("(${})::text::{}", i + 1, c.type_name))
                    .collect();
                conds.push(format!("({}) > ({})", left.join(", "), right.join(", ")));
                params = keys.clone();
            }
        } else if let Some(c) = &req.cursor {
            offset = c.offset;
        }
        if !conds.is_empty() {
            sql.push_str(&format!(" where {}", conds.join(" and ")));
        }
        let order: Vec<String> = if keyset {
            pk.iter().map(|c| quote(&c.name)).collect()
        } else {
            let mut o = Vec::new();
            for k in &req.sort {
                if !info.columns.iter().any(|c| c.name == k.column) {
                    return Err(AppError::validation(format!(
                        "There is no column {} to sort by.",
                        k.column
                    )));
                }
                o.push(format!(
                    "{} {}",
                    quote(&k.column),
                    if k.descending { "desc" } else { "asc" }
                ));
            }
            // Offset paging needs a total order: the primary key breaks ties; without one the server's order is used.
            o.extend(pk.iter().map(|c| quote(&c.name)));
            o
        };
        if !order.is_empty() {
            sql.push_str(&format!(" order by {}", order.join(", ")));
        }
        sql.push_str(&format!(" limit {}", size + 1));
        if !keyset && offset > 0 {
            sql.push_str(&format!(" offset {offset}"));
        }

        let rows = self
            .read_only(async |c| {
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                    .iter()
                    .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();
                c.query(&sql, &refs).await
            })
            .await?;
        let more = rows.len() as u32 > size;
        let page_rows = &rows[..rows.len().min(size as usize)];
        let next = more.then(|| {
            if keyset {
                let last = &page_rows[page_rows.len() - 1];
                PageCursor {
                    keys: Some(
                        (0..pk.len())
                            .map(|i| last.get::<_, String>(shown.len() + i))
                            .collect(),
                    ),
                    offset: 0,
                }
            } else {
                PageCursor {
                    keys: None,
                    offset: offset + u64::from(size),
                }
            }
        });
        let out = page_rows
            .iter()
            .map(|r| {
                shown
                    .iter()
                    .enumerate()
                    .map(|(i, c)| SqlCell::new(c.kind, r.get::<_, Option<String>>(i)))
                    .collect()
            })
            .collect();
        Ok(SqlPage {
            columns: shown.into_iter().cloned().collect(),
            rows: out,
            next,
            keyset,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        })
    }

    /// `count(*)` with the same filter: "Count exactly".
    pub async fn count(&self, schema: &str, table: &str, filter: &str) -> AppResult<i64> {
        let filter = filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        let mut sql = format!("select count(*) from {}.{}", quote(schema), quote(table));
        if !filter.is_empty() {
            sql.push_str(&format!(" where ({filter})"));
        }
        self.read_only(async |c| Ok(c.query_one(&sql, &[]).await?.get::<_, i64>(0)))
            .await
    }

    /// One whole cell, for the cell panel. `key` is the row's primary key as (column, text) pairs.
    pub async fn cell(
        &self,
        schema: &str,
        table: &str,
        key: &[(String, String)],
        column: &str,
    ) -> AppResult<SqlCell> {
        let info = self.table_info(schema, table).await?;
        let col = info
            .columns
            .iter()
            .find(|c| c.name == column)
            .ok_or_else(|| AppError::validation(format!("There is no column {column}.")))?;
        let mut conds = Vec::new();
        let mut params: Vec<String> = Vec::new();
        for (i, (name, value)) in key.iter().enumerate() {
            let kc = info
                .columns
                .iter()
                .find(|c| &c.name == name && c.pk_position.is_some())
                .ok_or_else(|| {
                    AppError::validation(format!("{name} is not part of the primary key."))
                })?;
            conds.push(format!(
                "{} = (${})::text::{}",
                quote(name),
                i + 1,
                kc.type_name
            ));
            params.push(value.clone());
        }
        if conds.is_empty() {
            return Err(AppError::validation(
                "This table has no key, so one row can't be picked out.",
            ));
        }
        let sql = format!(
            "select {}::text from {}.{} where {}",
            quote(column),
            quote(schema),
            quote(table),
            conds.join(" and ")
        );
        let row = self
            .read_only(async |c| {
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                    .iter()
                    .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();
                c.query_opt(&sql, &refs).await
            })
            .await?
            .ok_or_else(|| AppError::not_found("That row is gone."))?;
        // The whole value: no cut.
        let text: Option<String> = row.get(0);
        Ok(SqlCell {
            kind: if text.is_some() {
                col.kind
            } else {
                CellKind::Null
            },
            v: text,
            cut: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_is_one_expression() {
        assert!(check_filter("status = 'open' and total > 100").is_ok());
        assert!(check_filter("id in (select id from refunds)").is_ok());
        assert!(check_filter("placed_at > now() - interval '7 days'").is_ok());
        assert!(check_filter("1 = 1; drop table orders").is_err());
        assert!(check_filter("status = ").is_err());
        assert!(check_filter("1 = 1 order by id").is_err());
    }

    #[test]
    fn names_are_quoted_with_their_quotes_doubled() {
        assert_eq!(quote("orders"), "\"orders\"");
        assert_eq!(quote("we\"ird"), "\"we\"\"ird\"");
    }
}
