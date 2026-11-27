//! Reading two tables for a data compare (D16, E4), and making the target match. Each side is read a page at a time
//! in key order (keyset paging), as text, then put in its canonical form (`dbs::datadiff`). Text keys are ordered
//! bytewise on both servers (`COLLATE "C"`, `BINARY`) so the merge sees the same order on both sides.
//!
//! The quick pass (same engine only) splits the source's keys into ranges of 10,000 rows, hashes each range on
//! both servers, and merges only the ranges whose hashes differ.

use std::io::Write;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::dbs::datadiff::{self, canon, Event, Row, RowSource, ValueKind};
use crate::dbs::schemadiff::{quote_my, quote_pg, Family};
use crate::error::{AppError, AppResult};

use super::mydb::lit;
use super::pg::map_error;
use super::RawConn;

const PAGE: usize = 5000;
const RANGE: usize = 10_000;
const SAMPLES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DataSide {
    pub conn_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct Col {
    pub name: String,
    pub data_type: String,
    /// The type as the table declares it (`numeric(6,2)`, `varchar(40)`), for a manifest.
    pub full_type: String,
    pub kind: ValueKind,
}

fn family(c: &RawConn) -> AppResult<Family> {
    match c {
        RawConn::Pg(_) => Ok(Family::Pg),
        RawConn::My(_) => Ok(Family::My),
        RawConn::Mongo(_) => Err(AppError::validation(
            "Data compare is for PostgreSQL, MySQL and MariaDB tables.",
        )),
    }
}

/// A query's rows as text (either SQL engine).
pub async fn text_rows_pub(c: &RawConn, sql: &str) -> AppResult<Vec<Vec<Option<String>>>> {
    text_rows(c, sql).await
}

async fn text_rows(c: &RawConn, sql: &str) -> AppResult<Vec<Vec<Option<String>>>> {
    match c {
        RawConn::Pg(p) => Ok(p
            .client
            .query(sql, &[])
            .await
            .map_err(|e| map_error(&e))?
            .iter()
            .map(|r| {
                (0..r.len())
                    .map(|i| r.get::<_, Option<String>>(i))
                    .collect()
            })
            .collect()),
        RawConn::My(m) => Ok(m.query(sql, 0).await?.rows),
        RawConn::Mongo(_) => Err(AppError::validation("Data compare is for SQL tables.")),
    }
}

fn lit_for(f: Family, v: &str) -> String {
    match f {
        Family::Pg => format!("'{}'", v.replace('\'', "''")),
        Family::My => lit(v),
    }
}

fn table_name(f: Family, schema: &str, table: &str) -> String {
    match f {
        Family::Pg => format!("{}.{}", quote_pg(schema), quote_pg(table)),
        Family::My => format!("{}.{}", quote_my(schema), quote_my(table)),
    }
}

/// The table's columns (name, type, kind) in order, and its primary key.
pub async fn columns(c: &RawConn, schema: &str, table: &str) -> AppResult<(Vec<Col>, Vec<String>)> {
    let f = family(c)?;
    let (cols_sql, pk_sql) = match f {
        Family::Pg => (
            format!("select c.column_name::text, c.data_type::text, format_type(a.atttypid, a.atttypmod) from information_schema.columns c join pg_attribute a on a.attrelid = (quote_ident(c.table_schema) || '.' || quote_ident(c.table_name))::regclass and a.attname = c.column_name where c.table_schema = {} and c.table_name = {} order by c.ordinal_position", lit_for(f, schema), lit_for(f, table)),
            format!("select a.attname::text from pg_index x join pg_attribute a on a.attrelid = x.indrelid and a.attnum = any(x.indkey) where x.indisprimary and x.indrelid = (quote_ident({})||'.'||quote_ident({}))::regclass order by array_position(x.indkey, a.attnum)", lit_for(f, schema), lit_for(f, table)),
        ),
        Family::My => (
            format!("select column_name, data_type, column_type from information_schema.columns where table_schema = {} and table_name = {} order by ordinal_position", lit(schema), lit(table)),
            format!("select column_name from information_schema.key_column_usage where table_schema = {} and table_name = {} and constraint_name = 'PRIMARY' order by ordinal_position", lit(schema), lit(table)),
        ),
    };
    let cols: Vec<Col> = text_rows(c, &cols_sql)
        .await?
        .into_iter()
        .filter_map(|r| {
            let name = r.first().cloned().flatten()?;
            let dt = r.get(1).cloned().flatten().unwrap_or_default();
            let ct = r.get(2).cloned().flatten().unwrap_or_default();
            Some(Col {
                kind: datadiff::kind_of(&dt, &ct),
                name,
                data_type: dt,
                full_type: ct,
            })
        })
        .collect();
    if cols.is_empty() {
        return Err(AppError::not_found(format!(
            "There's no table {schema}.{table} there."
        )));
    }
    let pk = text_rows(c, &pk_sql)
        .await?
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect();
    Ok((cols, pk))
}

/// What one side reads: its table, key and compared columns (in the same order on both sides).
#[derive(Clone)]
pub struct Reader {
    pub family: Family,
    pub table: String,
    pub key: Vec<Col>,
    pub cols: Vec<Col>,
}

impl Reader {
    fn q(&self, n: &str) -> String {
        match self.family {
            Family::Pg => quote_pg(n),
            Family::My => quote_my(n),
        }
    }
    /// The column as text (UTC for PostgreSQL timestamps with a zone).
    fn select(&self, c: &Col) -> String {
        let n = self.q(&c.name);
        match self.family {
            Family::Pg if c.data_type == "timestamp with time zone" => {
                format!("to_char({n} at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.US')")
            }
            Family::Pg => format!("{n}::text"),
            Family::My => n,
        }
    }
    /// The key column as ordered: text bytewise on both servers.
    fn ordered(&self, c: &Col) -> String {
        let n = self.q(&c.name);
        let text = matches!(
            c.data_type.as_str(),
            "text"
                | "character varying"
                | "character"
                | "varchar"
                | "char"
                | "tinytext"
                | "mediumtext"
                | "longtext"
        );
        match (self.family, text) {
            (Family::Pg, true) => format!("{n} COLLATE \"C\""),
            (Family::My, true) => format!("BINARY {n}"),
            _ => n,
        }
    }
    fn tuple(&self, vals: &[Option<String>]) -> String {
        let parts: Vec<String> = vals
            .iter()
            .map(|v| {
                v.as_deref()
                    .map_or("NULL".into(), |v| lit_for(self.family, v))
            })
            .collect();
        format!("({})", parts.join(", "))
    }
    fn order(&self) -> String {
        self.key
            .iter()
            .map(|k| self.ordered(k))
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn range(&self, lo: Option<&[Option<String>]>, hi: Option<&[Option<String>]>) -> String {
        let keys = format!("({})", self.order());
        let mut w = Vec::new();
        if let Some(lo) = lo {
            w.push(format!("{keys} > {}", self.tuple(lo)));
        }
        if let Some(hi) = hi {
            w.push(format!("{keys} <= {}", self.tuple(hi)));
        }
        if w.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", w.join(" AND "))
        }
    }
    fn row(&self, raw: Vec<Option<String>>) -> Row {
        let nk = self.key.len();
        let key = raw[..nk]
            .iter()
            .zip(&self.key)
            .map(|(v, c)| v.as_deref().map(|v| canon(c.kind, v)))
            .collect();
        let values = raw[nk..]
            .iter()
            .zip(&self.cols)
            .map(|(v, c)| v.as_deref().map(|v| canon(c.kind, v)))
            .collect();
        Row { key, values }
    }
}

/// One side's rows between `lo` (exclusive) and `hi` (inclusive), a page at a time.
pub struct Pager<'a> {
    pub conn: &'a RawConn,
    pub r: &'a Reader,
    pub after: Option<Vec<Option<String>>>,
    pub hi: Option<Vec<Option<String>>>,
    pub done: bool,
    pub seen: u64,
}

impl RowSource for Pager<'_> {
    async fn page(&mut self) -> AppResult<Vec<Row>> {
        if self.done {
            return Ok(vec![]);
        }
        let r = self.r;
        // Aliased, so ORDER BY reads the columns and not the text forms of the same name.
        let cols: Vec<String> = r
            .key
            .iter()
            .chain(&r.cols)
            .enumerate()
            .map(|(i, c)| format!("{} AS dalan_c{i}", r.select(c)))
            .collect();
        let sql = format!(
            "SELECT {} FROM {}{} ORDER BY {} LIMIT {PAGE}",
            cols.join(", "),
            r.table,
            r.range(self.after.as_deref(), self.hi.as_deref()),
            r.order()
        );
        let raw = text_rows(self.conn, &sql).await?;
        if raw.len() < PAGE {
            self.done = true;
        }
        // The next page starts after the last key as the server sent it (before canonical forms).
        if let Some(last) = raw.last() {
            self.after = Some(last[..r.key.len()].to_vec());
        }
        self.seen += raw.len() as u64;
        Ok(raw.into_iter().map(|x| r.row(x)).collect())
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DataChange {
    pub column: String,
    pub source: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DataSample {
    /// `only_source`, `only_target` or `different`.
    pub kind: String,
    pub key: String,
    pub changes: Vec<DataChange>,
}

#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct DataCompareResult {
    pub only_source: f64,
    pub only_target: f64,
    pub different: f64,
    pub identical: f64,
    /// Up to 100 of each, in key order.
    pub samples: Vec<DataSample>,
    pub columns: Vec<String>,
    pub key: Vec<String>,
    /// Columns left out (binary, or only on one side).
    pub skipped: Vec<String>,
    pub ranges: f64,
    pub ranges_same: f64,
    pub warnings: Vec<String>,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DataProgress {
    pub rows: f64,
    pub ranges_done: f64,
    pub ranges: f64,
}

/// Both sides' readers for these tables and key (the key defaults to the source's primary key).
pub async fn readers(
    src: &RawConn,
    s: &DataSide,
    dst: &RawConn,
    t: &DataSide,
    key: &[String],
) -> AppResult<(Reader, Reader, Vec<String>)> {
    let (fs, fd) = (family(src)?, family(dst)?);
    let (sc, spk) = columns(src, &s.schema, &s.table).await?;
    let (tc, _) = columns(dst, &t.schema, &t.table).await?;
    let key: Vec<String> = if key.is_empty() { spk } else { key.to_vec() };
    if key.is_empty() {
        return Err(AppError::validation(format!(
            "{}.{} has no primary key: choose the columns that identify a row.",
            s.schema, s.table
        )));
    }
    let mut skipped = Vec::new();
    let (mut sk, mut tk, mut scs, mut tcs) = (vec![], vec![], vec![], vec![]);
    for k in &key {
        let (Some(a), Some(b)) = (
            sc.iter().find(|c| &c.name == k),
            tc.iter().find(|c| &c.name == k),
        ) else {
            return Err(AppError::validation(format!(
                "The key column {k} must be in both tables."
            )));
        };
        sk.push(a.clone());
        tk.push(b.clone());
    }
    for c in &sc {
        if key.contains(&c.name) {
            continue;
        }
        match tc.iter().find(|x| x.name == c.name) {
            Some(o) if c.kind != ValueKind::Binary && o.kind != ValueKind::Binary => {
                scs.push(c.clone());
                tcs.push(o.clone());
            }
            Some(_) => skipped.push(format!("{} (binary)", c.name)),
            None => skipped.push(format!("{} (only in the source)", c.name)),
        }
    }
    for c in &tc {
        if !key.contains(&c.name) && !sc.iter().any(|x| x.name == c.name) {
            skipped.push(format!("{} (only in the target)", c.name));
        }
    }
    Ok((
        Reader {
            family: fs,
            table: table_name(fs, &s.schema, &s.table),
            key: sk,
            cols: scs,
        },
        Reader {
            family: fd,
            table: table_name(fd, &t.schema, &t.table),
            key: tk,
            cols: tcs,
        },
        skipped,
    ))
}

/// The source's key every 10,000 rows: the ranges of the quick pass end there.
async fn boundaries(
    c: &RawConn,
    r: &Reader,
    stop: &AtomicBool,
) -> AppResult<Vec<Vec<Option<String>>>> {
    let keys: Vec<String> = r
        .key
        .iter()
        .enumerate()
        .map(|(i, k)| format!("{} AS dalan_k{i}", r.select(k)))
        .collect();
    let mut out: Vec<Vec<Option<String>>> = Vec::new();
    loop {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::new(crate::error::ErrorCode::Internal, "Stopped."));
        }
        let sql = format!(
            "SELECT {} FROM {}{} ORDER BY {} LIMIT 1 OFFSET {}",
            keys.join(", "),
            r.table,
            r.range(out.last().map(Vec::as_slice), None),
            r.order(),
            RANGE - 1
        );
        match text_rows(c, &sql).await?.into_iter().next() {
            Some(k) => out.push(k),
            None => return Ok(out),
        }
    }
}

/// A range's row count and hash (the same expression on both sides of one engine).
async fn range_hash(
    c: &RawConn,
    r: &Reader,
    lo: Option<&[Option<String>]>,
    hi: Option<&[Option<String>]>,
) -> AppResult<(u64, String)> {
    let cols: Vec<String> = r.key.iter().chain(&r.cols).map(|c| r.select(c)).collect();
    let sql = match r.family {
        Family::Pg => format!(
            "SELECT count(*)::text, md5(string_agg(md5(concat_ws(chr(31), {})), '' ORDER BY {})) FROM {}{}",
            cols.iter().map(|c| format!("coalesce({c}, '\\N')")).collect::<Vec<_>>().join(", "),
            r.order(),
            r.table,
            r.range(lo, hi)
        ),
        Family::My => format!(
            "SELECT count(*), md5(group_concat(md5(concat_ws(char(31), {})) ORDER BY {} SEPARATOR '')) FROM {}{}",
            cols.iter().map(|c| format!("ifnull({c}, '\\\\N')")).collect::<Vec<_>>().join(", "),
            r.order(),
            r.table,
            r.range(lo, hi)
        ),
    };
    let row = text_rows(c, &sql)
        .await?
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok((
        row.first()
            .cloned()
            .flatten()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0),
        row.get(1).cloned().flatten().unwrap_or_default(),
    ))
}

/// Walks every difference: the whole tables, or (quick pass) only the ranges whose hashes differ. `same_rows` is
/// told how many identical rows a skipped range held.
#[allow(clippy::too_many_arguments)]
pub async fn walk(
    src: &RawConn,
    sr: &Reader,
    dst: &RawConn,
    tr: &Reader,
    quick: bool,
    stop: &AtomicBool,
    progress: &(dyn Fn(DataProgress) + Sync),
    on: &mut (dyn FnMut(Event<'_>) -> AppResult<()> + Send),
) -> AppResult<(u64, u64, u64)> {
    if !quick {
        let mut a = Pager {
            conn: src,
            r: sr,
            after: None,
            hi: None,
            done: false,
            seen: 0,
        };
        let mut b = Pager {
            conn: dst,
            r: tr,
            after: None,
            hi: None,
            done: false,
            seen: 0,
        };
        datadiff::merge(&mut a, &mut b, stop, on).await?;
        progress(DataProgress {
            rows: a.seen as f64,
            ranges_done: 0.0,
            ranges: 0.0,
        });
        return Ok((0, 0, 0));
    }
    if let (RawConn::My(c), RawConn::My(d)) = (src, dst) {
        c.query("SET SESSION group_concat_max_len = 1073741824", 0)
            .await?;
        d.query("SET SESSION group_concat_max_len = 1073741824", 0)
            .await?;
    }
    let bounds = boundaries(src, sr, stop).await?;
    let n = bounds.len() + 1;
    let (mut same_ranges, mut same_rows) = (0u64, 0u64);
    let mut rows = 0u64;
    for i in 0..n {
        let lo = if i == 0 {
            None
        } else {
            Some(bounds[i - 1].as_slice())
        };
        let hi = bounds.get(i).map(Vec::as_slice);
        let (ca, ha) = range_hash(src, sr, lo, hi).await?;
        let (cb, hb) = range_hash(dst, tr, lo, hi).await?;
        if ca == cb && ha == hb {
            same_ranges += 1;
            same_rows += ca;
        } else {
            let mut a = Pager {
                conn: src,
                r: sr,
                after: lo.map(<[_]>::to_vec),
                hi: hi.map(<[_]>::to_vec),
                done: false,
                seen: 0,
            };
            let mut b = Pager {
                conn: dst,
                r: tr,
                after: lo.map(<[_]>::to_vec),
                hi: hi.map(<[_]>::to_vec),
                done: false,
                seen: 0,
            };
            datadiff::merge(&mut a, &mut b, stop, on).await?;
        }
        rows += ca;
        progress(DataProgress {
            rows: rows as f64,
            ranges_done: (i + 1) as f64,
            ranges: n as f64,
        });
    }
    Ok((n as u64, same_ranges, same_rows))
}

fn key_text(key: &[Option<String>]) -> String {
    key.iter()
        .map(|k| k.as_deref().unwrap_or("NULL"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The compare itself: counts, and up to 100 samples of each kind.
pub async fn compare(
    src: &RawConn,
    sr: &Reader,
    dst: &RawConn,
    tr: &Reader,
    quick: bool,
    stop: &AtomicBool,
    progress: &(dyn Fn(DataProgress) + Sync),
) -> AppResult<DataCompareResult> {
    let started = std::time::Instant::now();
    let mut res = DataCompareResult {
        columns: sr.cols.iter().map(|c| c.name.clone()).collect(),
        key: sr.key.iter().map(|c| c.name.clone()).collect(),
        ..Default::default()
    };
    let (mut ns, mut nt, mut nd) = (0usize, 0usize, 0usize);
    let cols = res.columns.clone();
    let quick = quick && sr.family == tr.family;
    let (ranges, same, same_rows) = walk(src, sr, dst, tr, quick, stop, progress, &mut |e| {
        match e {
            Event::OnlySource(r) => {
                res.only_source += 1.0;
                if ns < SAMPLES {
                    ns += 1;
                    res.samples.push(DataSample {
                        kind: "only_source".into(),
                        key: key_text(&r.key),
                        changes: vec![],
                    });
                }
            }
            Event::OnlyTarget(r) => {
                res.only_target += 1.0;
                if nt < SAMPLES {
                    nt += 1;
                    res.samples.push(DataSample {
                        kind: "only_target".into(),
                        key: key_text(&r.key),
                        changes: vec![],
                    });
                }
            }
            Event::Different(a, b, changed) => {
                res.different += 1.0;
                if nd < SAMPLES {
                    nd += 1;
                    res.samples.push(DataSample {
                        kind: "different".into(),
                        key: key_text(&a.key),
                        changes: changed
                            .iter()
                            .map(|&i| DataChange {
                                column: cols[i].clone(),
                                source: a.values[i].as_ref().map(|v| v.chars().take(200).collect()),
                                target: b.values[i].as_ref().map(|v| v.chars().take(200).collect()),
                            })
                            .collect(),
                    });
                }
            }
            Event::Same => res.identical += 1.0,
        }
        Ok(())
    })
    .await?;
    res.identical += same_rows as f64;
    res.ranges = ranges as f64;
    res.ranges_same = same as f64;
    res.seconds = started.elapsed().as_secs_f64();
    Ok(res)
}

/// The statements that make one difference right in the target.
pub fn statement(tr: &Reader, e: &Event<'_>) -> Option<String> {
    let f = tr.family;
    let v = |x: &Option<String>| x.as_deref().map_or("NULL".into(), |s| lit_for(f, s));
    let q = |n: &str| tr.q(n);
    let where_key = |key: &[Option<String>]| {
        tr.key
            .iter()
            .zip(key)
            .map(|(c, k)| format!("{} = {}", q(&c.name), v(k)))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    match e {
        Event::OnlySource(r) => {
            let names: Vec<String> = tr.key.iter().chain(&tr.cols).map(|c| q(&c.name)).collect();
            let vals: Vec<String> = r.key.iter().chain(&r.values).map(v).collect();
            Some(format!(
                "INSERT INTO {} ({}) VALUES ({})",
                tr.table,
                names.join(", "),
                vals.join(", ")
            ))
        }
        Event::OnlyTarget(r) => Some(format!(
            "DELETE FROM {} WHERE {}",
            tr.table,
            where_key(&r.key)
        )),
        Event::Different(a, _, changed) => {
            let sets: Vec<String> = changed
                .iter()
                .map(|&i| format!("{} = {}", q(&tr.cols[i].name), v(&a.values[i])))
                .collect();
            Some(format!(
                "UPDATE {} SET {} WHERE {}",
                tr.table,
                sets.join(", "),
                where_key(&a.key)
            ))
        }
        Event::Same => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct MatchReport {
    pub inserted: f64,
    pub updated: f64,
    pub deleted: f64,
    pub seconds: f64,
}

/// Makes the target match: walks the differences again and applies them in one transaction (both engines' DML
/// is transactional), 200 statements at a time on PostgreSQL. Nothing changes if anything fails.
pub async fn make_match(
    src: &RawConn,
    sr: &Reader,
    dst: &RawConn,
    tr: &Reader,
    quick: bool,
    stop: &AtomicBool,
    progress: &(dyn Fn(DataProgress) + Sync),
) -> AppResult<MatchReport> {
    let started = std::time::Instant::now();
    let begin = match dst {
        RawConn::Pg(_) => "BEGIN",
        _ => "START TRANSACTION",
    };
    exec(dst, begin).await?;
    let mut rep = MatchReport::default();
    let mut batch: Vec<String> = Vec::new();
    let quick = quick && sr.family == tr.family;
    // Statements are collected per page of events and run between them (the merge callback can't await).
    let res = walk(src, sr, dst, tr, quick, stop, progress, &mut |e| {
        match &e {
            Event::OnlySource(_) => rep.inserted += 1.0,
            Event::OnlyTarget(_) => rep.deleted += 1.0,
            Event::Different(..) => rep.updated += 1.0,
            Event::Same => {}
        }
        if let Some(s) = statement(tr, &e) {
            batch.push(s);
        }
        Ok(())
    })
    .await;
    // ponytail: the statements of one walk are held until it ends (memory grows with the number of differences,
    // not the table); apply in pages from inside the merge if a table ever has millions of differences.
    let applied = match res {
        Ok(_) => run_all(dst, &batch).await,
        Err(e) => Err(e),
    };
    match applied {
        Ok(()) => {
            exec(dst, "COMMIT").await?;
            rep.seconds = started.elapsed().as_secs_f64();
            Ok(rep)
        }
        Err(e) => {
            let _ = exec(dst, "ROLLBACK").await;
            Err(e)
        }
    }
}

async fn exec(c: &RawConn, sql: &str) -> AppResult<()> {
    match c {
        RawConn::Pg(p) => p.client.batch_execute(sql).await.map_err(|e| map_error(&e)),
        RawConn::My(m) => m.query(sql, 0).await.map(|_| ()),
        RawConn::Mongo(_) => Err(AppError::validation("Data compare is for SQL tables.")),
    }
}

async fn run_all(c: &RawConn, stmts: &[String]) -> AppResult<()> {
    match c {
        RawConn::Pg(_) => {
            for chunk in stmts.chunks(200) {
                exec(c, &chunk.join(";\n")).await?;
            }
        }
        _ => {
            for s in stmts {
                exec(c, s).await?;
            }
        }
    }
    Ok(())
}

/// The same statements into a file, with a header, in one transaction.
pub async fn write_script(
    src: &RawConn,
    sr: &Reader,
    dst: &RawConn,
    tr: &Reader,
    quick: bool,
    out: &mut (dyn Write + Send),
    title: &str,
) -> AppResult<u64> {
    let stop = AtomicBool::new(false);
    let mut n = 0u64;
    let begin = if tr.family == Family::Pg {
        "BEGIN"
    } else {
        "START TRANSACTION"
    };
    let io = |e: std::io::Error| AppError::internal(format!("Couldn't write the script: {e}"));
    writeln!(out, "-- {title}\n-- Made by Dalan.\n\n{begin};\n").map_err(io)?;
    walk(
        src,
        sr,
        dst,
        tr,
        quick && sr.family == tr.family,
        &stop,
        &|_| {},
        &mut |e| {
            if let Some(s) = statement(tr, &e) {
                writeln!(out, "{s};").map_err(io)?;
                n += 1;
            }
            Ok(())
        },
    )
    .await?;
    writeln!(out, "\nCOMMIT;").map_err(io)?;
    Ok(n)
}

/// D16 against the Docker test servers: PostgreSQL with PostgreSQL through the quick pass (25,000 rows, three
/// ranges), made to match and compared again; and PostgreSQL with MySQL, where the same values written by each
/// server compare equal.
#[cfg(test)]
mod docker_tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::sql::copy::docker_tests::{my, pg, psql};

    fn side(schema: &str, table: &str) -> DataSide {
        DataSide {
            conn_id: String::new(),
            schema: schema.into(),
            table: table.into(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the Docker test servers"]
    async fn l3_tables_compare_by_key_on_one_engine_and_across_and_the_target_is_made_to_match() {
        psql("shop", "drop schema if exists d16d cascade; create schema d16d;
            create table d16d.a (id bigint primary key, name text, amount numeric(10,2), active boolean, created timestamptz, doc jsonb);
            insert into d16d.a select i, 'n' || i, i / 100.0, i % 2 = 0, timestamptz '2026-01-01 00:00:00+00' + i * interval '1 second', jsonb_build_object('tag', 'x', 'i', i) from generate_series(1, 25000) i;
            create table d16d.b (like d16d.a including all);
            insert into d16d.b select * from d16d.a;
            delete from d16d.b where id in (10, 24998, 24999);
            update d16d.b set name = 'changed' where id = 500;
            update d16d.b set amount = 0, doc = '{\"tag\": \"y\"}' where id = 20001;
            insert into d16d.b values (25001, 'extra', 1, true, now(), null), (25002, 'extra', 1, true, now(), null), (25003, 'extra', 1, true, now(), null), (0, 'zero', 0, false, now(), null);");
        let (src, dst) = (pg("shop").await, pg("shop").await);
        let (sr, tr, skipped) = readers(&src, &side("d16d", "a"), &dst, &side("d16d", "b"), &[])
            .await
            .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        let stop = AtomicBool::new(false);
        let r = compare(&src, &sr, &dst, &tr, true, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(
            (r.only_source, r.only_target, r.different),
            (3.0, 4.0, 2.0),
            "{r:#?}"
        );
        assert_eq!(r.identical, 25000.0 - 3.0 - 2.0);
        assert!(
            r.ranges >= 3.0 && r.ranges_same >= 1.0,
            "the quick pass skipped nothing: {r:#?}"
        );
        let d = r
            .samples
            .iter()
            .find(|s| s.kind == "different" && s.key == "20001")
            .unwrap();
        let cols: Vec<&str> = d.changes.iter().map(|c| c.column.as_str()).collect();
        assert_eq!(cols, vec!["amount", "doc"]);
        let full = compare(&src, &sr, &dst, &tr, false, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(
            (
                full.only_source,
                full.only_target,
                full.different,
                full.identical
            ),
            (r.only_source, r.only_target, r.different, r.identical)
        );
        let m = make_match(&src, &sr, &dst, &tr, true, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!((m.inserted, m.updated, m.deleted), (3.0, 2.0, 4.0));
        let again = compare(&src, &sr, &dst, &tr, true, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(
            (
                again.only_source,
                again.only_target,
                again.different,
                again.identical
            ),
            (0.0, 0.0, 0.0, 25000.0)
        );

        // Across engines: the same 300 rows as MySQL writes them, one amount changed.
        psql("shop", "create table d16d.c as select * from d16d.a where id <= 300; alter table d16d.c add primary key (id);");
        let mysql = my(13306, "test-mysql", None).await;
        let RawConn::My(c) = &mysql else {
            unreachable!()
        };
        for sql in [
            "drop database if exists d16d", "create database d16d",
            "create table d16d.c (id bigint primary key, name varchar(50), amount decimal(10,2), active tinyint(1), created datetime(6), doc json)",
            "set session cte_max_recursion_depth = 100000",
            "insert into d16d.c with recursive s(i) as (select 1 union all select i + 1 from s where i < 300) select i, concat('n', i), i / 100, i % 2 = 0, timestamp('2026-01-01 00:00:00') + interval i second, json_object('i', i, 'tag', 'x') from s",
            "update d16d.c set amount = 9.99 where id = 150",
        ] {
            c.query(sql, 0).await.unwrap_or_else(|e| panic!("{sql}: {}", e.message));
        }
        let (sr, tr, _) = readers(&src, &side("d16d", "c"), &mysql, &side("d16d", "c"), &[])
            .await
            .unwrap();
        let r = compare(&src, &sr, &mysql, &tr, true, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(
            (r.only_source, r.only_target, r.different, r.identical),
            (0.0, 0.0, 1.0, 299.0),
            "{r:#?}"
        );
        assert_eq!(r.samples[0].changes[0].column, "amount");
        assert!(
            r.warnings.is_empty() && r.ranges == 0.0,
            "no quick pass across engines"
        );
        let m = make_match(&src, &sr, &mysql, &tr, false, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(m.updated, 1.0);
        let again = compare(&src, &sr, &mysql, &tr, false, &stop, &|_| {})
            .await
            .unwrap();
        assert_eq!(again.identical, 300.0);
        c.query("drop database d16d", 0).await.unwrap();
        psql("shop", "drop schema d16d cascade;");
    }
}
