//! MongoDB import (D17, E6; the gap D10 left): JSON lines, a JSON array, extended JSON (`{"$oid": …}`,
//! `{"$date": …}`), or CSV whose header may give each column a type (`age.int32()`, `born.date()`, like
//! mongoimport's `--columnsHaveTypes`), into a collection: insert, upsert by `_id` or a chosen field, or replace
//! what is there (typed). Documents go in batches of up to 1,000 (or 8 MB).

use bson::{doc, Bson, Document};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::conn::MongoConn;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MgFormat {
    JsonLines,
    JsonArray,
    Csv,
}

fn to_doc(v: serde_json::Value, what: &str) -> AppResult<Document> {
    match Bson::try_from(v) {
        Ok(Bson::Document(d)) => Ok(d),
        Ok(_) => Err(AppError::validation(format!("{what} isn't a document."))),
        Err(e) => Err(AppError::validation(format!("{what}: {e}"))),
    }
}

/// One CSV value by its header's type (`auto` reads numbers and true/false).
/// `date(layout)`: the layout isn't read; ISO dates and times are.
fn typed(kind: &str, _layout: &str, v: &str) -> AppResult<Bson> {
    let bad = || AppError::validation(format!("“{v}” isn't a {kind}."));
    Ok(match kind {
        "string" => Bson::String(v.into()),
        "int32" => Bson::Int32(v.trim().parse().map_err(|_| bad())?),
        "int64" | "long" => Bson::Int64(v.trim().parse().map_err(|_| bad())?),
        "double" => Bson::Double(v.trim().parse().map_err(|_| bad())?),
        "decimal" => Bson::Decimal128(v.trim().parse().map_err(|_| bad())?),
        "boolean" | "bool" => Bson::Boolean(matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "t" | "yes"
        )),
        "objectid" | "objectId" => {
            Bson::ObjectId(bson::oid::ObjectId::parse_str(v.trim()).map_err(|_| bad())?)
        }
        "date" | "date_ms" => {
            let t = v.trim();
            let ms = if let Ok(d) = chrono::DateTime::parse_from_rfc3339(t) {
                d.timestamp_millis()
            } else if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
                d.and_hms_opt(0, 0, 0)
                    .map(|x| x.and_utc().timestamp_millis())
                    .unwrap_or(0)
            } else if let Ok(d) = chrono::NaiveDateTime::parse_from_str(
                &t.replacen('T', " ", 1),
                "%Y-%m-%d %H:%M:%S%.f",
            ) {
                d.and_utc().timestamp_millis()
            } else {
                return Err(bad());
            };
            Bson::DateTime(bson::DateTime::from_millis(ms))
        }
        _ => {
            let t = v.trim();
            if let Ok(n) = t.parse::<i32>() {
                Bson::Int32(n)
            } else if let Ok(n) = t.parse::<i64>() {
                Bson::Int64(n)
            } else if let Ok(n) = t.parse::<f64>().map_err(|_| ()).and_then(|n| {
                if n.is_finite() {
                    Ok(n)
                } else {
                    Err(())
                }
            }) {
                Bson::Double(n)
            } else if t == "true" || t == "false" {
                Bson::Boolean(t == "true")
            } else {
                Bson::String(v.into())
            }
        }
    })
}

/// `name.type(arg)` → (name, type, arg); a plain name is `auto`.
fn header(h: &str) -> (String, String, String) {
    if let Some(open) = h.rfind('(') {
        if h.ends_with(')') {
            if let Some(dot) = h[..open].rfind('.') {
                return (
                    h[..dot].to_string(),
                    h[dot + 1..open].to_string(),
                    h[open + 1..h.len() - 1].to_string(),
                );
            }
        }
    }
    (h.to_string(), "auto".into(), String::new())
}

/// Reads a file's documents; the format is told by its first character (`[` array, `{` lines, else CSV).
pub fn parse(text: &str) -> AppResult<(MgFormat, Vec<Document>)> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('[') {
        let arr: Vec<serde_json::Value> = serde_json::from_str(t)
            .map_err(|e| AppError::validation(format!("That isn't a JSON array: {e}")))?;
        let docs = arr
            .into_iter()
            .enumerate()
            .map(|(i, v)| to_doc(v, &format!("Item {}", i + 1)))
            .collect::<AppResult<_>>()?;
        return Ok((MgFormat::JsonArray, docs));
    }
    if t.starts_with('{') {
        let docs = t
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| {
                let v: serde_json::Value = serde_json::from_str(l)
                    .map_err(|e| AppError::validation(format!("Line {} isn't JSON: {e}", i + 1)))?;
                to_doc(v, &format!("Line {}", i + 1))
            })
            .collect::<AppResult<_>>()?;
        return Ok((MgFormat::JsonLines, docs));
    }
    let rows = crate::sql::transfer::parse_csv(t, ',');
    let Some((head, body)) = rows.split_first() else {
        return Ok((MgFormat::Csv, vec![]));
    };
    let cols: Vec<(String, String, String)> = head.iter().map(|h| header(h.trim())).collect();
    let mut docs = Vec::with_capacity(body.len());
    for (n, r) in body.iter().enumerate() {
        let mut d = Document::new();
        for ((name, kind, arg), v) in cols.iter().zip(r) {
            // An empty value leaves the field out (mongoimport's --ignoreBlanks).
            if v.is_empty() {
                continue;
            }
            let value = typed(kind, arg, v).map_err(|e| {
                AppError::validation(format!("Row {}, {name}: {}", n + 2, e.message))
            })?;
            // `a.b` is a nested field.
            let mut parts: Vec<&str> = name.split('.').collect();
            let last = parts.pop().unwrap_or("");
            let mut at = &mut d;
            for p in parts {
                if !matches!(at.get(p), Some(Bson::Document(_))) {
                    at.insert(p, Document::new());
                }
                at = at
                    .get_document_mut(p)
                    .map_err(|e| AppError::internal(e.to_string()))?;
            }
            at.insert(last, value);
        }
        docs.push(d);
    }
    Ok((MgFormat::Csv, docs))
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MgImportMode {
    Insert,
    /// Replace the document whose `field` has the same value, or insert it.
    Upsert {
        field: String,
    },
    /// Remove every document of the collection first.
    Replace,
}

#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct MgImportReport {
    pub inserted: f64,
    pub upserted: f64,
    pub updated: f64,
    pub removed: f64,
    /// The server's words for documents it refused (the first 20).
    pub failed: Vec<String>,
    pub failed_count: f64,
}

fn errors(r: &Document, rep: &mut MgImportReport) {
    if let Ok(errs) = r.get_array("writeErrors") {
        for e in errs {
            rep.failed_count += 1.0;
            if rep.failed.len() < 20 {
                if let Some(d) = e.as_document() {
                    rep.failed
                        .push(d.get_str("errmsg").unwrap_or("refused").to_string());
                }
            }
        }
    }
}

fn n(r: &Document, k: &str) -> f64 {
    r.get_i32(k)
        .map(f64::from)
        .or_else(|_| r.get_i64(k).map(|x| x as f64))
        .unwrap_or(0.0)
}

/// Writes the documents. Batches stop at 1,000 documents or 8 MB; a refused document doesn't stop the others.
pub async fn write(
    c: &MongoConn,
    db: &str,
    coll: &str,
    docs: Vec<Document>,
    mode: &MgImportMode,
    progress: &(dyn Fn(f64) + Sync),
) -> AppResult<MgImportReport> {
    let mut rep = MgImportReport::default();
    if let MgImportMode::Upsert { field } = mode {
        if let Some(i) = docs.iter().position(|d| d.get(field).is_none()) {
            return Err(AppError::validation(format!(
                "Document {} has no {field}: every document needs it to upsert by it.",
                i + 1
            )));
        }
    }
    if matches!(mode, MgImportMode::Replace) {
        let r = c
            .command(
                db,
                doc! { "delete": coll, "deletes": [{ "q": {}, "limit": 0 }] },
            )
            .await?;
        rep.removed = n(&r, "n");
    }
    let mut batch: Vec<Document> = Vec::new();
    let mut size = 0usize;
    let mut done = 0f64;
    let total = docs.len();
    for (i, d) in docs.into_iter().enumerate() {
        size += d.to_vec().map(|v| v.len()).unwrap_or(0);
        batch.push(d);
        if batch.len() >= 1000 || size >= 8 << 20 || i + 1 == total {
            let b = std::mem::take(&mut batch);
            let count = b.len() as f64;
            size = 0;
            match mode {
                MgImportMode::Upsert { field } => {
                    let updates: Vec<Document> = b
                        .into_iter()
                        .map(|d| {
                            let key = d.get(field).cloned().unwrap_or(Bson::Null);
                            doc! { "q": { field.as_str(): key }, "u": d, "upsert": true }
                        })
                        .collect();
                    let r = c
                        .command_raw(
                            db,
                            doc! { "update": coll, "updates": updates, "ordered": false },
                        )
                        .await?;
                    let up = r
                        .get_array("upserted")
                        .map(|a| a.len() as f64)
                        .unwrap_or(0.0);
                    rep.upserted += up;
                    rep.updated += n(&r, "nModified");
                    errors(&r, &mut rep);
                }
                _ => {
                    let r = c
                        .command_raw(
                            db,
                            doc! { "insert": coll, "documents": b, "ordered": false },
                        )
                        .await?;
                    rep.inserted += n(&r, "n");
                    errors(&r, &mut rep);
                }
            }
            done += count;
            progress(done);
        }
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_reads_into_documents() {
        let (f, d) = parse("{\"_id\": {\"$oid\": \"5f1d7f3e9b1e8a3b2c4d5e6f\"}, \"at\": {\"$date\": \"2026-01-02T00:00:00Z\"}, \"n\": 1}\n{\"n\": 2}\n").unwrap();
        assert_eq!(f, MgFormat::JsonLines);
        assert!(matches!(d[0].get("_id"), Some(Bson::ObjectId(_))));
        assert!(matches!(d[0].get("at"), Some(Bson::DateTime(_))));
        let (f, d) =
            parse("[{\"a\": 1}, {\"a\": {\"$numberLong\": \"9007199254740993\"}}]").unwrap();
        assert_eq!(f, MgFormat::JsonArray);
        assert_eq!(d[1].get_i64("a").unwrap(), 9_007_199_254_740_993);
        let (f, d) = parse("name.string(),age.int32(),born.date(2006-01-02),score,addr.city,code.string()\nAda,36,1815-12-10,9.5,London,007\nBo,,,x,,\n").unwrap();
        assert_eq!(f, MgFormat::Csv);
        assert_eq!(d[0].get_i32("age").unwrap(), 36);
        assert!(matches!(d[0].get("born"), Some(Bson::DateTime(_))));
        assert_eq!(d[0].get_f64("score").unwrap(), 9.5);
        assert_eq!(
            d[0].get_document("addr").unwrap().get_str("city").unwrap(),
            "London"
        );
        assert_eq!(d[0].get_str("code").unwrap(), "007");
        assert!(
            d[1].get("age").is_none(),
            "an empty value leaves the field out"
        );
        assert_eq!(d[1].get_str("score").unwrap(), "x");
        assert!(parse("a.int32()\nnope\n")
            .unwrap_err()
            .message
            .contains("Row 2, a"));
        assert!(parse("[1, 2]").is_err());
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;

    /// D17 Docker: insert, upsert by a field, and replace, into a collection of the test MongoDB.
    #[tokio::test]
    #[ignore = "needs the Docker test servers"]
    async fn l3_documents_are_inserted_upserted_and_replaced() {
        let crate::sql::RawConn::Mongo(c) = crate::sql::copy::docker_tests::mongo().await else {
            unreachable!()
        };
        let _ = c.command("d17m", doc! { "dropDatabase": 1 }).await;
        let count = async |c: &MongoConn| {
            let r = c.command("d17m", doc! { "count": "items" }).await.unwrap();
            n(&r, "n")
        };
        let (_, docs) = parse("{\"sku\": \"a\", \"qty\": 1}\n{\"sku\": \"b\", \"qty\": 2}\n{\"_id\": 7, \"sku\": \"c\"}\n").unwrap();
        let r = write(&c, "d17m", "items", docs, &MgImportMode::Insert, &|_| {})
            .await
            .unwrap();
        assert_eq!(r.inserted, 3.0);
        // The same _id again: refused for that document only.
        let (_, again) = parse("{\"_id\": 7, \"sku\": \"dup\"}\n{\"sku\": \"d\"}\n").unwrap();
        let r = write(&c, "d17m", "items", again, &MgImportMode::Insert, &|_| {})
            .await
            .unwrap();
        assert_eq!((r.inserted, r.failed_count), (1.0, 1.0), "{r:?}");
        assert!(r.failed[0].contains("duplicate key"), "{:?}", r.failed);
        let (_, up) = parse("sku,qty.int32()\na,10\ne,5\n").unwrap();
        let r = write(
            &c,
            "d17m",
            "items",
            up,
            &MgImportMode::Upsert {
                field: "sku".into(),
            },
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!((r.updated, r.upserted), (1.0, 1.0), "{r:?}");
        assert_eq!(count(&c).await, 5.0);
        let (_, no_sku) = parse("{\"qty\": 1}\n").unwrap();
        assert!(write(
            &c,
            "d17m",
            "items",
            no_sku,
            &MgImportMode::Upsert {
                field: "sku".into()
            },
            &|_| {}
        )
        .await
        .is_err());
        let (_, fresh) = parse("[{\"sku\": \"z\"}]").unwrap();
        let r = write(&c, "d17m", "items", fresh, &MgImportMode::Replace, &|_| {})
            .await
            .unwrap();
        assert_eq!((r.removed, r.inserted), (5.0, 1.0));
        assert_eq!(count(&c).await, 1.0);
        let _ = c.command("d17m", doc! { "dropDatabase": 1 }).await;
    }
}
