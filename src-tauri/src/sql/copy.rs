//! Copy between databases (D13, S49, F32; replica/databases/restore-copy.md §3). Part 1: the same engine on both
//! sides (PostgreSQL → PostgreSQL, MySQL/MariaDB → MySQL/MariaDB, MongoDB → MongoDB).
//!
//! The data flows through Dalan: a connection of its own to the source (by the source tab's route) reads in one
//! consistent snapshot, and one to the target (by the target tab's route) writes, so the two servers never need to
//! reach each other. Order: tables without keys → each table's rows in one target transaction (MongoDB has none) →
//! primary and unique keys, indexes, foreign keys, sequences → views. Stop finishes nothing half way: the table being
//! loaded is rolled back, the ones before it stay, and the report says which.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bson::{doc, Bson, Document};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::mydb::{lit, q};
use super::pg::{map_error, quote, PgConn};
use super::RawConn;
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::mongo::conn::MongoConn;

/// What happens to a table that already exists at the target (§3 "Existing tables at the target").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CopyMode {
    /// Stop if it exists (the plan says so before anything runs).
    Create,
    /// Drop it and create it again (typed).
    Recreate,
    /// Empty it, then load (typed).
    Empty,
    /// Add the rows.
    Append,
    /// Insert or replace by primary key (typed).
    Upsert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CopyContent {
    All,
    Schema,
    Data,
}

/// One table, view or collection to copy. `schema` is the PostgreSQL schema, the MySQL database or the MongoDB
/// database it is in at the source.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CopyObject {
    pub schema: String,
    pub name: String,
    /// `table`, `view` or `collection`.
    pub kind: String,
    /// Only these rows (a condition, checked like the grid's; for MongoDB a JSON filter).
    #[serde(default)]
    pub filter: Option<String>,
    /// At most this many rows (a sample).
    #[serde(default)]
    pub limit: Option<f64>,
    /// Another name at the target.
    #[serde(default)]
    pub rename: Option<String>,
    /// Between PostgreSQL and MySQL/MariaDB: target types the person chose in place of the mapping's.
    #[serde(default)]
    pub types: Option<Vec<CopyColumnType>>,
    /// Between PostgreSQL and MySQL/MariaDB: the `create table` the person edited (checked again before it runs).
    #[serde(default)]
    pub ddl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CopyColumnType {
    pub name: String,
    pub target_type: String,
}

/// One column of a table copied between PostgreSQL and MySQL/MariaDB, for the mapping step.
#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyColumn {
    pub name: String,
    pub source_type: String,
    pub target_type: String,
    /// What is lost, in words (`None`: nothing).
    pub lossy: Option<String>,
}

/// Something the copy leaves out, with its source DDL to copy by hand.
#[derive(Debug, Clone, Serialize, Type)]
pub struct NotCopied {
    pub name: String,
    pub kind: String,
    pub ddl: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct CopyRequest {
    /// The open tab to copy into.
    pub target_conn_id: String,
    /// Create this database on the target's server first; `None`: the target tab's database.
    #[serde(default)]
    pub new_database: Option<String>,
    pub objects: Vec<CopyObject>,
    pub mode: CopyMode,
    pub content: CopyContent,
    /// Stop at the first failing table (default); otherwise skip it and go on.
    pub stop_on_error: bool,
    /// Same engine: also copy the functions and procedures of the chosen tables' schemas (or database) and the
    /// triggers of the chosen tables.
    #[serde(default)]
    pub routines: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyTablePlan {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub exists: bool,
    /// What happens, in words: "create", "drop and create", "empty and load", "add rows", "insert or replace".
    pub action: String,
    pub rows_estimate: Option<f64>,
    /// Between PostgreSQL and MySQL/MariaDB: each column's mapping, and the `create table` that will run.
    pub columns: Vec<CopyColumn>,
    pub ddl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyPlan {
    pub pending_id: Option<String>,
    pub blocked: Option<String>,
    pub level: String,
    pub typed_name: String,
    pub source_name: String,
    pub target_name: String,
    pub environment: String,
    pub target_db: String,
    pub new_database: bool,
    pub tables: Vec<CopyTablePlan>,
    pub warnings: Vec<String>,
    /// The source is read in one snapshot (a MongoDB copy isn't).
    pub snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyProgress {
    pub table: String,
    pub index: f64,
    pub tables: f64,
    pub rows: f64,
    pub estimate: Option<f64>,
    /// `schema`, `rows`, `keys` (keys, indexes, sequences, views after the data).
    pub phase: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyTableReport {
    pub source: String,
    pub target: String,
    /// `done`, `failed`, `stopped` or `skipped`.
    pub status: String,
    pub rows: f64,
    /// The row check: the source's count in the snapshot and the target's after the copy.
    pub source_rows: Option<f64>,
    pub target_rows: Option<f64>,
    pub error: Option<String>,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CopyReport {
    /// `done`, `stopped`, `failed` (a table failed and the copy stopped) or `partial` (some tables failed).
    pub outcome: String,
    pub target_db: String,
    pub tables: Vec<CopyTableReport>,
    pub warnings: Vec<String>,
    pub seconds: f64,
    /// Views, functions and triggers a copy between engines leaves out, with their DDL.
    pub not_copied: Vec<NotCopied>,
}

/// Engines that can copy to each other in this version.
/// Engines that can copy to each other: the same family, or PostgreSQL ↔ MySQL/MariaDB.
pub fn compatible(a: Engine, b: Engine) -> bool {
    let sql = |e: &str| e == "postgres" || e == "mysql";
    family(a) == family(b) || (sql(family(a)) && sql(family(b)))
}

/// Pages of a MySQL table by its primary key inside the source's snapshot (a keyless table in one page).
pub struct MyPager {
    rel: String,
    list: String,
    names: Vec<String>,
    pk: Vec<String>,
    filter: Option<String>,
    limit: Option<u64>,
    after: Option<Vec<Option<String>>>,
    done: u64,
    finished: bool,
}

impl MyPager {
    pub fn new(
        db: &str,
        name: &str,
        names: Vec<String>,
        pk: Vec<String>,
        filter: Option<String>,
        limit: Option<u64>,
    ) -> MyPager {
        MyPager {
            rel: format!("{}.{}", q(db), q(name)),
            list: names.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "),
            names,
            pk,
            filter,
            limit,
            after: None,
            done: 0,
            finished: false,
        }
    }

    pub async fn next(
        &mut self,
        s: &crate::sql::mysql::MyConn,
    ) -> AppResult<Option<Vec<Vec<Option<String>>>>> {
        const PAGE: usize = 1000;
        const NO_KEY_CAP: usize = 200_000;
        if self.finished {
            return Ok(None);
        }
        let mut cond: Vec<String> = self.filter.iter().map(|f| format!("({f})")).collect();
        if let (false, Some(a)) = (self.pk.is_empty(), &self.after) {
            let keys = self.pk.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
            let vals = a
                .iter()
                .map(|v| v.as_ref().map_or("NULL".into(), |x| lit(x)))
                .collect::<Vec<_>>()
                .join(", ");
            cond.push(format!("({keys}) > ({vals})"));
        }
        let page = if self.pk.is_empty() { NO_KEY_CAP } else { PAGE };
        let take = self.limit.map_or(page as u64, |l| {
            l.saturating_sub(self.done).min(page as u64)
        });
        if take == 0 {
            self.finished = true;
            return Ok(None);
        }
        let order = if self.pk.is_empty() {
            String::new()
        } else {
            format!(
                " order by {}",
                self.pk.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let wh = if cond.is_empty() {
            String::new()
        } else {
            format!(" where {}", cond.join(" and "))
        };
        let res = s
            .query(
                &format!(
                    "select {} from {}{wh}{order} limit {take}",
                    self.list, self.rel
                ),
                page + 1,
            )
            .await?;
        if res.rows.is_empty() {
            self.finished = true;
            return Ok(None);
        }
        let got = res.rows.len();
        if self.pk.is_empty() && got >= NO_KEY_CAP {
            return Err(AppError::new(ErrorCode::SqlNoKey, format!("{} has no primary key and more than {NO_KEY_CAP} rows: Dalan copies a keyless table of up to that many.", self.rel)));
        }
        let at: Vec<usize> = self
            .pk
            .iter()
            .filter_map(|k| self.names.iter().position(|c| c == k))
            .collect();
        self.after = res
            .rows
            .last()
            .map(|r| at.iter().map(|j| r[*j].clone()).collect());
        self.done += got as u64;
        self.finished = self.pk.is_empty() || got < take as usize;
        Ok(Some(res.rows))
    }
}

pub fn family(e: Engine) -> &'static str {
    match e {
        Engine::Postgres => "postgres",
        Engine::Mysql | Engine::Mariadb => "mysql",
        Engine::Mongodb => "mongodb",
        Engine::Redis => "redis",
    }
}

pub fn action(mode: CopyMode, exists: bool) -> &'static str {
    match (mode, exists) {
        (CopyMode::Create, _) => "create",
        (CopyMode::Recreate, true) => "drop and create",
        (CopyMode::Recreate, false) => "create",
        (CopyMode::Empty, _) => "empty and load",
        (CopyMode::Append, _) => "add rows",
        (CopyMode::Upsert, _) => "insert or replace",
    }
}

pub fn target_name(o: &CopyObject) -> String {
    o.rename
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| o.name.clone())
}

fn shown(o: &CopyObject, engine: Engine) -> String {
    if engine == Engine::Mongodb || o.schema.is_empty() {
        o.name.clone()
    } else {
        format!("{}.{}", o.schema, o.name)
    }
}

/// What the target already has, as `schema.name` (PostgreSQL), `name` (MySQL, MongoDB).
pub async fn target_has(conn: &RawConn, db: &str) -> AppResult<Vec<String>> {
    Ok(match conn {
        RawConn::Pg(c) => c
            .client
            .query(
                "select n.nspname::text || '.' || c.relname::text from pg_class c join pg_namespace n on n.oid = c.relnamespace
                 where c.relkind in ('r','p','v','m','f') and n.nspname not in ('pg_catalog','information_schema')",
                &[],
            )
            .await
            .map_err(|e| map_error(&e))?
            .iter()
            .map(|r| r.get(0))
            .collect(),
        RawConn::My(c) => c
            .query(&format!("select table_name from information_schema.tables where table_schema = {}", lit(db)), 0)
            .await?
            .rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect(),
        RawConn::Mongo(c) => {
            let r = c.command(db, doc! { "listCollections": 1, "nameOnly": true }).await?;
            crate::mongo::docs::names(&r, "name").iter().filter_map(|d| d.get_str("name").ok().map(str::to_string)).collect()
        }
    })
}

/// The source's estimate of each object's rows (for progress and the review).
pub async fn estimates(conn: &RawConn, objs: &[CopyObject]) -> Vec<Option<f64>> {
    let mut out = Vec::new();
    for o in objs {
        let n = match conn {
            RawConn::Pg(c) => c
                .client
                .query_opt("select reltuples::float8 from pg_class where oid = $1::text::regclass", &[&format!("{}.{}", quote(&o.schema), quote(&o.name))])
                .await
                .ok()
                .flatten()
                .map(|r| r.get::<_, f64>(0))
                .filter(|n| *n >= 0.0),
            RawConn::My(c) => c
                .query(&format!("select table_rows from information_schema.tables where table_schema = {} and table_name = {}", lit(&o.schema), lit(&o.name)), 0)
                .await
                .ok()
                .and_then(|r| r.rows.into_iter().next())
                .and_then(|r| r.into_iter().next().flatten())
                .and_then(|v| v.parse().ok()),
            RawConn::Mongo(c) => c
                .command(&o.schema, doc! { "count": &o.name })
                .await
                .ok()
                .and_then(|r| r.get("n").and_then(Bson::as_i32).map(f64::from).or_else(|| r.get("n").and_then(Bson::as_i64).map(|x| x as f64))),
        };
        out.push(match (n, o.limit) {
            (Some(n), Some(l)) => Some(n.min(l)),
            (n, l) => n.or(l),
        });
    }
    out
}

/// The tables `objects` reference by foreign key, followed to the end (those already chosen are left out).
pub async fn related(conn: &RawConn, objects: &[CopyObject]) -> AppResult<Vec<CopyObject>> {
    let mut seen: Vec<(String, String)> = objects
        .iter()
        .map(|o| (o.schema.clone(), o.name.clone()))
        .collect();
    let mut todo = seen.clone();
    let mut out = Vec::new();
    while let Some((schema, name)) = todo.pop() {
        let refs: Vec<(String, String)> = match conn {
            RawConn::Pg(c) => c
                .client
                .query(
                    "select n.nspname::text, r.relname::text from pg_constraint c join pg_class r on r.oid = c.confrelid join pg_namespace n on n.oid = r.relnamespace
                     where c.contype = 'f' and c.conrelid = $1::text::regclass",
                    &[&format!("{}.{}", quote(&schema), quote(&name))],
                )
                .await
                .map_err(|e| map_error(&e))?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect(),
            RawConn::My(c) => c
                .query(&format!("select referenced_table_schema, referenced_table_name from information_schema.key_column_usage where table_schema = {} and table_name = {} and referenced_table_name is not null", lit(&schema), lit(&name)), 0)
                .await?
                .rows
                .into_iter()
                .filter_map(|r| Some((r.first().cloned().flatten()?, r.get(1).cloned().flatten()?)))
                .collect(),
            RawConn::Mongo(_) => vec![],
        };
        for r in refs {
            if !seen.contains(&r) {
                seen.push(r.clone());
                todo.push(r.clone());
                out.push(CopyObject {
                    schema: r.0,
                    name: r.1,
                    kind: "table".into(),
                    filter: None,
                    limit: None,
                    rename: None,
                    types: None,
                    ddl: None,
                });
            }
        }
    }
    Ok(out)
}

/// Everything a run needs (held by the gate between the review and the confirmation).
pub struct CopyJob {
    pub req: CopyRequest,
    pub source_engine: Engine,
    pub target_db: String,
    pub new_database: bool,
    pub typed_name: String,
    pub plans: Vec<CopyTablePlan>,
}

pub struct Run<'a> {
    pub job: &'a CopyJob,
    pub src: RawConn,
    pub dst: RawConn,
    pub stop: &'a AtomicBool,
    pub progress: &'a (dyn Fn(CopyProgress) + Sync),
}

pub async fn run(r: Run<'_>) -> AppResult<CopyReport> {
    let started = Instant::now();
    let mut rep = CopyReport {
        outcome: "done".into(),
        target_db: r.job.target_db.clone(),
        tables: Vec::new(),
        warnings: Vec::new(),
        seconds: 0.0,
        not_copied: Vec::new(),
    };
    let cross = matches!(
        (&r.src, &r.dst),
        (RawConn::Pg(_), RawConn::My(_)) | (RawConn::My(_), RawConn::Pg(_))
    );
    if cross {
        super::copycross::run(r.job, &r.src, &r.dst, r.stop, r.progress, &mut rep).await?;
    } else {
        match (r.src, r.dst) {
            (RawConn::Pg(s), RawConn::Pg(d)) => {
                pg::run(r.job, &s, &d, r.stop, r.progress, &mut rep).await?
            }
            (RawConn::My(s), RawConn::My(d)) => {
                my::run(r.job, &s, &d, r.stop, r.progress, &mut rep).await?
            }
            (RawConn::Mongo(s), RawConn::Mongo(d)) => {
                mongo::run(r.job, &s, &d, r.stop, r.progress, &mut rep).await?
            }
            _ => {
                return Err(AppError::validation(
                    "MongoDB copies go to MongoDB, and SQL copies to SQL.",
                ))
            }
        }
    }
    let failed = rep.tables.iter().filter(|t| t.status == "failed").count();
    rep.outcome = if rep.tables.iter().any(|t| t.status == "stopped") {
        "stopped"
    } else if failed > 0 && r.job.req.stop_on_error {
        "failed"
    } else if failed > 0 {
        "partial"
    } else {
        "done"
    }
    .into();
    rep.seconds = started.elapsed().as_secs_f64();
    Ok(rep)
}

fn report(o: &CopyObject, engine: Engine) -> CopyTableReport {
    CopyTableReport {
        source: shown(o, engine),
        target: target_name(o),
        status: "skipped".into(),
        rows: 0.0,
        source_rows: None,
        target_rows: None,
        error: None,
        seconds: 0.0,
    }
}

fn stopped() -> AppError {
    AppError::new(ErrorCode::Internal, "Stopped.")
}

// ---------------------------------------------------------------- PostgreSQL

mod pg {
    use super::*;

    /// A column's sequence, from a `nextval('public.t_id_seq'::regclass)` default.
    fn serial_seq(default: &str) -> Option<String> {
        let d = default.strip_prefix("nextval('")?;
        Some(d.split('\'').next()?.to_string())
    }

    /// name, type, nullable, default, generated
    type Col = (String, String, bool, Option<String>, Option<String>);

    struct Table {
        cols: Vec<Col>,
        checks: Vec<(String, String)>,
        keys: Vec<(String, String)>,
        fks: Vec<(String, String)>,
        indexes: Vec<String>,
        pk: Vec<String>,
        enums: Vec<(String, Vec<String>)>,
    }

    async fn describe(s: &PgConn, schema: &str, name: &str) -> AppResult<Table> {
        let st = s.structure(schema, name).await?;
        let rel = format!("{}.{}", quote(schema), quote(name));
        let enums = s
            .client
            .query(
                "select format('%I.%I', n.nspname, t.typname), array_agg(e.enumlabel::text order by e.enumsortorder)
                 from pg_attribute a join pg_type t on t.oid = a.atttypid join pg_namespace n on n.oid = t.typnamespace
                 join pg_enum e on e.enumtypid = t.oid where a.attrelid = $1::text::regclass and a.attnum > 0 group by 1",
                &[&rel],
            )
            .await
            .map_err(|e| map_error(&e))?
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, Vec<String>>(1)))
            .collect();
        let key_names: Vec<String> = st.constraints.iter().map(|c| c.name.clone()).collect();
        Ok(Table {
            cols: st
                .columns
                .iter()
                .map(|c| {
                    (
                        c.name.clone(),
                        c.type_name.clone(),
                        c.nullable,
                        c.default.clone(),
                        c.generated.clone(),
                    )
                })
                .collect(),
            checks: st
                .constraints
                .iter()
                .filter(|c| c.kind == "c")
                .map(|c| (c.name.clone(), c.definition.clone()))
                .collect(),
            keys: st
                .constraints
                .iter()
                .filter(|c| matches!(c.kind.as_str(), "p" | "u" | "x"))
                .map(|c| (c.name.clone(), c.definition.clone()))
                .collect(),
            fks: st
                .constraints
                .iter()
                .filter(|c| c.kind == "f")
                .map(|c| (c.name.clone(), c.definition.clone()))
                .collect(),
            indexes: st
                .indexes
                .iter()
                .filter(|i| !key_names.contains(&i.name))
                .map(|i| i.definition.clone())
                .collect(),
            pk: st
                .constraints
                .iter()
                .find(|c| c.kind == "p")
                .map(|c| c.columns.clone())
                .unwrap_or_default(),
            enums,
        })
    }

    /// `create table` without keys (they come after the data). Identity and stored generated columns keep their kind.
    fn create_sql(t: &Table, target: &str) -> String {
        let mut lines: Vec<String> = t
            .cols
            .iter()
            .map(|(n, ty, nullable, default, generated)| {
                let mut l = format!("  {} {ty}", quote(n));
                match (generated, default) {
                    (Some(g), _) => l.push_str(&format!(" {g}")),
                    (None, Some(d)) => l.push_str(&format!(" default {d}")),
                    _ => {}
                }
                if !nullable {
                    l.push_str(" not null");
                }
                l
            })
            .collect();
        lines.extend(
            t.checks
                .iter()
                .map(|(n, d)| format!("  constraint {} {d}", quote(n))),
        );
        format!("create table {target} (\n{}\n)", lines.join(",\n"))
    }

    /// Column list for COPY: everything but stored generated columns (the target computes them).
    fn copy_cols(t: &Table) -> Vec<String> {
        t.cols
            .iter()
            .filter(|c| !c.4.as_deref().is_some_and(|g| g.contains("stored")))
            .map(|c| quote(&c.0))
            .collect()
    }

    pub async fn run(
        job: &CopyJob,
        s: &PgConn,
        d: &PgConn,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
        rep: &mut CopyReport,
    ) -> AppResult<()> {
        let req = &job.req;
        // One snapshot for every table: what is copied is one point in time.
        s.client
            .batch_execute("begin isolation level repeatable read read only")
            .await
            .map_err(|e| map_error(&e))?;
        // Functions and procedures first (defaults and checks may call them), when asked for.
        if req.routines && req.content != CopyContent::Data {
            let schemas: Vec<String> = req
                .objects
                .iter()
                .map(|o| o.schema.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let defs = s
                .client
                .query(
                    "select n.nspname::text, pg_get_functiondef(p.oid) from pg_proc p join pg_namespace n on n.oid = p.pronamespace
                     where n.nspname = any($1) and p.prokind in ('f', 'p')
                       and not exists (select 1 from pg_depend d where d.objid = p.oid and d.deptype = 'e') order by p.oid",
                    &[&schemas],
                )
                .await
                .map_err(|e| map_error(&e))?;
            for r in &defs {
                let (schema, def): (String, String) = (r.get(0), r.get(1));
                let res = async {
                    // Bodies may name tables that come later (as pg_dump does).
                    d.client
                        .batch_execute(&format!(
                            "set check_function_bodies = false; create schema if not exists {}",
                            quote(&schema)
                        ))
                        .await?;
                    d.client.batch_execute(&def).await
                }
                .await;
                if let Err(e) = res {
                    rep.warnings.push(format!(
                        "A function in {schema} wasn't copied: {}",
                        map_error(&e).message
                    ));
                }
            }
        }
        let n = req.objects.len() as f64;
        let mut described: Vec<Option<(Table, String)>> = Vec::new();
        for (i, o) in req.objects.iter().enumerate() {
            let mut tr = report(o, Engine::Postgres);
            let target = format!("{}.{}", quote(&o.schema), quote(&target_name(o)));
            if o.kind != "table" {
                described.push(None);
                rep.tables.push(tr);
                continue;
            }
            if stop.load(Ordering::SeqCst) {
                tr.status = "stopped".into();
                rep.tables.push(tr);
                described.push(None);
                continue;
            }
            let t0 = Instant::now();
            let est = job.plans.get(i).and_then(|p| p.rows_estimate);
            progress(CopyProgress {
                table: tr.source.clone(),
                index: i as f64,
                tables: n,
                rows: 0.0,
                estimate: est,
                phase: "rows".into(),
            });
            let res = one_table(job, s, d, o, &target, i, n, est, stop, progress).await;
            tr.seconds = t0.elapsed().as_secs_f64();
            match res {
                Ok((t, rows, src_rows)) => {
                    tr.status = "done".into();
                    tr.rows = rows as f64;
                    tr.source_rows = Some(src_rows as f64);
                    tr.target_rows = d
                        .client
                        .query_one(&format!("select count(*) from {target}"), &[])
                        .await
                        .ok()
                        .map(|r| r.get::<_, i64>(0) as f64);
                    described.push(Some((t, target)));
                }
                Err(e) => {
                    let _ = d.client.batch_execute("rollback").await;
                    tr.status = if e.message == "Stopped." {
                        "stopped"
                    } else {
                        "failed"
                    }
                    .into();
                    tr.error = (tr.status == "failed").then(|| e.message.clone());
                    described.push(None);
                    let halt = tr.status == "stopped" || req.stop_on_error;
                    rep.tables.push(tr);
                    if halt {
                        for o in &req.objects[i + 1..] {
                            let mut x = report(o, Engine::Postgres);
                            x.status = "skipped".into();
                            rep.tables.push(x);
                        }
                        break;
                    }
                    continue;
                }
            }
            rep.tables.push(tr);
        }
        // Keys, indexes, foreign keys and sequences once the rows are in (faster, and no load order to get right).
        if req.content != CopyContent::Data
            && matches!(req.mode, CopyMode::Create | CopyMode::Recreate)
        {
            progress(CopyProgress {
                table: String::new(),
                index: n,
                tables: n,
                rows: 0.0,
                estimate: None,
                phase: "keys".into(),
            });
            for (t, target) in described.iter().flatten() {
                for (name, def) in &t.keys {
                    if let Err(e) = d
                        .client
                        .batch_execute(&format!(
                            "alter table {target} add constraint {} {def}",
                            quote(name)
                        ))
                        .await
                    {
                        rep.warnings.push(format!(
                            "{target}: key {name} wasn't added: {}",
                            map_error(&e).message
                        ));
                    }
                }
                for def in &t.indexes {
                    if let Err(e) = d.client.batch_execute(def).await {
                        rep.warnings.push(format!(
                            "{target}: index wasn't made: {}",
                            map_error(&e).message
                        ));
                    }
                }
            }
            for (t, target) in described.iter().flatten() {
                for (name, def) in &t.fks {
                    if let Err(e) = d
                        .client
                        .batch_execute(&format!(
                            "alter table {target} add constraint {} {def}",
                            quote(name)
                        ))
                        .await
                    {
                        rep.warnings.push(format!(
                            "{target}: foreign key {name} wasn't added: {}",
                            map_error(&e).message
                        ));
                    }
                }
            }
        }
        // Triggers of the copied tables, once their rows are in (so they don't fire on the copy).
        if req.routines
            && req.content != CopyContent::Data
            && matches!(req.mode, CopyMode::Create | CopyMode::Recreate)
        {
            for (o, (_, target)) in req
                .objects
                .iter()
                .zip(described.iter())
                .filter_map(|(o, x)| x.as_ref().map(|x| (o, x)))
            {
                let rel = format!("{}.{}", quote(&o.schema), quote(&o.name));
                let trig = s.client.query("select pg_get_triggerdef(oid) from pg_trigger where tgrelid = $1::text::regclass and not tgisinternal", &[&rel]).await.map_err(|e| map_error(&e))?;
                for r in &trig {
                    let def: String = r.get(0);
                    let def = if target_name(o) == o.name {
                        def
                    } else {
                        def.replace(&format!(" ON {rel} "), &format!(" ON {target} "))
                    };
                    if let Err(e) = d.client.batch_execute(&def).await {
                        rep.warnings.push(format!(
                            "{target}: a trigger wasn't copied: {}",
                            map_error(&e).message
                        ));
                    }
                }
            }
        }
        if req.content != CopyContent::Schema {
            // Sequences past the highest copied value.
            for (t, target) in described.iter().flatten() {
                for (col, _, _, default, generated) in &t.cols {
                    let seq = match (
                        generated.as_deref(),
                        default.as_deref().and_then(serial_seq),
                    ) {
                        (Some(g), _) if g.contains("identity") => format!(
                            "pg_get_serial_sequence({}, {})",
                            lit_pg(target),
                            lit_pg(col)
                        ),
                        (_, Some(sq)) => lit_pg(&sq),
                        _ => continue,
                    };
                    let sql = format!("select setval({seq}, coalesce(max({c}), 1), max({c}) is not null) from {target}", c = quote(col));
                    if let Err(e) = d.client.batch_execute(&sql).await {
                        rep.warnings.push(format!(
                            "{target}: the sequence of {col} wasn't moved on: {}",
                            map_error(&e).message
                        ));
                    }
                }
            }
        }
        // Views after the tables they read.
        if req.content != CopyContent::Data {
            for (i, o) in req
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| o.kind == "view" || o.kind == "matview")
            {
                let target = format!("{}.{}", quote(&o.schema), quote(&target_name(o)));
                let def: Option<String> = s
                    .client
                    .query_opt(
                        "select pg_get_viewdef($1::text::regclass, true)",
                        &[&format!("{}.{}", quote(&o.schema), quote(&o.name))],
                    )
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.get(0));
                let res = match def {
                    Some(def) => {
                        let drop = if req.mode == CopyMode::Recreate {
                            format!(
                                "drop {} if exists {target} cascade; ",
                                if o.kind == "matview" {
                                    "materialized view"
                                } else {
                                    "view"
                                }
                            )
                        } else {
                            String::new()
                        };
                        let create = if o.kind == "matview" {
                            format!("create materialized view {target} as {def}")
                        } else {
                            format!("create view {target} as {def}")
                        };
                        d.client
                            .batch_execute(&format!("{drop}{}", create.trim_end_matches(';')))
                            .await
                            .map_err(|e| map_error(&e).message)
                    }
                    None => Err("its definition couldn't be read".into()),
                };
                if let Some(tr) = rep.tables.get_mut(i) {
                    match res {
                        Ok(()) => tr.status = "done".into(),
                        Err(e) => {
                            tr.status = "failed".into();
                            tr.error = Some(e);
                        }
                    }
                }
            }
        }
        let _ = s.client.batch_execute("rollback").await;
        Ok(())
    }

    fn lit_pg(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    /// One table in one target transaction: its schema (by the mode), then its rows streamed COPY to COPY.
    #[allow(clippy::too_many_arguments)]
    async fn one_table(
        job: &CopyJob,
        s: &PgConn,
        d: &PgConn,
        o: &CopyObject,
        target: &str,
        i: usize,
        n: f64,
        est: Option<f64>,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
    ) -> AppResult<(Table, u64, u64)> {
        let req = &job.req;
        let t = describe(s, &o.schema, &o.name).await?;
        let src = format!("{}.{}", quote(&o.schema), quote(&o.name));
        let filter = o.filter.as_deref().map(str::trim).filter(|f| !f.is_empty());
        if let Some(f) = filter {
            super::super::pg::check_filter(f)?;
        }
        let tail = format!(
            "{}{}",
            filter.map(|f| format!(" where ({f})")).unwrap_or_default(),
            o.limit
                .filter(|l| *l >= 0.0)
                .map(|l| format!(" limit {}", l as u64))
                .unwrap_or_default()
        );
        d.client
            .batch_execute("begin")
            .await
            .map_err(|e| map_error(&e))?;
        if req.content != CopyContent::Data {
            d.client
                .batch_execute(&format!("create schema if not exists {}", quote(&o.schema)))
                .await
                .map_err(|e| map_error(&e))?;
            for (name, labels) in &t.enums {
                let exists = !d
                    .client
                    .query("select 1 from pg_type where oid = to_regtype($1)", &[name])
                    .await
                    .map_err(|e| map_error(&e))?
                    .is_empty();
                if !exists {
                    let labels: Vec<String> = labels.iter().map(|l| lit_pg(l)).collect();
                    d.client
                        .batch_execute(&format!(
                            "create type {name} as enum ({})",
                            labels.join(", ")
                        ))
                        .await
                        .map_err(|e| map_error(&e))?;
                }
            }
            for (_, _, _, default, _) in &t.cols {
                if let Some(seq) = default.as_deref().and_then(serial_seq) {
                    d.client
                        .batch_execute(&format!("create sequence if not exists {seq}"))
                        .await
                        .map_err(|e| map_error(&e))?;
                }
            }
            match req.mode {
                CopyMode::Recreate => {
                    d.client
                        .batch_execute(&format!("drop table if exists {target} cascade"))
                        .await
                        .map_err(|e| map_error(&e))?;
                    d.client
                        .batch_execute(&create_sql(&t, target))
                        .await
                        .map_err(|e| map_error(&e))?;
                }
                CopyMode::Create => d
                    .client
                    .batch_execute(&create_sql(&t, target))
                    .await
                    .map_err(|e| map_error(&e))?,
                _ => {}
            }
        }
        if req.mode == CopyMode::Empty {
            d.client
                .batch_execute(&format!("truncate {target}"))
                .await
                .map_err(|e| map_error(&e))?;
        }
        let mut rows = 0u64;
        let cols = copy_cols(&t).join(", ");
        if req.content != CopyContent::Schema {
            // Upsert: through a staging table, then one insert … on conflict.
            let into = if req.mode == CopyMode::Upsert {
                if t.pk.is_empty() {
                    return Err(AppError::new(ErrorCode::SqlNoKey, format!("{src} has no primary key, so its rows can't be matched for “insert or replace”.")));
                }
                d.client.batch_execute(&format!("create temp table _dalan_stage (like {target} including defaults) on commit drop")).await.map_err(|e| map_error(&e))?;
                "_dalan_stage".to_string()
            } else {
                target.to_string()
            };
            let out = s
                .client
                .copy_out(&format!("copy (select {cols} from {src}{tail}) to stdout"))
                .await
                .map_err(|e| map_error(&e))?;
            let sink = d
                .client
                .copy_in::<_, bytes::Bytes>(&format!("copy {into} ({cols}) from stdin"))
                .await
                .map_err(|e| map_error(&e))?;
            futures_util::pin_mut!(out);
            futures_util::pin_mut!(sink);
            let mut last = Instant::now();
            while let Some(chunk) = out.next().await {
                let chunk = chunk.map_err(|e| map_error(&e))?;
                if stop.load(Ordering::SeqCst) {
                    // The sink is dropped unfinished: the COPY fails and the transaction is rolled back.
                    return Err(stopped());
                }
                rows += chunk.iter().filter(|b| **b == b'\n').count() as u64;
                sink.send(chunk).await.map_err(|e| map_error(&e))?;
                if last.elapsed().as_millis() > 200 {
                    last = Instant::now();
                    progress(CopyProgress {
                        table: format!("{}.{}", o.schema, o.name),
                        index: i as f64,
                        tables: n,
                        rows: rows as f64,
                        estimate: est,
                        phase: "rows".into(),
                    });
                }
            }
            sink.as_mut().finish().await.map_err(|e| map_error(&e))?;
            if req.mode == CopyMode::Upsert {
                let pk: Vec<String> = t.pk.iter().map(|c| quote(c)).collect();
                let set: Vec<String> = copy_cols(&t)
                    .iter()
                    .filter(|c| !pk.contains(c))
                    .map(|c| format!("{c} = excluded.{c}"))
                    .collect();
                let action = if set.is_empty() {
                    "do nothing".to_string()
                } else {
                    format!("do update set {}", set.join(", "))
                };
                d.client.batch_execute(&format!("insert into {target} ({cols}) select {cols} from _dalan_stage on conflict ({}) {action}", pk.join(", "))).await.map_err(|e| map_error(&e))?;
            }
        }
        d.client
            .batch_execute("commit")
            .await
            .map_err(|e| map_error(&e))?;
        let src_rows: i64 = s
            .client
            .query_one(
                &format!("select count(*) from (select 1 from {src}{tail}) x"),
                &[],
            )
            .await
            .map_err(|e| map_error(&e))?
            .get(0);
        Ok((t, rows, src_rows as u64))
    }
}

// ---------------------------------------------------------------- MySQL and MariaDB

mod my {
    use super::*;
    use crate::sql::mysql::MyConn;

    /// Rows read per page from the source (by primary key, inside the snapshot).
    const PAGE: usize = 1000;
    /// A batched INSERT is sent at this size.
    const BATCH_BYTES: usize = 1 << 20;
    /// A table without a primary key is read whole; past this many rows it is refused.
    const NO_KEY_CAP: usize = 200_000;

    fn value(v: &Option<String>, bytes: bool) -> String {
        match v {
            None => "NULL".into(),
            Some(s) if bytes => format!("X'{}'", s.trim_start_matches("\\x")),
            Some(s) => lit(s),
        }
    }

    pub async fn run(
        job: &CopyJob,
        s: &MyConn,
        d: &MyConn,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
        rep: &mut CopyReport,
    ) -> AppResult<()> {
        let req = &job.req;
        s.query("set session transaction isolation level repeatable read", 0)
            .await?;
        s.query("start transaction with consistent snapshot, read only", 0)
            .await?;
        // Tables are created in any order: foreign keys are checked only once everything is in.
        d.query("set foreign_key_checks = 0", 0).await?;
        let n = req.objects.len() as f64;
        for (i, o) in req.objects.iter().enumerate() {
            let mut tr = report(o, Engine::Mysql);
            if stop.load(Ordering::SeqCst) {
                tr.status = "stopped".into();
                rep.tables.push(tr);
                continue;
            }
            let t0 = Instant::now();
            let est = job.plans.get(i).and_then(|p| p.rows_estimate);
            let target = format!("{}.{}", q(&job.target_db), q(&target_name(o)));
            let res = if o.kind == "view" {
                view(job, s, d, o, &target).await.map(|_| (0, 0))
            } else {
                table(job, s, d, o, &target, i, n, est, stop, progress).await
            };
            tr.seconds = t0.elapsed().as_secs_f64();
            match res {
                Ok((rows, src_rows)) => {
                    tr.status = "done".into();
                    tr.rows = rows as f64;
                    if o.kind != "view" {
                        tr.source_rows = Some(src_rows as f64);
                        tr.target_rows = d
                            .query(&format!("select count(*) from {target}"), 0)
                            .await
                            .ok()
                            .and_then(|r| r.rows.into_iter().next())
                            .and_then(|r| r.into_iter().next().flatten())
                            .and_then(|v| v.parse().ok());
                    }
                }
                Err(e) => {
                    let _ = d.query("rollback", 0).await;
                    tr.status = if e.message == "Stopped." {
                        "stopped"
                    } else {
                        "failed"
                    }
                    .into();
                    tr.error = (tr.status == "failed").then(|| e.message.clone());
                    let halt = tr.status == "stopped" || req.stop_on_error;
                    rep.tables.push(tr);
                    if halt {
                        rep.tables.extend(
                            req.objects[i + 1..]
                                .iter()
                                .map(|o| report(o, Engine::Mysql)),
                        );
                        break;
                    }
                    continue;
                }
            }
            rep.tables.push(tr);
        }
        // Routines of the source databases, and triggers of the copied tables, when asked for (no DEFINER).
        if req.routines && req.content != CopyContent::Data {
            let dbs: std::collections::BTreeSet<&str> =
                req.objects.iter().map(|o| o.schema.as_str()).collect();
            for db in dbs {
                let rows = s.query(&format!("select routine_name, routine_type from information_schema.routines where routine_schema = {}", lit(db)), 0).await?.rows;
                for r in rows {
                    let (name, kind) = (
                        r.first().cloned().flatten().unwrap_or_default(),
                        r.get(1).cloned().flatten().unwrap_or_default(),
                    );
                    let def = s
                        .query(
                            &format!(
                                "show create {} {}.{}",
                                kind.to_ascii_lowercase(),
                                q(db),
                                q(&name)
                            ),
                            1,
                        )
                        .await
                        .ok()
                        .and_then(|x| x.rows.into_iter().next())
                        .and_then(|x| x.into_iter().nth(2).flatten());
                    let res = match def {
                        Some(def) => d
                            .query(
                                &String::from_utf8_lossy(&crate::dbs::dumpscan::strip_definer(
                                    def.as_bytes(),
                                )),
                                0,
                            )
                            .await
                            .map(|_| ()),
                        None => Err(AppError::validation(
                            "its definition couldn't be read (it needs the right to see it)",
                        )),
                    };
                    if let Err(e) = res {
                        rep.warnings
                            .push(format!("{kind} {name} wasn't copied: {}", e.message));
                    }
                }
            }
            if matches!(req.mode, CopyMode::Create | CopyMode::Recreate) {
                for o in req.objects.iter().filter(|o| o.kind == "table") {
                    let rows = s.query(&format!("select trigger_name from information_schema.triggers where event_object_schema = {} and event_object_table = {}", lit(&o.schema), lit(&o.name)), 0).await?.rows;
                    for r in rows {
                        let name = r.first().cloned().flatten().unwrap_or_default();
                        let def = s
                            .query(
                                &format!("show create trigger {}.{}", q(&o.schema), q(&name)),
                                1,
                            )
                            .await
                            .ok()
                            .and_then(|x| x.rows.into_iter().next())
                            .and_then(|x| x.into_iter().nth(2).flatten());
                        if let Some(def) = def {
                            if let Err(e) = d
                                .query(
                                    &String::from_utf8_lossy(&crate::dbs::dumpscan::strip_definer(
                                        def.as_bytes(),
                                    )),
                                    0,
                                )
                                .await
                            {
                                rep.warnings
                                    .push(format!("trigger {name} wasn't copied: {}", e.message));
                            }
                        }
                    }
                }
            }
        }
        let _ = d.query("set foreign_key_checks = 1", 0).await;
        let _ = s.query("rollback", 0).await;
        Ok(())
    }

    async fn first_value(c: &MyConn, sql: &str) -> AppResult<Option<String>> {
        Ok(c.query(sql, 1)
            .await?
            .rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().nth(1).flatten()))
    }

    async fn view(
        job: &CopyJob,
        s: &MyConn,
        d: &MyConn,
        o: &CopyObject,
        target: &str,
    ) -> AppResult<()> {
        if job.req.content == CopyContent::Data {
            return Ok(());
        }
        let ddl = first_value(
            s,
            &format!("show create view {}.{}", q(&o.schema), q(&o.name)),
        )
        .await?
        .ok_or_else(|| AppError::not_found("That view is gone."))?;
        // `CREATE ALGORITHM=… DEFINER=… SQL SECURITY … VIEW `v` AS …`: the restoring user owns it, under its new name.
        let body = ddl
            .split_once(" AS ")
            .map(|(_, b)| b.to_string())
            .ok_or_else(|| AppError::internal("SHOW CREATE VIEW gave no AS"))?;
        // The view names its tables with the source database: they are the target's now.
        let body = body.replace(
            &format!("{}.", q(&o.schema)),
            &format!("{}.", q(&job.target_db)),
        );
        if job.req.mode == CopyMode::Recreate {
            d.query(&format!("drop view if exists {target}"), 0).await?;
        }
        d.query(&format!("create view {target} as {body}"), 0)
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn table(
        job: &CopyJob,
        s: &MyConn,
        d: &MyConn,
        o: &CopyObject,
        target: &str,
        i: usize,
        n: f64,
        est: Option<f64>,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
    ) -> AppResult<(u64, u64)> {
        let req = &job.req;
        let src = format!("{}.{}", q(&o.schema), q(&o.name));
        let filter = o.filter.as_deref().map(str::trim).filter(|f| !f.is_empty());
        if let Some(f) = filter {
            super::super::mydb::check_filter(f)?;
        }
        if req.content != CopyContent::Data {
            let ddl = first_value(s, &format!("show create table {src}"))
                .await?
                .ok_or_else(|| AppError::not_found("That table is gone."))?;
            // `CREATE TABLE `name` (…`: the target's name in place of the source's.
            let body = ddl
                .split_once('(')
                .map(|(_, b)| b.to_string())
                .ok_or_else(|| AppError::internal("SHOW CREATE TABLE gave no columns"))?;
            if req.mode == CopyMode::Recreate {
                d.query(&format!("drop table if exists {target}"), 0)
                    .await?;
            }
            if matches!(req.mode, CopyMode::Create | CopyMode::Recreate) {
                d.query(&format!("create table {target} ({body}"), 0)
                    .await?;
            }
        }
        if req.mode == CopyMode::Empty {
            d.query(&format!("delete from {target}"), 0).await?;
        }
        if req.content == CopyContent::Schema {
            return Ok((0, 0));
        }
        // Generated columns are computed by the target; the key orders the pages.
        let cols = s
            .query(&format!("select column_name, extra, column_key from information_schema.columns where table_schema = {} and table_name = {} order by ordinal_position", lit(&o.schema), lit(&o.name)), 0)
            .await?
            .rows;
        let names: Vec<String> = cols
            .iter()
            .filter(|r| {
                !r.get(1)
                    .cloned()
                    .flatten()
                    .unwrap_or_default()
                    .to_ascii_uppercase()
                    .contains("GENERATED")
            })
            .filter_map(|r| r.first().cloned().flatten())
            .collect();
        let pk: Vec<String> = cols
            .iter()
            .filter(|r| r.get(2).cloned().flatten().as_deref() == Some("PRI"))
            .filter_map(|r| r.first().cloned().flatten())
            .collect();
        let list = names.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
        let limit = o.limit.filter(|l| *l >= 0.0).map(|l| l as u64);
        let src_rows: u64 = s
            .query(
                &format!(
                    "select count(*) from (select 1 from {src}{}{}) x",
                    filter.map(|f| format!(" where ({f})")).unwrap_or_default(),
                    limit.map(|l| format!(" limit {l}")).unwrap_or_default()
                ),
                0,
            )
            .await?
            .rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next().flatten())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if pk.is_empty() && src_rows as usize > NO_KEY_CAP {
            return Err(AppError::new(ErrorCode::SqlNoKey, format!("{src} has no primary key and {src_rows} rows: Dalan copies a keyless table of up to {NO_KEY_CAP} rows.")));
        }
        let upsert = if req.mode == CopyMode::Upsert {
            if pk.is_empty() {
                return Err(AppError::new(ErrorCode::SqlNoKey, format!("{src} has no primary key, so its rows can't be matched for “insert or replace”.")));
            }
            let set: Vec<String> = names
                .iter()
                .filter(|c| !pk.contains(c))
                .map(|c| format!("{0} = values({0})", q(c)))
                .collect();
            if set.is_empty() {
                format!(" on duplicate key update {0} = {0}", q(&pk[0]))
            } else {
                format!(" on duplicate key update {}", set.join(", "))
            }
        } else {
            String::new()
        };
        d.query("start transaction", 0).await?;
        let mut rows = 0u64;
        let mut after: Option<Vec<Option<String>>> = None;
        let mut last = Instant::now();
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err(stopped());
            }
            // Keyset pages inside the snapshot: memory stays one page and one batch, and the target sets the pace.
            let mut cond: Vec<String> = filter.map(|f| vec![format!("({f})")]).unwrap_or_default();
            if let (false, Some(a)) = (pk.is_empty(), &after) {
                let keys = pk.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
                let vals = a
                    .iter()
                    .map(|v| value(v, false))
                    .collect::<Vec<_>>()
                    .join(", ");
                cond.push(format!("({keys}) > ({vals})"));
            }
            let order = if pk.is_empty() {
                String::new()
            } else {
                format!(
                    " order by {}",
                    pk.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
                )
            };
            let page = if pk.is_empty() { NO_KEY_CAP } else { PAGE };
            let take = limit.map_or(page as u64, |l| (l.saturating_sub(rows)).min(page as u64));
            if take == 0 {
                break;
            }
            let sql = format!(
                "select {list} from {src}{}{order} limit {take}",
                if cond.is_empty() {
                    String::new()
                } else {
                    format!(" where {}", cond.join(" and "))
                }
            );
            let res = s.query(&sql, page + 1).await?;
            if res.rows.is_empty() {
                break;
            }
            let bytes_cols: Vec<bool> = res
                .columns
                .iter()
                .map(|c| c.kind == crate::sql::values::CellKind::Bytes)
                .collect();
            let pk_at: Vec<usize> = pk
                .iter()
                .filter_map(|k| names.iter().position(|c| c == k))
                .collect();
            let got = res.rows.len();
            let mut batch = String::new();
            for row in &res.rows {
                let vals = row
                    .iter()
                    .enumerate()
                    .map(|(j, v)| value(v, bytes_cols.get(j).copied().unwrap_or(false)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if batch.is_empty() {
                    batch = format!("insert into {target} ({list}) values ({vals})");
                } else {
                    batch.push_str(&format!(", ({vals})"));
                }
                if batch.len() > BATCH_BYTES {
                    d.query(&format!("{batch}{upsert}"), 0).await?;
                    batch.clear();
                }
            }
            if !batch.is_empty() {
                d.query(&format!("{batch}{upsert}"), 0).await?;
            }
            rows += got as u64;
            after = res
                .rows
                .last()
                .map(|r| pk_at.iter().map(|j| r[*j].clone()).collect());
            if last.elapsed().as_millis() > 200 {
                last = Instant::now();
                progress(CopyProgress {
                    table: format!("{}.{}", o.schema, o.name),
                    index: i as f64,
                    tables: n,
                    rows: rows as f64,
                    estimate: est,
                    phase: "rows".into(),
                });
            }
            if pk.is_empty() || got < take as usize {
                break;
            }
        }
        d.query("commit", 0).await?;
        Ok((rows, src_rows))
    }
}

// ---------------------------------------------------------------- MongoDB

mod mongo {
    use super::*;

    const BATCH: usize = 1000;
    const BATCH_BYTES: usize = 8 << 20;
    /// Collection options carried to the target as they are.
    const OPTIONS: &[&str] = &[
        "capped",
        "size",
        "max",
        "collation",
        "validator",
        "validationLevel",
        "validationAction",
        "timeseries",
        "expireAfterSeconds",
        "clusteredIndex",
        "storageEngine",
        "indexOptionDefaults",
    ];

    fn cursor(r: &Document) -> AppResult<(Vec<Document>, i64)> {
        let cur = r
            .get_document("cursor")
            .map_err(|_| AppError::new(ErrorCode::DbRefused, "MongoDB's answer had no cursor."))?;
        let batch = if cur.contains_key("firstBatch") {
            "firstBatch"
        } else {
            "nextBatch"
        };
        let id = match cur.get("id") {
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => i64::from(*i),
            _ => 0,
        };
        Ok((crate::mongo::docs::names(cur, batch), id))
    }

    pub async fn run(
        job: &CopyJob,
        s: &MongoConn,
        d: &MongoConn,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
        rep: &mut CopyReport,
    ) -> AppResult<()> {
        let req = &job.req;
        let n = req.objects.len() as f64;
        let mut done: Vec<(usize, Vec<Document>)> = Vec::new();
        // Collections first, views after them.
        let mut order: Vec<usize> = (0..req.objects.len())
            .filter(|i| req.objects[*i].kind != "view")
            .collect();
        order.extend((0..req.objects.len()).filter(|i| req.objects[*i].kind == "view"));
        let mut reports: Vec<Option<CopyTableReport>> = vec![None; req.objects.len()];
        let mut halted = false;
        for i in order {
            let o = &req.objects[i];
            let mut tr = report(o, Engine::Mongodb);
            if halted || stop.load(Ordering::SeqCst) {
                tr.status = if halted { "skipped" } else { "stopped" }.into();
                reports[i] = Some(tr);
                continue;
            }
            let t0 = Instant::now();
            let est = job.plans.get(i).and_then(|p| p.rows_estimate);
            let res = collection(job, s, d, o, i, n, est, stop, progress).await;
            tr.seconds = t0.elapsed().as_secs_f64();
            match res {
                Ok((rows, src_rows, indexes)) => {
                    tr.status = "done".into();
                    tr.rows = rows as f64;
                    if o.kind != "view" {
                        tr.source_rows = Some(src_rows as f64);
                        tr.target_rows = d
                            .command(&job.target_db, doc! { "count": target_name(o) })
                            .await
                            .ok()
                            .and_then(|r| {
                                r.get("n").and_then(|v| {
                                    v.as_i32()
                                        .map(f64::from)
                                        .or_else(|| v.as_i64().map(|x| x as f64))
                                })
                            });
                    }
                    done.push((i, indexes));
                }
                Err(e) => {
                    tr.status = if e.message == "Stopped." {
                        "stopped"
                    } else {
                        "failed"
                    }
                    .into();
                    tr.error = (tr.status == "failed").then(|| e.message.clone());
                    halted = tr.status == "stopped" || req.stop_on_error;
                }
            }
            reports[i] = Some(tr);
        }
        // Indexes once the documents are in.
        if req.content != CopyContent::Data {
            for (i, idx) in done.iter().filter(|(_, x)| !x.is_empty()) {
                let name = target_name(&req.objects[*i]);
                if let Err(e) = d
                    .command(
                        &job.target_db,
                        doc! { "createIndexes": &name, "indexes": idx.clone() },
                    )
                    .await
                {
                    rep.warnings
                        .push(format!("{name}: indexes weren't made: {}", e.message));
                }
            }
        }
        rep.tables.extend(reports.into_iter().flatten());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn collection(
        job: &CopyJob,
        s: &MongoConn,
        d: &MongoConn,
        o: &CopyObject,
        i: usize,
        n: f64,
        est: Option<f64>,
        stop: &AtomicBool,
        progress: &(dyn Fn(CopyProgress) + Sync),
    ) -> AppResult<(u64, u64, Vec<Document>)> {
        let req = &job.req;
        let (db, to) = (&job.target_db, target_name(o));
        let info = s
            .command(
                &o.schema,
                doc! { "listCollections": 1, "filter": { "name": &o.name } },
            )
            .await?;
        let info = cursor(&info)?
            .0
            .into_iter()
            .next()
            .ok_or_else(|| AppError::not_found(format!("{} is gone.", o.name)))?;
        let options = info.get_document("options").cloned().unwrap_or_default();
        if req.content != CopyContent::Data {
            if req.mode == CopyMode::Recreate {
                let _ = d.command(db, doc! { "drop": &to }).await;
            }
            if matches!(req.mode, CopyMode::Create | CopyMode::Recreate) {
                let mut create = doc! { "create": &to };
                if o.kind == "view" {
                    create.insert(
                        "viewOn",
                        options.get("viewOn").cloned().unwrap_or(Bson::Null),
                    );
                    create.insert(
                        "pipeline",
                        options
                            .get("pipeline")
                            .cloned()
                            .unwrap_or(Bson::Array(vec![])),
                    );
                } else {
                    for k in OPTIONS {
                        if let Some(v) = options.get(*k) {
                            create.insert(*k, v.clone());
                        }
                    }
                }
                d.command(db, create).await?;
            }
        }
        if o.kind == "view" {
            return Ok((0, 0, vec![]));
        }
        let idx: Vec<Document> = if req.content != CopyContent::Data {
            let r = s
                .command(&o.schema, doc! { "listIndexes": &o.name })
                .await?;
            cursor(&r)?
                .0
                .into_iter()
                .filter(|x| x.get_str("name").ok() != Some("_id_"))
                .map(|mut x| {
                    x.remove("v");
                    x.remove("ns");
                    x
                })
                .collect()
        } else {
            vec![]
        };
        if req.mode == CopyMode::Empty {
            d.command(
                db,
                doc! { "delete": &to, "deletes": [{ "q": {}, "limit": 0 }] },
            )
            .await?;
        }
        if req.content == CopyContent::Schema {
            return Ok((0, 0, idx));
        }
        let filter = match o.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            Some(f) => crate::mongo::docs::parse_doc(f, "The filter")?,
            None => Document::new(),
        };
        let mut find =
            doc! { "find": &o.name, "filter": filter.clone(), "batchSize": BATCH as i32 };
        if let Some(l) = o.limit.filter(|l| *l >= 0.0) {
            find.insert("limit", l as i64);
        }
        let mut count_cmd = doc! { "count": &o.name, "query": filter };
        if let Some(l) = o.limit.filter(|l| *l >= 0.0) {
            count_cmd.insert("limit", l as i64);
        }
        let src_rows = s
            .command(&o.schema, count_cmd)
            .await
            .ok()
            .and_then(|r| {
                r.get("n")
                    .and_then(|v| v.as_i32().map(i64::from).or_else(|| v.as_i64()))
            })
            .unwrap_or(0) as u64;
        let mut r = s.command(&o.schema, find).await?;
        let mut rows = 0u64;
        let mut last = Instant::now();
        loop {
            let (docs, id) = cursor(&r)?;
            // Ordered batches: a failure stops at the document that failed (the report says how many went in).
            let mut batch: Vec<Document> = Vec::new();
            let mut size = 0usize;
            for doc in docs {
                size += {
                    let mut b = Vec::new();
                    doc.to_writer(&mut b).map(|_| b.len()).unwrap_or(0)
                };
                batch.push(doc);
                if batch.len() >= BATCH || size > BATCH_BYTES {
                    rows += write(d, db, &to, std::mem::take(&mut batch), req.mode).await?;
                    size = 0;
                }
            }
            if !batch.is_empty() {
                rows += write(d, db, &to, batch, req.mode).await?;
            }
            if last.elapsed().as_millis() > 200 {
                last = Instant::now();
                progress(CopyProgress {
                    table: o.name.clone(),
                    index: i as f64,
                    tables: n,
                    rows: rows as f64,
                    estimate: est,
                    phase: "rows".into(),
                });
            }
            if id == 0 {
                break;
            }
            if stop.load(Ordering::SeqCst) {
                let _ = s
                    .command(&o.schema, doc! { "killCursors": &o.name, "cursors": [id] })
                    .await;
                return Err(stopped());
            }
            r = s
                .command(
                    &o.schema,
                    doc! { "getMore": id, "collection": &o.name, "batchSize": BATCH as i32 },
                )
                .await?;
        }
        Ok((rows, src_rows, idx))
    }

    async fn write(
        d: &MongoConn,
        db: &str,
        coll: &str,
        docs: Vec<Document>,
        mode: CopyMode,
    ) -> AppResult<u64> {
        let n = docs.len() as u64;
        if mode == CopyMode::Upsert {
            let updates: Vec<Document> = docs
                .into_iter()
                .map(|x| {
                    let id = x.get("_id").cloned().unwrap_or(Bson::Null);
                    doc! { "q": { "_id": id }, "u": x, "upsert": true }
                })
                .collect();
            d.command(
                db,
                doc! { "update": coll, "updates": updates, "ordered": true },
            )
            .await?;
        } else {
            d.command(
                db,
                doc! { "insert": coll, "documents": docs, "ordered": true },
            )
            .await?;
        }
        Ok(n)
    }
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod docker_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_say_what_they_do() {
        assert_eq!(action(CopyMode::Recreate, true), "drop and create");
        assert_eq!(action(CopyMode::Recreate, false), "create");
        assert_eq!(action(CopyMode::Upsert, true), "insert or replace");
        assert_eq!(family(Engine::Mariadb), family(Engine::Mysql));
        let o = CopyObject {
            schema: "public".into(),
            name: "t".into(),
            kind: "table".into(),
            filter: None,
            limit: None,
            rename: Some(" ".into()),
            types: None,
            ddl: None,
        };
        assert!(
            compatible(Engine::Postgres, Engine::Mariadb)
                && !compatible(Engine::Postgres, Engine::Mongodb)
        );
        assert_eq!(target_name(&o), "t");
    }
}
