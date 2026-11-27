//! The backup check's restore (D17, E7): row counts of every table or collection, in the scratch database the
//! backup was restored into and in the source, side by side.

use bson::doc;
use serde::Serialize;
use specta::Type;

use super::mydb::{lit, q};
use super::pg::{map_error, quote};
use super::RawConn;
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Type)]
pub struct CountRow {
    pub name: String,
    /// `None`: not in the source (any more).
    pub source: Option<f64>,
    pub restored: f64,
    pub same: bool,
}

/// `name → rows` for every table (PostgreSQL: of every user schema, `schema.table`) or collection of `db`.
pub async fn counts(c: &RawConn, db: &str) -> AppResult<Vec<(String, u64)>> {
    let mut out = Vec::new();
    match c {
        RawConn::Pg(p) => {
            let names: Vec<(String, String)> = p
                .client
                .query(
                    "select schemaname::text, relname::text from pg_stat_user_tables order by 1, 2",
                    &[],
                )
                .await
                .map_err(|e| map_error(&e))?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect();
            for (s, t) in names {
                let n: i64 = p
                    .client
                    .query_one(
                        &format!("select count(*) from {}.{}", quote(&s), quote(&t)),
                        &[],
                    )
                    .await
                    .map_err(|e| map_error(&e))?
                    .get(0);
                out.push((format!("{s}.{t}"), n as u64));
            }
        }
        RawConn::My(m) => {
            let names = m
                .query(&format!("select table_name from information_schema.tables where table_schema = {} and table_type = 'BASE TABLE' order by 1", lit(db)), 0)
                .await?
                .rows;
            for t in names
                .into_iter()
                .filter_map(|r| r.into_iter().next().flatten())
            {
                let n = m
                    .query(&format!("select count(*) from {}.{}", q(db), q(&t)), 0)
                    .await?
                    .rows
                    .first()
                    .and_then(|r| r.first().cloned().flatten())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                out.push((t, n));
            }
        }
        RawConn::Mongo(m) => {
            let r = m.command(db, doc! { "listCollections": 1, "nameOnly": true, "filter": { "type": "collection" } }).await?;
            let mut names: Vec<String> = crate::mongo::docs::names(&r, "name")
                .iter()
                .filter_map(|d| d.get_str("name").ok().map(str::to_string))
                .filter(|n| !n.starts_with("system."))
                .collect();
            names.sort();
            for coll in names {
                let r = m.command(db, doc! { "count": &coll }).await?;
                let n = r
                    .get_i32("n")
                    .map(i64::from)
                    .or_else(|_| r.get_i64("n"))
                    .unwrap_or(0);
                out.push((coll, n as u64));
            }
        }
    }
    Ok(out)
}

/// Side by side: every restored table, with the source's count (when the source has it).
pub fn compare(restored: Vec<(String, u64)>, source: &[(String, u64)]) -> Vec<CountRow> {
    restored
        .into_iter()
        .map(|(name, n)| {
            let s = source.iter().find(|(x, _)| *x == name).map(|(_, c)| *c);
            CountRow {
                same: s == Some(n),
                source: s.map(|x| x as f64),
                restored: n as f64,
                name,
            }
        })
        .collect()
}
