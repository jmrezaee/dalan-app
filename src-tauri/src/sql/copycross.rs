//! Copies between PostgreSQL and MySQL/MariaDB (D13 part 2; replica/databases/restore-copy.md §3). The source table
//! is described (columns, keys, indexes, foreign keys), each column is mapped through `typemap` (or the type the person
//! chose in the review), the target's `create table` is generated (or the one the person edited, checked by the
//! classifier), and the rows stream through Dalan converted per column. Keys and indexes come after the data, foreign
//! keys after every table; views, functions and triggers are reported as not copied, with their source DDL.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};

use super::copy::{
    CopyContent, CopyJob, CopyMode, CopyObject, CopyProgress, CopyReport, CopyTableReport,
    NotCopied,
};
use super::mydb::{lit, q};
use super::mysql::MyConn;
use super::pg::{map_error, quote, PgConn};
use super::typemap::{self, Conv, Mapped};
use crate::error::{AppError, AppResult, ErrorCode};

/// One source column and where it goes.
#[derive(Debug, Clone)]
pub struct XCol {
    pub name: String,
    pub source_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    /// Identity, serial or AUTO_INCREMENT.
    pub auto: bool,
    pub labels: Vec<String>,
    pub mapped: Mapped,
}

/// (name, columns, referenced table, referenced columns, on delete, on update)
pub type Fk = (String, Vec<String>, String, Vec<String>, String, String);

#[derive(Debug, Clone)]
pub struct XTable {
    pub cols: Vec<XCol>,
    pub pk: Vec<String>,
    /// (name, unique, columns); expression indexes are left out (named in `skipped`).
    pub indexes: Vec<(String, bool, Vec<String>)>,
    /// (name, columns, referenced table, referenced columns, on delete, on update)
    pub fks: Vec<Fk>,
    pub skipped: Vec<String>,
}

fn pg_action(c: &str) -> &'static str {
    match c {
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        "r" => "RESTRICT",
        _ => "NO ACTION",
    }
}

/// A PostgreSQL table, mapped for MySQL.
pub async fn describe_pg(c: &PgConn, schema: &str, name: &str) -> AppResult<XTable> {
    let rel = format!("{}.{}", quote(schema), quote(name));
    let err = |e: tokio_postgres::Error| map_error(&e);
    let idx = c
        .client
        .query(
            "select i.relname::text, ix.indisunique, ix.indisprimary, ix.indexprs is null and ix.indpred is null,
                    array(select a.attname::text from unnest(ix.indkey) with ordinality k(n, o) join pg_attribute a on a.attrelid = ix.indrelid and a.attnum = k.n order by k.o)
             from pg_index ix join pg_class i on i.oid = ix.indexrelid where ix.indrelid = $1::text::regclass order by 1",
            &[&rel],
        )
        .await
        .map_err(err)?;
    let mut pk = Vec::new();
    let mut indexes = Vec::new();
    let mut skipped = Vec::new();
    for r in &idx {
        let (n, uniq, prim, plain, cols): (String, bool, bool, bool, Vec<String>) =
            (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4));
        if prim {
            pk = cols;
        } else if plain {
            indexes.push((n, uniq, cols));
        } else {
            skipped.push(format!("index {n} (on an expression or with a condition)"));
        }
    }
    let fks = c
        .client
        .query(
            "select c.conname::text,
                    array(select a.attname::text from unnest(c.conkey) with ordinality k(n, o) join pg_attribute a on a.attrelid = c.conrelid and a.attnum = k.n order by k.o),
                    cl.relname::text,
                    array(select a.attname::text from unnest(c.confkey) with ordinality k(n, o) join pg_attribute a on a.attrelid = c.confrelid and a.attnum = k.n order by k.o),
                    c.confdeltype::text, c.confupdtype::text
             from pg_constraint c join pg_class cl on cl.oid = c.confrelid where c.conrelid = $1::text::regclass and c.contype = 'f' order by 1",
            &[&rel],
        )
        .await
        .map_err(err)?
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), pg_action(&r.get::<_, String>(4)).to_string(), pg_action(&r.get::<_, String>(5)).to_string()))
        .collect::<Vec<_>>();
    let in_key = |n: &str| {
        pk.iter().any(|x| x == n)
            || indexes.iter().any(|i| i.2.iter().any(|x| x == n))
            || fks.iter().any(
                |f: &(String, Vec<String>, String, Vec<String>, String, String)| {
                    f.1.iter().any(|x| x == n)
                },
            )
    };
    let rows = c
        .client
        .query(
            "select a.attname::text, format_type(a.atttypid, a.atttypmod), t.typcategory::text, not a.attnotnull, pg_get_expr(d.adbin, d.adrelid),
                    a.attidentity::text, a.attgenerated::text,
                    array(select e.enumlabel::text from pg_enum e where e.enumtypid = t.oid order by e.enumsortorder)
             from pg_attribute a join pg_type t on t.oid = a.atttypid left join pg_attrdef d on d.adrelid = a.attrelid and d.adnum = a.attnum
             where a.attrelid = $1::text::regclass and a.attnum > 0 and not a.attisdropped order by a.attnum",
            &[&rel],
        )
        .await
        .map_err(err)?;
    let mut cols = Vec::new();
    for r in &rows {
        let (name, ty, cat, nullable, default, ident, gen, labels): (
            String,
            String,
            String,
            bool,
            Option<String>,
            String,
            String,
            Vec<String>,
        ) = (
            r.get(0),
            r.get(1),
            r.get(2),
            r.get(3),
            r.get(4),
            r.get(5),
            r.get(6),
            r.get(7),
        );
        if gen == "s" {
            skipped.push(format!(
                "generated column {name} (its expression isn't translated)"
            ));
            continue;
        }
        let auto = !ident.is_empty()
            || default
                .as_deref()
                .is_some_and(|d| d.starts_with("nextval("));
        let mapped = typemap::pg_to_my(&ty, &cat, in_key(&name));
        cols.push(XCol {
            name,
            source_type: ty,
            nullable,
            default: if auto { None } else { default },
            auto,
            labels,
            mapped,
        });
    }
    Ok(XTable {
        cols,
        pk,
        indexes,
        fks,
        skipped,
    })
}

/// A MySQL or MariaDB table, mapped for PostgreSQL.
pub async fn describe_my(c: &MyConn, db: &str, name: &str) -> AppResult<XTable> {
    let rows = c
        .query(
            &format!(
                "select column_name, column_type, is_nullable, column_default, extra from information_schema.columns where table_schema = {} and table_name = {} order by ordinal_position",
                lit(db),
                lit(name)
            ),
            0,
        )
        .await?
        .rows;
    let mut cols = Vec::new();
    let mut skipped = Vec::new();
    for r in rows {
        let get = |i: usize| r.get(i).cloned().flatten();
        let (name, ty, extra) = (
            get(0).unwrap_or_default(),
            get(1).unwrap_or_default(),
            get(4).unwrap_or_default().to_ascii_lowercase(),
        );
        if extra.contains("virtual generated") || extra.contains("stored generated") {
            skipped.push(format!(
                "generated column {name} (its expression isn't translated)"
            ));
            continue;
        }
        cols.push(XCol {
            labels: typemap::my_labels(&ty),
            mapped: typemap::my_to_pg(&ty),
            nullable: get(2).as_deref() == Some("YES"),
            default: get(3),
            auto: extra.contains("auto_increment"),
            source_type: ty,
            name,
        });
    }
    let stats = c
        .query(&format!("select index_name, non_unique, column_name from information_schema.statistics where table_schema = {} and table_name = {} order by index_name, seq_in_index", lit(db), lit(name)), 0)
        .await?
        .rows;
    let mut pk = Vec::new();
    let mut indexes: Vec<(String, bool, Vec<String>)> = Vec::new();
    for r in stats {
        let (idx, non_unique, col) = (
            r.first().cloned().flatten().unwrap_or_default(),
            r.get(1).cloned().flatten().unwrap_or_default(),
            r.get(2).cloned().flatten(),
        );
        let Some(col) = col else {
            skipped.push(format!("index {idx} (on an expression)"));
            continue;
        };
        if idx == "PRIMARY" {
            pk.push(col);
        } else if let Some(i) = indexes.iter_mut().find(|i| i.0 == idx) {
            i.2.push(col);
        } else {
            indexes.push((idx, non_unique == "0", vec![col]));
        }
    }
    let mut fks: Vec<Fk> = Vec::new();
    let rows = c
        .query(
            &format!(
                "select k.constraint_name, k.column_name, k.referenced_table_name, k.referenced_column_name, r.delete_rule, r.update_rule
                 from information_schema.key_column_usage k join information_schema.referential_constraints r
                   on r.constraint_schema = k.constraint_schema and r.constraint_name = k.constraint_name and r.table_name = k.table_name
                 where k.table_schema = {} and k.table_name = {} and k.referenced_table_name is not null order by k.constraint_name, k.ordinal_position",
                lit(db),
                lit(name)
            ),
            0,
        )
        .await?
        .rows;
    for r in rows {
        let g = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
        match fks.iter_mut().find(|f| f.0 == g(0)) {
            Some(f) => {
                f.1.push(g(1));
                f.3.push(g(3));
            }
            None => fks.push((g(0), vec![g(1)], g(2), vec![g(3)], g(4), g(5))),
        }
    }
    Ok(XTable {
        cols,
        pk,
        indexes,
        fks,
        skipped,
    })
}

/// The type the person chose for a column, if any.
fn chosen(o: &CopyObject, col: &str) -> Option<String> {
    o.types
        .as_ref()?
        .iter()
        .find(|t| t.name == col)
        .map(|t| t.target_type.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// The name of an enum type made for a column on PostgreSQL.
fn enum_name(schema: &str, table: &str, col: &str) -> String {
    format!("{}.{}", quote(schema), quote(&format!("{table}_{col}")))
}

fn is_int(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    [
        "smallint",
        "integer",
        "bigint",
        "int",
        "tinyint",
        "mediumint",
    ]
    .iter()
    .any(|x| t == *x || t.starts_with(&format!("{x} ")) || t.starts_with(&format!("{x}(")))
}

/// `create table` for the target. `to_my`: MySQL/MariaDB; otherwise PostgreSQL in `schema`. Returns the statements
/// to run before it (PostgreSQL enum types) and the statement.
pub fn create_sql(
    t: &XTable,
    o: &CopyObject,
    target: &str,
    schema: &str,
    to_my: bool,
) -> (Vec<String>, String) {
    let mut pre = Vec::new();
    let mut lines = Vec::new();
    for c in &t.cols {
        let mut ty = chosen(o, &c.name).unwrap_or_else(|| c.mapped.target.clone());
        if ty == "enum" {
            if to_my {
                ty = format!(
                    "enum({})",
                    c.labels
                        .iter()
                        .map(|l| lit(l))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            } else {
                let en = enum_name(schema, target, &c.name);
                let labels = c
                    .labels
                    .iter()
                    .map(|l| format!("'{}'", l.replace('\'', "''")))
                    .collect::<Vec<_>>()
                    .join(", ");
                pre.push(format!("do $$ begin if to_regtype('{}') is null then create type {en} as enum ({labels}); end if; end $$", en.replace('\'', "''")));
                ty = en;
            }
        }
        let mut l = if to_my {
            format!("  {} {ty}", q(&c.name))
        } else {
            format!("  {} {ty}", quote(&c.name))
        };
        let auto = c.auto && is_int(&ty);
        if auto && !to_my {
            l.push_str(" generated by default as identity");
        }
        if !c.nullable {
            l.push_str(" not null");
        }
        if auto && to_my {
            l.push_str(" auto_increment");
        }
        if let Some(d) = c
            .default
            .as_deref()
            .and_then(|d| typemap::default_for(d, to_my))
        {
            // MySQL wants the column's fraction digits on CURRENT_TIMESTAMP (datetime(6) → CURRENT_TIMESTAMP(6)).
            let d = match (to_my, d.as_str(), ty.find('(')) {
                (true, "CURRENT_TIMESTAMP", Some(p))
                    if ty.starts_with("datetime") || ty.starts_with("timestamp") =>
                {
                    format!("CURRENT_TIMESTAMP{}", &ty[p..])
                }
                _ => d,
            };
            if !(to_my && matches!(ty.as_str(), "longtext" | "json" | "longblob")) {
                l.push_str(&format!(" default {d}"));
            }
        }
        lines.push(l);
    }
    if !t.pk.is_empty() {
        let cols =
            t.pk.iter()
                .map(|c| if to_my { key_col(t, o, c) } else { quote(c) })
                .collect::<Vec<_>>()
                .join(", ");
        lines.push(format!("  primary key ({cols})"));
    }
    let head = if to_my {
        format!("create table {target}")
    } else {
        format!("create table {}.{}", quote(schema), quote(target))
    };
    let tail = if to_my {
        " engine=InnoDB default charset=utf8mb4"
    } else {
        ""
    };
    (pre, format!("{head} (\n{}\n){tail}", lines.join(",\n")))
}

/// A MySQL key column: long texts and blobs need a prefix length.
fn key_col(t: &XTable, o: &CopyObject, name: &str) -> String {
    let ty = t
        .cols
        .iter()
        .find(|c| c.name == name)
        .map(|c| chosen(o, name).unwrap_or_else(|| c.mapped.target.clone()))
        .unwrap_or_default();
    if matches!(
        ty.as_str(),
        "longtext" | "longblob" | "json" | "text" | "blob"
    ) {
        format!("{}(191)", q(name))
    } else {
        q(name)
    }
}

/// The source's SELECT expression for a PostgreSQL column on its way to MySQL.
fn pg_read(c: &XCol) -> String {
    let n = quote(&c.name);
    match c.mapped.conv {
        Conv::Bool => format!("{n}::int::text"),
        Conv::Bytes => format!("encode({n}, 'hex')"),
        Conv::Json => format!("to_json({n})::text"),
        Conv::Utc => format!("to_char({n} at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.US')"),
        Conv::TimeOnly => format!("{n}::time::text"),
        _ => format!("{n}::text"),
    }
}

/// One line of PostgreSQL COPY text output into its fields (`\N` is NULL; backslash escapes decoded).
pub fn copy_fields(line: &[u8]) -> Vec<Option<String>> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let mut out = Vec::new();
    for f in line.split(|b| *b == b'\t') {
        if f == b"\\N" {
            out.push(None);
            continue;
        }
        let mut v = Vec::with_capacity(f.len());
        let mut i = 0;
        while i < f.len() {
            if f[i] == b'\\' && i + 1 < f.len() {
                i += 1;
                match f[i] {
                    b'n' => v.push(b'\n'),
                    b't' => v.push(b'\t'),
                    b'r' => v.push(b'\r'),
                    b'b' => v.push(8),
                    b'f' => v.push(12),
                    b'v' => v.push(11),
                    b'x' if i + 1 < f.len() => {
                        let mut n = 0u8;
                        let mut k = 0;
                        while k < 2 && i + 1 < f.len() && f[i + 1].is_ascii_hexdigit() {
                            i += 1;
                            n = n * 16 + (f[i] as char).to_digit(16).unwrap_or(0) as u8;
                            k += 1;
                        }
                        v.push(n);
                    }
                    d @ b'0'..=b'7' => {
                        let mut n = u32::from(d - b'0');
                        let mut k = 1;
                        while k < 3 && i + 1 < f.len() && (b'0'..=b'7').contains(&f[i + 1]) {
                            i += 1;
                            n = n * 8 + u32::from(f[i] - b'0');
                            k += 1;
                        }
                        v.push(n as u8);
                    }
                    other => v.push(other),
                }
            } else {
                v.push(f[i]);
            }
            i += 1;
        }
        out.push(Some(String::from_utf8_lossy(&v).into_owned()));
    }
    out
}

/// A MySQL value as a field of PostgreSQL COPY text input.
pub fn copy_field(v: &Option<String>, conv: Conv) -> String {
    let Some(s) = v else { return "\\N".into() };
    let s = match conv {
        Conv::ZeroDate if s.starts_with("0000-00-00") => return "\\N".into(),
        Conv::Bool if s.starts_with("\\x") => if s
            .trim_start_matches("\\x")
            .trim_start_matches('0')
            .is_empty()
        {
            "0"
        } else {
            "1"
        }
        .to_string(),
        Conv::Set => {
            let items: Vec<String> = s
                .split(',')
                .filter(|x| !x.is_empty())
                .map(|x| format!("\"{}\"", x.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect();
            format!("{{{}}}", items.join(","))
        }
        _ => s.clone(),
    };
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// A value for MySQL from PostgreSQL's text: hex for binary, a quoted literal otherwise.
fn my_value(v: &Option<String>, conv: Conv) -> String {
    match v {
        None => "NULL".into(),
        Some(s) if conv == Conv::Bytes => format!("X'{s}'"),
        Some(s) => lit(s),
    }
}

fn stopped() -> AppError {
    AppError::new(ErrorCode::Internal, "Stopped.")
}

/// Checks DDL the person edited: one `create table` statement and nothing else, whatever the classifier makes of it.
pub fn check_ddl(ddl: &str, to_my: bool) -> AppResult<()> {
    use super::classify::{classify_for, split_for, Class, Dialect};
    let d = if to_my { Dialect::My } else { Dialect::Pg };
    let pieces = split_for(ddl, d);
    let one = pieces.len() == 1;
    let create = pieces.first().is_some_and(|p| {
        p.text
            .split_whitespace()
            .take(2)
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            == ["create", "table"]
    });
    if !one || !create || classify_for(ddl, d) == Class::Dangerous {
        return Err(AppError::validation(
            "The edited DDL must be one CREATE TABLE statement and nothing else.",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    job: &CopyJob,
    src: &super::RawConn,
    dst: &super::RawConn,
    stop: &AtomicBool,
    progress: &(dyn Fn(CopyProgress) + Sync),
    rep: &mut CopyReport,
) -> AppResult<()> {
    use super::RawConn;
    let req = &job.req;
    let to_my = matches!(dst, RawConn::My(_));
    match src {
        RawConn::Pg(s) => {
            s.client.batch_execute("set intervalstyle = 'iso_8601'; begin isolation level repeatable read read only").await.map_err(|e| map_error(&e))?;
        }
        RawConn::My(s) => {
            s.query("set session transaction isolation level repeatable read", 0)
                .await?;
            s.query("start transaction with consistent snapshot, read only", 0)
                .await?;
        }
        RawConn::Mongo(_) => {
            return Err(AppError::validation("MongoDB copies go to MongoDB only."))
        }
    }
    if let RawConn::My(d) = dst {
        d.query("set foreign_key_checks = 0", 0).await?;
    }
    let schema = "public";
    let n = req.objects.len() as f64;
    let mut made: Vec<(XTable, &CopyObject, String)> = Vec::new();
    for (i, o) in req.objects.iter().enumerate() {
        let mut tr = CopyTableReport {
            source: format!("{}.{}", o.schema, o.name),
            target: super::copy::target_name(o),
            status: "skipped".into(),
            rows: 0.0,
            source_rows: None,
            target_rows: None,
            error: None,
            seconds: 0.0,
        };
        if o.kind != "table" {
            // Views and the like aren't translated: their DDL goes in the report.
            let ddl = match src {
                RawConn::Pg(s) => s
                    .client
                    .query_opt(
                        "select pg_get_viewdef($1::text::regclass, true)",
                        &[&format!("{}.{}", quote(&o.schema), quote(&o.name))],
                    )
                    .await
                    .ok()
                    .flatten()
                    .map(|r| {
                        format!(
                            "create view {} as\n{}",
                            quote(&o.name),
                            r.get::<_, String>(0)
                        )
                    }),
                RawConn::My(s) => s
                    .query(
                        &format!("show create view {}.{}", q(&o.schema), q(&o.name)),
                        1,
                    )
                    .await
                    .ok()
                    .and_then(|r| r.rows.into_iter().next())
                    .and_then(|r| r.into_iter().nth(1).flatten()),
                RawConn::Mongo(_) => None,
            };
            rep.not_copied.push(NotCopied {
                name: tr.source.clone(),
                kind: o.kind.clone(),
                ddl: ddl.unwrap_or_default(),
            });
            rep.tables.push(tr);
            continue;
        }
        if stop.load(Ordering::SeqCst) {
            tr.status = "stopped".into();
            rep.tables.push(tr);
            continue;
        }
        let t0 = Instant::now();
        let est = job.plans.get(i).and_then(|p| p.rows_estimate);
        let res = one(job, src, dst, o, schema, to_my, i, n, est, stop, progress).await;
        tr.seconds = t0.elapsed().as_secs_f64();
        match res {
            Ok((t, rows, src_rows, target_ref)) => {
                tr.status = "done".into();
                tr.rows = rows as f64;
                tr.source_rows = Some(src_rows as f64);
                tr.target_rows = count(dst, &target_ref).await;
                for s in &t.skipped {
                    rep.warnings
                        .push(format!("{}: {s} wasn't copied.", tr.source));
                }
                for c in t.cols.iter().filter(|c| chosen(o, &c.name).is_none()) {
                    if let Some(why) = &c.mapped.lossy {
                        rep.warnings
                            .push(format!("{}.{}: {why}.", tr.source, c.name));
                    }
                }
                made.push((t, o, target_ref));
            }
            Err(e) => {
                match dst {
                    RawConn::Pg(d) => {
                        let _ = d.client.batch_execute("rollback").await;
                    }
                    RawConn::My(d) => {
                        let _ = d.query("rollback", 0).await;
                    }
                    RawConn::Mongo(_) => {}
                }
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
                    for o in &req.objects[i + 1..] {
                        rep.tables.push(CopyTableReport {
                            source: format!("{}.{}", o.schema, o.name),
                            target: super::copy::target_name(o),
                            status: "skipped".into(),
                            rows: 0.0,
                            source_rows: None,
                            target_rows: None,
                            error: None,
                            seconds: 0.0,
                        });
                    }
                    break;
                }
                continue;
            }
        }
        rep.tables.push(tr);
    }
    // Indexes, then foreign keys, then identities moved on.
    if req.content != CopyContent::Data && matches!(req.mode, CopyMode::Create | CopyMode::Recreate)
    {
        progress(CopyProgress {
            table: String::new(),
            index: n,
            tables: n,
            rows: 0.0,
            estimate: None,
            phase: "keys".into(),
        });
        let rename = |table: &str| {
            req.objects
                .iter()
                .find(|o| o.name == table)
                .map_or(table.to_string(), super::copy::target_name)
        };
        for (t, o, target) in &made {
            let short = super::copy::target_name(o);
            for (name, uniq, cols) in &t.indexes {
                let u = if *uniq { "unique " } else { "" };
                let sql = if to_my {
                    format!(
                        "create {u}index {} on {target} ({})",
                        q(&format!("{short}_{name}")
                            .chars()
                            .take(64)
                            .collect::<String>()),
                        cols.iter()
                            .map(|c| key_col(t, o, c))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    format!(
                        "create {u}index {} on {target} ({})",
                        quote(
                            &format!("{short}_{name}")
                                .chars()
                                .take(63)
                                .collect::<String>()
                        ),
                        cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ")
                    )
                };
                if let Err(e) = exec(dst, &sql).await {
                    rep.warnings
                        .push(format!("{short}: index {name} wasn't made: {}", e.message));
                }
            }
        }
        for (t, o, target) in &made {
            let short = super::copy::target_name(o);
            for (name, cols, rt, rc, del, upd) in &t.fks {
                let rt = rename(rt);
                let (cols, rc, rt) = if to_my {
                    (
                        cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "),
                        rc.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "),
                        format!("{}.{}", q(&job.target_db), q(&rt)),
                    )
                } else {
                    (
                        cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", "),
                        rc.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", "),
                        format!("{}.{}", quote(schema), quote(&rt)),
                    )
                };
                let nm = if to_my { q(name) } else { quote(name) };
                let sql = format!("alter table {target} add constraint {nm} foreign key ({cols}) references {rt} ({rc}) on delete {del} on update {upd}");
                if let Err(e) = exec(dst, &sql).await {
                    rep.warnings.push(format!(
                        "{short}: foreign key {name} wasn't added: {}",
                        e.message
                    ));
                }
            }
        }
    }
    if !to_my && req.content != CopyContent::Schema {
        for (t, _, target) in &made {
            for c in t.cols.iter().filter(|c| c.auto && is_int(&c.mapped.target)) {
                let sql = format!("select setval(pg_get_serial_sequence('{}', '{}'), coalesce(max({n}), 1), max({n}) is not null) from {target}", target.replace('\'', "''"), c.name.replace('\'', "''"), n = quote(&c.name));
                if let Err(e) = exec(dst, &sql).await {
                    rep.warnings.push(format!(
                        "{target}: the identity of {} wasn't moved on: {}",
                        c.name, e.message
                    ));
                }
            }
        }
    }
    if let RawConn::My(d) = dst {
        let _ = d.query("set foreign_key_checks = 1", 0).await;
    }
    match src {
        RawConn::Pg(s) => {
            let _ = s.client.batch_execute("rollback").await;
        }
        RawConn::My(s) => {
            let _ = s.query("rollback", 0).await;
        }
        RawConn::Mongo(_) => {}
    }
    Ok(())
}

async fn exec(c: &super::RawConn, sql: &str) -> AppResult<()> {
    match c {
        super::RawConn::Pg(d) => d.client.batch_execute(sql).await.map_err(|e| map_error(&e)),
        super::RawConn::My(d) => d.query(sql, 0).await.map(|_| ()),
        super::RawConn::Mongo(_) => Ok(()),
    }
}

async fn count(c: &super::RawConn, target: &str) -> Option<f64> {
    match c {
        super::RawConn::Pg(d) => d
            .client
            .query_one(&format!("select count(*) from {target}"), &[])
            .await
            .ok()
            .map(|r| r.get::<_, i64>(0) as f64),
        super::RawConn::My(d) => d
            .query(&format!("select count(*) from {target}"), 0)
            .await
            .ok()
            .and_then(|r| r.rows.into_iter().next())
            .and_then(|r| r.into_iter().next().flatten())
            .and_then(|v| v.parse().ok()),
        super::RawConn::Mongo(_) => None,
    }
}

/// One table: describe, create (by the mode), stream the rows converted, in one target transaction.
#[allow(clippy::too_many_arguments)]
async fn one(
    job: &CopyJob,
    src: &super::RawConn,
    dst: &super::RawConn,
    o: &CopyObject,
    schema: &str,
    to_my: bool,
    i: usize,
    n: f64,
    est: Option<f64>,
    stop: &AtomicBool,
    progress: &(dyn Fn(CopyProgress) + Sync),
) -> AppResult<(XTable, u64, u64, String)> {
    use super::RawConn;
    let req = &job.req;
    let t = match src {
        RawConn::Pg(s) => describe_pg(s, &o.schema, &o.name).await?,
        RawConn::My(s) => describe_my(s, &o.schema, &o.name).await?,
        RawConn::Mongo(_) => unreachable!("checked by the caller"),
    };
    let short = super::copy::target_name(o);
    let target = if to_my {
        format!("{}.{}", q(&job.target_db), q(&short))
    } else {
        format!("{}.{}", quote(schema), quote(&short))
    };
    let filter = o.filter.as_deref().map(str::trim).filter(|f| !f.is_empty());
    let limit = o.limit.filter(|l| *l >= 0.0).map(|l| l as u64);
    let (pre, generated) = create_sql(&t, o, &short, schema, to_my);
    let ddl = match o.ddl.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            check_ddl(d, to_my)?;
            d.to_string()
        }
        None => generated,
    };
    // PostgreSQL DDL is transactional, so the table and its rows go together; MySQL commits DDL at once.
    match dst {
        RawConn::Pg(d) => d
            .client
            .batch_execute("begin")
            .await
            .map_err(|e| map_error(&e))?,
        RawConn::My(_) => {}
        RawConn::Mongo(_) => unreachable!(),
    }
    if req.content != CopyContent::Data && matches!(req.mode, CopyMode::Create | CopyMode::Recreate)
    {
        if !to_my {
            exec(
                dst,
                &format!("create schema if not exists {}", quote(schema)),
            )
            .await?;
        }
        if req.mode == CopyMode::Recreate {
            exec(
                dst,
                &format!(
                    "drop table if exists {target}{}",
                    if to_my { "" } else { " cascade" }
                ),
            )
            .await?;
        }
        for p in &pre {
            exec(dst, p).await?;
        }
        exec(dst, &ddl).await?;
    }
    if req.mode == CopyMode::Empty {
        exec(dst, &format!("delete from {target}")).await?;
    }
    if to_my {
        exec(dst, "start transaction").await?;
    }
    let mut rows = 0u64;
    let mut src_rows = 0u64;
    let cols: Vec<&XCol> = t.cols.iter().collect();
    if req.content != CopyContent::Schema {
        let pk: Vec<String> = t.pk.clone();
        match (src, dst) {
            (RawConn::Pg(s), RawConn::My(d)) => {
                if let Some(f) = filter {
                    super::pg::check_filter(f)?;
                }
                let srcrel = format!("{}.{}", quote(&o.schema), quote(&o.name));
                let tail = format!(
                    "{}{}",
                    filter.map(|f| format!(" where ({f})")).unwrap_or_default(),
                    limit.map(|l| format!(" limit {l}")).unwrap_or_default()
                );
                src_rows = s
                    .client
                    .query_one(
                        &format!("select count(*) from (select 1 from {srcrel}{tail}) x"),
                        &[],
                    )
                    .await
                    .map_err(|e| map_error(&e))?
                    .get::<_, i64>(0) as u64;
                let list = cols
                    .iter()
                    .map(|c| pg_read(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let names = cols
                    .iter()
                    .map(|c| q(&c.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let upsert = if req.mode == CopyMode::Upsert {
                    if pk.is_empty() {
                        return Err(AppError::new(ErrorCode::SqlNoKey, format!("{srcrel} has no primary key, so its rows can't be matched for “insert or replace”.")));
                    }
                    let set: Vec<String> = cols
                        .iter()
                        .filter(|c| !pk.contains(&c.name))
                        .map(|c| format!("{0} = values({0})", q(&c.name)))
                        .collect();
                    if set.is_empty() {
                        format!(" on duplicate key update {0} = {0}", q(&pk[0]))
                    } else {
                        format!(" on duplicate key update {}", set.join(", "))
                    }
                } else {
                    String::new()
                };
                let out = s
                    .client
                    .copy_out(&format!(
                        "copy (select {list} from {srcrel}{tail}) to stdout"
                    ))
                    .await
                    .map_err(|e| map_error(&e))?;
                futures_util::pin_mut!(out);
                let mut partial: Vec<u8> = Vec::new();
                let mut batch = String::new();
                let mut last = Instant::now();
                // One masker for the table: a shuffle's held-back rows come out once the source has ended.
                let mask = super::copy::masker(
                    job,
                    o,
                    &cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                );
                let mut ended = false;
                while !ended {
                    let mut lines: Vec<Vec<Option<String>>> = Vec::new();
                    match out.next().await {
                        Some(chunk) => {
                            let chunk = chunk.map_err(|e| map_error(&e))?;
                            if stop.load(Ordering::SeqCst) {
                                return Err(stopped());
                            }
                            partial.extend_from_slice(&chunk);
                            while let Some(p) = partial.iter().position(|b| *b == b'\n') {
                                let line: Vec<u8> = partial.drain(..=p).collect();
                                lines.push(copy_fields(&line));
                            }
                            if let Some(m) = &mask {
                                m.batch(&mut lines);
                            }
                        }
                        None => {
                            ended = true;
                            if let Some(m) = &mask {
                                lines = m.finish();
                            }
                        }
                    }
                    for f in lines {
                        let vals = f
                            .iter()
                            .zip(&cols)
                            .map(|(v, c)| my_value(v, c.mapped.conv))
                            .collect::<Vec<_>>()
                            .join(", ");
                        if batch.is_empty() {
                            batch = format!("insert into {target} ({names}) values ({vals})");
                        } else {
                            batch.push_str(&format!(", ({vals})"));
                        }
                        rows += 1;
                        if batch.len() > 1 << 20 {
                            d.query(&format!("{batch}{upsert}"), 0).await?;
                            batch.clear();
                        }
                    }
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
                if !batch.is_empty() {
                    d.query(&format!("{batch}{upsert}"), 0).await?;
                }
                d.query("commit", 0).await?;
            }
            (RawConn::My(s), RawConn::Pg(d)) => {
                if let Some(f) = filter {
                    super::mydb::check_filter(f)?;
                }
                let srcrel = format!("{}.{}", q(&o.schema), q(&o.name));
                let tail = format!(
                    "{}{}",
                    filter.map(|f| format!(" where ({f})")).unwrap_or_default(),
                    limit.map(|l| format!(" limit {l}")).unwrap_or_default()
                );
                src_rows = s
                    .query(
                        &format!("select count(*) from (select 1 from {srcrel}{tail}) x"),
                        0,
                    )
                    .await?
                    .rows
                    .into_iter()
                    .next()
                    .and_then(|r| r.into_iter().next().flatten())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let names = cols
                    .iter()
                    .map(|c| quote(&c.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let into = if req.mode == CopyMode::Upsert {
                    if pk.is_empty() {
                        return Err(AppError::new(ErrorCode::SqlNoKey, format!("{srcrel} has no primary key, so its rows can't be matched for “insert or replace”.")));
                    }
                    d.client.batch_execute(&format!("create temp table _dalan_stage (like {target} including defaults) on commit drop")).await.map_err(|e| map_error(&e))?;
                    "_dalan_stage".to_string()
                } else {
                    target.clone()
                };
                let sink = d
                    .client
                    .copy_in::<_, bytes::Bytes>(&format!("copy {into} ({names}) from stdin"))
                    .await
                    .map_err(|e| map_error(&e))?;
                futures_util::pin_mut!(sink);
                let mut pager = super::copy::MyPager::new(
                    &o.schema,
                    &o.name,
                    cols.iter().map(|c| c.name.clone()).collect(),
                    pk.clone(),
                    filter.map(str::to_string),
                    limit,
                );
                let mut last = Instant::now();
                // One masker for the table: a shuffle's held-back rows come out after the last page.
                let mask = super::copy::masker(
                    job,
                    o,
                    &cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                );
                let mut tail_done = false;
                loop {
                    let page = match pager.next(s).await? {
                        Some(p) => p,
                        None if !tail_done => {
                            tail_done = true;
                            match &mask {
                                Some(m) => m.finish(),
                                None => break,
                            }
                        }
                        None => break,
                    };
                    if stop.load(Ordering::SeqCst) {
                        return Err(stopped());
                    }
                    let mut buf = String::new();
                    let mut page = page;
                    if let Some(m) = &mask {
                        m.batch(&mut page);
                    }
                    for row in &page {
                        let line = row
                            .iter()
                            .zip(&cols)
                            .map(|(v, c)| copy_field(v, c.mapped.conv))
                            .collect::<Vec<_>>()
                            .join("\t");
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    rows += page.len() as u64;
                    sink.send(bytes::Bytes::from(buf))
                        .await
                        .map_err(|e| map_error(&e))?;
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
                    let pkq: Vec<String> = pk.iter().map(|c| quote(c)).collect();
                    let set: Vec<String> = cols
                        .iter()
                        .filter(|c| !pk.contains(&c.name))
                        .map(|c| format!("{0} = excluded.{0}", quote(&c.name)))
                        .collect();
                    let action = if set.is_empty() {
                        "do nothing".to_string()
                    } else {
                        format!("do update set {}", set.join(", "))
                    };
                    d.client.batch_execute(&format!("insert into {target} ({names}) select {names} from _dalan_stage on conflict ({}) {action}", pkq.join(", "))).await.map_err(|e| map_error(&e))?;
                }
                d.client
                    .batch_execute("commit")
                    .await
                    .map_err(|e| map_error(&e))?;
            }
            _ => {
                return Err(AppError::validation(
                    "These two engines don't copy to each other.",
                ))
            }
        }
    } else {
        exec(dst, "commit").await?;
    }
    Ok((t, rows, src_rows, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_text_is_read_and_written_both_ways() {
        assert_eq!(
            copy_fields(b"1\t\\N\ttab\\there\tline\\nbreak\tback\\\\slash\t\\x41\t\\101\n"),
            vec![
                Some("1".into()),
                None,
                Some("tab\there".into()),
                Some("line\nbreak".into()),
                Some("back\\slash".into()),
                Some("A".into()),
                Some("A".into())
            ]
        );
        assert_eq!(
            copy_field(&Some("a\tb\nc\\d".into()), Conv::Plain),
            "a\\tb\\nc\\\\d"
        );
        assert_eq!(copy_field(&None, Conv::Plain), "\\N");
        assert_eq!(
            copy_field(&Some("0000-00-00 00:00:00".into()), Conv::ZeroDate),
            "\\N"
        );
        assert_eq!(
            copy_field(&Some("a,b\"c".into()), Conv::Set),
            "{\"a\",\"b\\\\\"c\"}"
        );
        assert_eq!(copy_field(&Some("\\x01".into()), Conv::Bool), "1");
        assert_eq!(
            copy_field(&Some("\\xdead".into()), Conv::Bytes),
            "\\\\xdead"
        );
        assert_eq!(my_value(&Some("dead".into()), Conv::Bytes), "X'dead'");
    }

    #[test]
    fn edited_ddl_must_be_one_create_table() {
        assert!(check_ddl("create table t (a int)", true).is_ok());
        assert!(check_ddl("create table t (a int); drop table x", true).is_err());
        assert!(check_ddl("drop table t", false).is_err());
        assert!(check_ddl("insert into t values (1)", false).is_err());
    }
}
