//! Schema, indexes and validation of one collection (S46; F29): a sample of documents summarised by field and type,
//! the indexes with how much they are used, and the validation rules with a check of how many documents break them.
//! Changes (create or drop an index, change the rules) are built here and held by the gate.

use std::collections::BTreeMap;

use bson::{doc, Bson, Document};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::conn::MongoConn;
use super::docs::{canonical, mg_doc, names, parse_doc, MgDoc, Parsed};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::sql::classify::Class;

const MAX_FIELDS: usize = 400;
const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgType {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgField {
    /// `a.b` for a nested field, `tags[]` for the items of an array.
    pub path: String,
    /// In how many of the sampled documents the field is present.
    pub present: u32,
    pub types: Vec<MgType>,
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgSchema {
    pub sampled: u32,
    pub fields: Vec<MgField>,
    /// The sample was cut (more fields than are listed).
    pub truncated: bool,
}

pub fn type_name(b: &Bson) -> &'static str {
    match b {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => "javascript",
        Bson::Int32(_) => "int",
        Bson::Int64(_) => "long",
        Bson::Timestamp(_) => "timestamp",
        Bson::Binary(_) => "binData",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::Symbol(_) => "symbol",
        Bson::Decimal128(_) => "decimal",
        Bson::Undefined => "undefined",
        Bson::MaxKey => "maxKey",
        Bson::MinKey => "minKey",
        Bson::DbPointer(_) => "dbPointer",
    }
}

struct Acc {
    present: u32,
    types: BTreeMap<&'static str, u32>,
    example: Option<String>,
}

/// Walks one document: every field once per document (an array's items count once under `path[]`).
fn walk(prefix: &str, v: &Bson, depth: usize, seen: &mut std::collections::HashSet<String>, acc: &mut BTreeMap<String, Acc>) {
    let e = acc.entry(prefix.to_string()).or_insert(Acc { present: 0, types: BTreeMap::new(), example: None });
    if seen.insert(prefix.to_string()) {
        e.present += 1;
    }
    *e.types.entry(type_name(v)).or_insert(0) += 1;
    if e.example.is_none() && !matches!(v, Bson::Document(_) | Bson::Array(_)) {
        let t = v.clone().into_relaxed_extjson().to_string();
        e.example = Some(if t.chars().count() > 80 { format!("{}…", t.chars().take(80).collect::<String>()) } else { t });
    }
    if depth >= MAX_DEPTH {
        return;
    }
    match v {
        Bson::Document(d) => {
            for (k, x) in d {
                walk(&format!("{prefix}.{k}"), x, depth + 1, seen, acc);
            }
        }
        Bson::Array(a) => {
            for x in a {
                walk(&format!("{prefix}[]"), x, depth + 1, seen, acc);
            }
        }
        _ => {}
    }
}

/// Summarises sampled documents by field and type.
pub fn summarise(docs: &[Document]) -> MgSchema {
    let mut acc = BTreeMap::new();
    for d in docs {
        let mut seen = std::collections::HashSet::new();
        for (k, v) in d {
            walk(k, v, 1, &mut seen, &mut acc);
        }
    }
    let truncated = acc.len() > MAX_FIELDS;
    let mut fields: Vec<MgField> = acc
        .into_iter()
        .take(MAX_FIELDS)
        .map(|(path, a)| {
            let mut types: Vec<MgType> = a.types.into_iter().map(|(n, c)| MgType { name: n.into(), count: c }).collect();
            types.sort_by(|x, y| y.count.cmp(&x.count).then(x.name.cmp(&y.name)));
            MgField { path, present: a.present, types, example: a.example }
        })
        .collect();
    // `_id` first, then by how common the field is, then by name.
    fields.sort_by(|a, b| (b.path == "_id").cmp(&(a.path == "_id")).then(b.present.cmp(&a.present)).then(a.path.cmp(&b.path)));
    MgSchema { sampled: docs.len() as u32, fields, truncated }
}

pub async fn sample(c: &MongoConn, db: &str, coll: &str, size: u32) -> AppResult<MgSchema> {
    let size = size.clamp(1, 1000);
    let r = c.command(db, doc! { "aggregate": coll, "pipeline": [{ "$sample": { "size": i64::from(size) } }], "cursor": { "batchSize": i64::from(size) } }).await?;
    let cur = r.get_document("cursor").map_err(|_| AppError::new(ErrorCode::DbRefused, "MongoDB's answer to aggregate had no cursor."))?;
    let mut docs = names(cur, "firstBatch");
    // A large sample can come in several batches.
    let mut id = match cur.get("id") {
        Some(Bson::Int64(i)) => *i,
        _ => 0,
    };
    while id != 0 && docs.len() < size as usize {
        let r = c.command(db, doc! { "getMore": id, "collection": coll }).await?;
        let cur = r.get_document("cursor").map_err(|_| AppError::new(ErrorCode::DbRefused, "MongoDB's answer to getMore had no cursor."))?;
        docs.extend(names(cur, "nextBatch"));
        id = match cur.get("id") {
            Some(Bson::Int64(i)) => *i,
            _ => 0,
        };
    }
    if id != 0 {
        let _ = c.command(db, doc! { "killCursors": coll, "cursors": [id] }).await;
    }
    Ok(summarise(&docs))
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgIndex {
    pub name: String,
    /// The key pattern as extended JSON, in order: `{"status":1,"placed":-1}`.
    pub keys: String,
    pub unique: bool,
    pub sparse: bool,
    pub ttl_seconds: Option<f64>,
    pub partial: Option<String>,
    /// How many times a query used it since the server (re)started or the index was made; `None` if unknown.
    pub uses: Option<f64>,
    /// Since when `uses` counts (UTC milliseconds).
    pub since_ms: Option<f64>,
}

fn num(b: Option<&Bson>) -> Option<f64> {
    match b {
        Some(Bson::Int32(i)) => Some(f64::from(*i)),
        Some(Bson::Int64(i)) => Some(*i as f64),
        Some(Bson::Double(d)) => Some(*d),
        _ => None,
    }
}

pub async fn indexes(c: &MongoConn, db: &str, coll: &str) -> AppResult<Vec<MgIndex>> {
    let r = c.command(db, doc! { "listIndexes": coll }).await?;
    let list = r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default();
    // `$indexStats` is best effort: views and some roles can't ask for it.
    let stats = c.command(db, doc! { "aggregate": coll, "pipeline": [{ "$indexStats": {} }], "cursor": {} }).await.ok();
    let stats: Vec<Document> = stats.and_then(|s| s.get_document("cursor").map(|c| names(c, "firstBatch")).ok()).unwrap_or_default();
    Ok(list
        .iter()
        .map(|i| {
            let name = i.get_str("name").unwrap_or("?").to_string();
            let st = stats.iter().find(|s| s.get_str("name").is_ok_and(|n| n == name));
            let acc = st.and_then(|s| s.get_document("accesses").ok());
            MgIndex {
                keys: i.get_document("key").map(|k| Bson::Document(k.clone()).into_relaxed_extjson().to_string()).unwrap_or_default(),
                unique: i.get_bool("unique").unwrap_or(false),
                sparse: i.get_bool("sparse").unwrap_or(false),
                ttl_seconds: num(i.get("expireAfterSeconds")),
                partial: i.get_document("partialFilterExpression").ok().map(|p| Bson::Document(p.clone()).into_relaxed_extjson().to_string()),
                uses: acc.and_then(|a| num(a.get("ops"))),
                since_ms: acc.and_then(|a| a.get_datetime("since").ok()).map(|d| d.timestamp_millis() as f64),
                name,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MgIndexOp {
    Create { keys: String, name: String, unique: bool, sparse: bool, ttl_seconds: Option<u32>, partial: String },
    Drop { name: String },
}

/// An index change as a held command: creating is a schema change (confirm), dropping is dangerous (typed name).
pub fn index_command(coll: &str, op: &MgIndexOp) -> AppResult<Parsed> {
    match op {
        MgIndexOp::Create { keys, name, unique, sparse, ttl_seconds, partial } => {
            let k = parse_doc(keys, "The index keys")?;
            if k.is_empty() {
                return Err(AppError::validation("An index needs at least one key, like { \"status\": 1 }."));
            }
            let mut spec = doc! { "key": k.clone() };
            let label = if name.trim().is_empty() { k.iter().map(|(f, d)| format!("{f}_{}", match d { Bson::Int32(i) => i.to_string(), Bson::Int64(i) => i.to_string(), Bson::Double(x) => format!("{}", *x as i64), Bson::String(s) => s.clone(), _ => "x".into() })).collect::<Vec<_>>().join("_") } else { name.trim().to_string() };
            spec.insert("name", label.clone());
            if *unique {
                spec.insert("unique", true);
            }
            if *sparse {
                spec.insert("sparse", true);
            }
            if let Some(t) = ttl_seconds {
                spec.insert("expireAfterSeconds", i64::from(*t));
            }
            let p = parse_doc(partial, "The partial filter")?;
            if !p.is_empty() {
                spec.insert("partialFilterExpression", p);
            }
            Ok(Parsed::Raw { class: Class::Schema, summary: format!("Create the index {label} on {coll}{}.", if *unique { " (unique: duplicates will be refused)" } else { "" }), cmd: doc! { "createIndexes": coll, "indexes": [spec] } })
        }
        MgIndexOp::Drop { name } => {
            if name == "_id_" {
                return Err(AppError::validation("The _id index can't be dropped."));
            }
            Ok(Parsed::Raw { class: Class::Dangerous, summary: format!("Drop the index {name} on {coll}. Queries that used it get slower."), cmd: doc! { "dropIndexes": coll, "index": name.clone() } })
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgValidation {
    /// The rules as canonical extended JSON; empty when there are none.
    pub validator: String,
    pub level: String,
    pub action: String,
}

pub async fn validation(c: &MongoConn, db: &str, coll: &str) -> AppResult<MgValidation> {
    let r = c.command(db, doc! { "listCollections": 1, "filter": { "name": coll } }).await?;
    let first = r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default().into_iter().next();
    let opts = first.as_ref().and_then(|d| d.get_document("options").ok());
    Ok(MgValidation {
        validator: opts.and_then(|o| o.get_document("validator").ok()).map(canonical).unwrap_or_default(),
        level: opts.and_then(|o| o.get_str("validationLevel").ok()).unwrap_or("strict").to_string(),
        action: opts.and_then(|o| o.get_str("validationAction").ok()).unwrap_or("error").to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgValidationCheck {
    /// How many documents the rules would reject.
    pub failing: f64,
    pub sample: Vec<MgDoc>,
}

/// How many existing documents break these rules (the rules are a query, so `$nor` of them finds the rest).
pub async fn check(c: &MongoConn, db: &str, coll: &str, validator: &str) -> AppResult<MgValidationCheck> {
    let v = parse_doc(validator, "The rules")?;
    if v.is_empty() {
        return Ok(MgValidationCheck { failing: 0.0, sample: vec![] });
    }
    let r = c.command(db, doc! { "aggregate": coll, "pipeline": [{ "$match": { "$nor": [v.clone()] } }, { "$count": "n" }], "cursor": {} }).await?;
    let n = r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default().first().and_then(|d| num(d.get("n"))).unwrap_or(0.0);
    let r = c.command(db, doc! { "find": coll, "filter": { "$nor": [v] }, "limit": 3_i64, "singleBatch": true }).await?;
    let sample = r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default().iter().map(mg_doc).collect();
    Ok(MgValidationCheck { failing: n, sample })
}

/// `collMod` with new rules: a schema change (confirm).
pub fn validation_command(coll: &str, validator: &str, level: &str, action: &str) -> AppResult<Parsed> {
    if !["off", "moderate", "strict"].contains(&level) || !["warn", "error"].contains(&action) {
        return Err(AppError::validation("Level is off, moderate or strict; action is warn or error."));
    }
    let v = parse_doc(validator, "The rules")?;
    Ok(Parsed::Raw {
        class: Class::Schema,
        summary: if v.is_empty() { format!("Remove the validation rules of {coll}.") } else { format!("Change the validation rules of {coll} (level {level}, action {action}). Existing documents are not touched.") },
        cmd: doc! { "collMod": coll, "validator": v, "validationLevel": level, "validationAction": action },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_is_summarised_by_field_and_type() {
        let docs = vec![
            doc! { "_id": 1, "name": "a", "tags": ["x", "y"], "addr": { "city": "Oslo", "zip": 123 }, "n": 1 },
            doc! { "_id": 2, "name": "b", "addr": { "city": "Rome" }, "n": 2.5 },
            doc! { "_id": 3, "name": Bson::Null, "n": 3_i64 },
        ];
        let s = summarise(&docs);
        assert_eq!(s.sampled, 3);
        assert_eq!(s.fields[0].path, "_id");
        let f = |p: &str| s.fields.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p}"));
        assert_eq!(f("name").present, 3);
        assert_eq!(f("name").types.iter().map(|t| (t.name.as_str(), t.count)).collect::<Vec<_>>(), [("string", 2), ("null", 1)]);
        assert_eq!(f("n").types.len(), 3);
        assert_eq!(f("addr.city").present, 2);
        assert_eq!(f("tags[]").present, 1);
        assert_eq!(f("tags[]").types[0].count, 2);
        assert_eq!(f("addr.zip").example.as_deref(), Some("123"));
    }

    #[test]
    fn index_and_validation_changes_are_classed_for_the_gate() {
        let create = MgIndexOp::Create { keys: r#"{"status":1,"placed":-1}"#.into(), name: String::new(), unique: true, sparse: false, ttl_seconds: None, partial: String::new() };
        let Parsed::Raw { class, summary, cmd } = index_command("orders", &create).unwrap() else { panic!() };
        assert_eq!(class, Class::Schema);
        assert!(summary.contains("status_1_placed_-1") && summary.contains("unique"));
        assert!(cmd.get_array("indexes").is_ok());
        assert!(index_command("o", &MgIndexOp::Create { keys: "{}".into(), name: String::new(), unique: false, sparse: false, ttl_seconds: None, partial: String::new() }).is_err());
        assert!(matches!(index_command("o", &MgIndexOp::Drop { name: "a_1".into() }).unwrap(), Parsed::Raw { class: Class::Dangerous, .. }));
        assert!(index_command("o", &MgIndexOp::Drop { name: "_id_".into() }).is_err());
        assert!(matches!(validation_command("o", r#"{"qty":{"$gt":0}}"#, "strict", "error").unwrap(), Parsed::Raw { class: Class::Schema, .. }));
        assert!(validation_command("o", "{}", "loose", "error").is_err());
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target, TlsMode};
    use crate::dbs::Engine;
    use crate::mongo::aggregate::{self, MgAggRequest, MgStage};
    use crate::mongo::conn::MongoLogin;
    use crate::mongo::docs::{run, MgWrite};
    use std::sync::Arc;
    use zeroize::Zeroizing;

    async fn conn() -> MongoConn {
        let reopen: crate::sql::pg::Reopen = Arc::new(|| Box::pin(async { transport::direct_for(Engine::Mongodb, &Target { address: "127.0.0.1".into(), port: 17017, tls: false, tls_server_name: None }).await }));
        let stream = (reopen)().await.unwrap();
        let login = MongoLogin { user: "app".into(), password: Some(Zeroizing::new("test-mongo".into())), auth_db: "admin".into(), tls: TlsMode::Off, server_name: "127.0.0.1".into() };
        MongoConn::connect(stream, login, reopen).await.unwrap().0
    }
    fn st(op: &str, body: &str) -> MgStage {
        MgStage { op: op.into(), body: body.into(), enabled: true }
    }
    async fn go(c: &MongoConn, db: &str, coll: &str, p: Parsed) -> AppResult<u32> {
        run(c, &MgWrite { db: db.into(), coll: coll.into(), op: p }).await
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_aggregate_schema_indexes_validation_and_views() {
        let c = conn().await;
        for n in ["sales", "sales_totals", "open_sales"] {
            let _ = c.command("d9", doc! { "drop": n }).await;
        }
        let rows: Vec<Document> = (0..300).map(|i| doc! { "_id": i, "status": if i % 3 == 0 { "open" } else { "paid" }, "qty": i % 7, "tags": ["a", "b"], "note": if i % 5 == 0 { Bson::Null } else { Bson::String(format!("n{i}")) } }).collect();
        c.command("d9", doc! { "insert": "sales", "documents": rows }).await.unwrap();

        // Output after each stage; a disabled stage is skipped; $out stops the preview.
        let mut stages = vec![st("$match", r#"{"status":"open"}"#), st("$group", r#"{"_id":"$qty","n":{"$sum":1}}"#), st("$sort", r#"{"_id":1}"#), st("$out", r#""sales_totals""#)];
        let req = |stages: Vec<MgStage>, upto| MgAggRequest { db: "d9".into(), coll: "sales".into(), stages, upto, sample: 5, hint: String::new() };
        let r0 = aggregate::run(&c, &req(stages.clone(), Some(0))).await.unwrap();
        assert_eq!((r0.docs.len(), r0.more), (5, true));
        let all = aggregate::run(&c, &req(stages.clone(), None)).await.unwrap();
        assert!(all.stopped_at_write && all.docs.len() == 5 && all.more);
        stages[0].enabled = false;
        let r2 = aggregate::run(&c, &req(stages.clone(), Some(2))).await.unwrap();
        assert_eq!(r2.docs.len(), 5);
        assert!(r2.docs[0].relaxed.contains("\"_id\": 0"), "{}", r2.docs[0].relaxed);
        stages[0].enabled = true;
        // A bad stage is the server's error, in its words.
        assert!(aggregate::run(&c, &req(vec![st("$bogus", "{}")], None)).await.unwrap_err().message.contains("$bogus"));
        // $out only runs through the held command.
        assert!(c.command("d9", doc! { "count": "sales_totals" }).await.map(|r| r.get_i32("n").unwrap_or(0)).unwrap_or(0) == 0);
        let w = aggregate::write_command(&req(stages.clone(), None)).unwrap();
        go(&c, "d9", "sales", w).await.unwrap();
        assert_eq!(c.command("d9", doc! { "count": "sales_totals" }).await.unwrap().get_i32("n").unwrap(), 7);
        // A view.
        let v = aggregate::view_command("d9", "sales", "open_sales", &stages[..1]).unwrap();
        go(&c, "d9", "sales", v).await.unwrap();
        assert_eq!(c.command("d9", doc! { "count": "open_sales" }).await.unwrap().get_i32("n").unwrap(), 100);

        // Schema sample.
        let s = sample(&c, "d9", "sales", 1000).await.unwrap();
        assert_eq!(s.sampled, 300);
        let note = s.fields.iter().find(|f| f.path == "note").unwrap();
        assert_eq!(note.types.iter().map(|t| (t.name.as_str(), t.count)).collect::<Vec<_>>(), [("string", 240), ("null", 60)]);
        assert!(s.fields.iter().any(|f| f.path == "tags[]"));

        // Indexes: create (unique + partial), see it, use it (stats), drop it.
        let create = MgIndexOp::Create { keys: r#"{"status":1,"qty":-1}"#.into(), name: String::new(), unique: false, sparse: false, ttl_seconds: Some(3600), partial: r#"{"qty":{"$gt":1}}"#.into() };
        // TTL needs a single date key: the server says so in its words.
        assert!(go(&c, "d9", "sales", index_command("sales", &create).unwrap()).await.is_err());
        let create = MgIndexOp::Create { keys: r#"{"status":1,"qty":-1}"#.into(), name: String::new(), unique: false, sparse: false, ttl_seconds: None, partial: r#"{"qty":{"$gt":1}}"#.into() };
        go(&c, "d9", "sales", index_command("sales", &create).unwrap()).await.unwrap();
        let ix = indexes(&c, "d9", "sales").await.unwrap();
        let mine = ix.iter().find(|i| i.name == "status_1_qty_-1").expect("index listed");
        assert!(mine.partial.is_some() && mine.uses.is_some());
        let mut hinted = req(vec![st("$match", r#"{"status":"open","qty":{"$gt":3}}"#)], None);
        hinted.hint = "status_1_qty_-1".into();
        assert!(!aggregate::run(&c, &hinted).await.unwrap().docs.is_empty());
        hinted.hint = "nope_1".into();
        assert!(aggregate::run(&c, &hinted).await.is_err());
        go(&c, "d9", "sales", index_command("sales", &MgIndexOp::Drop { name: "status_1_qty_-1".into() }).unwrap()).await.unwrap();
        assert!(indexes(&c, "d9", "sales").await.unwrap().iter().all(|i| i.name != "status_1_qty_-1"));

        // Validation: none to start with, a check counts what breaks, collMod sets it.
        assert_eq!(validation(&c, "d9", "sales").await.unwrap().validator, "");
        let rules = r#"{"qty":{"$lt":5}}"#;
        let chk = check(&c, "d9", "sales", rules).await.unwrap();
        assert_eq!(chk.failing, 85.0);
        assert_eq!(chk.sample.len(), 3);
        go(&c, "d9", "sales", validation_command("sales", rules, "strict", "error").unwrap()).await.unwrap();
        let v = validation(&c, "d9", "sales").await.unwrap();
        assert!(v.validator.contains("$lt") && v.level == "strict" && v.action == "error");
        // New documents are checked: the server's rejection comes back as an error.
        let bad = c.command("d9", doc! { "insert": "sales", "documents": [{ "_id": 9999, "qty": 6 }] }).await.unwrap_err();
        assert!(bad.message.to_lowercase().contains("validation"), "{}", bad.message);
        go(&c, "d9", "sales", validation_command("sales", "{}", "strict", "error").unwrap()).await.unwrap();
        assert_eq!(validation(&c, "d9", "sales").await.unwrap().validator, "");
        for n in ["sales", "sales_totals", "open_sales"] {
            let _ = c.command("d9", doc! { "drop": n }).await;
        }
    }
}
