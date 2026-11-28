//! MongoDB documents (S44; F27): the tree, `find` with its cursor, counts, and exact extended-JSON conversion both ways.
//! Documents travel to the window as text in two forms: relaxed extended JSON for the views (readable) and canonical
//! for the editor (every type visible, so a save gives back exactly what was shown).

use bson::{doc, Bson, Document};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::conn::MongoConn;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::sql::classify::Class;

/// Most documents one page returns.
pub const MAX_LIMIT: u32 = 500;
/// A document whose JSON is longer than this arrives cut in the views (the editor asks for it whole).
const CUT_AT: usize = 65_536;

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgNode {
    pub name: String,
    /// `database`, `collection` or `view`.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct MgFindRequest {
    pub db: String,
    pub coll: String,
    pub filter: String,
    pub project: String,
    pub sort: String,
    pub skip: u32,
    pub limit: u32,
    /// A collation document, e.g. `{"locale":"en","strength":2}`; empty for none.
    pub collation: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgDoc {
    /// The `_id` in canonical extended JSON: how an edit or delete finds this document again.
    pub id: String,
    /// Relaxed extended JSON for the views (cut when huge: then it is not valid JSON, and `cut` is set).
    pub relaxed: String,
    pub cut: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgPage {
    pub docs: Vec<MgDoc>,
    /// There may be more after this page (the cursor still had documents).
    pub more: bool,
    pub took_ms: u32,
}

fn bad(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::validation(format!("{what} isn't valid JSON: {e}"))
}

/// Extended JSON text to a document. Empty text is the empty document. Both canonical (`{"$numberLong":"5"}`) and
/// relaxed (`{"$oid":"…"}`, plain numbers) forms are read.
pub fn parse_doc(text: &str, what: &str) -> AppResult<Document> {
    if text.trim().is_empty() {
        return Ok(Document::new());
    }
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| bad(what, e))?;
    match Bson::try_from(v).map_err(|e| bad(what, e))? {
        Bson::Document(d) => Ok(d),
        _ => Err(AppError::validation(format!(
            "{what} must be a JSON object, like {{ \"status\": \"open\" }}."
        ))),
    }
}

/// Extended JSON of any shape (a stage's argument is often a number or a string, not an object).
pub fn parse_value(text: &str, what: &str) -> AppResult<Bson> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| bad(what, e))?;
    Bson::try_from(v).map_err(|e| bad(what, e))
}

pub fn relaxed(d: &Document) -> String {
    serde_json::to_string_pretty(&Bson::Document(d.clone()).into_relaxed_extjson())
        .unwrap_or_default()
}
pub fn canonical(d: &Document) -> String {
    serde_json::to_string_pretty(&Bson::Document(d.clone()).into_canonical_extjson())
        .unwrap_or_default()
}
/// The `_id` as one line of canonical JSON (any type: ObjectId, string, number, a compound document).
pub fn id_text(d: &Document) -> String {
    d.get("_id")
        .map(|b| b.clone().into_canonical_extjson().to_string())
        .unwrap_or_else(|| "null".into())
}
/// The inverse: an `_id` text from the window back to a value.
pub fn parse_id(text: &str) -> AppResult<Bson> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| bad("The _id", e))?;
    Bson::try_from(v).map_err(|e| bad("The _id", e))
}

pub fn mg_doc(d: &Document) -> MgDoc {
    let mut text = relaxed(d);
    let cut = text.len() > CUT_AT;
    if cut {
        let mut at = CUT_AT;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        text.truncate(at);
    }
    MgDoc {
        id: id_text(d),
        relaxed: text,
        cut,
    }
}

/// Streams the documents of a collection that match `filter` to `out`, one compact canonical extended JSON document
/// per line (what `mongoimport` reads), and returns how many were written. Stops (with `Ok`) when `stop` is set.
/// `masks` (E5) are applied to the named top-level fields before anything is written, with a new key per export.
#[allow(clippy::too_many_arguments)]
pub async fn export(
    c: &MongoConn,
    db: &str,
    coll: &str,
    filter: &str,
    masks: &[crate::dbs::masking::MaskRule],
    out: &mut (dyn std::io::Write + Send),
    stop: &std::sync::atomic::AtomicBool,
    progress: &(dyn Fn(f64, f64) + Sync),
) -> AppResult<f64> {
    use std::sync::atomic::Ordering;
    let f = parse_doc(filter, "The filter")?;
    let io = |e: std::io::Error| AppError::internal(format!("File error: {e}"));
    let mut r = c
        .command(db, doc! { "find": coll, "filter": f, "batchSize": 1000 })
        .await?;
    let (mut rows, mut bytes) = (0f64, 0f64);
    let fields: Vec<String> = masks.iter().map(|m| m.column.clone()).collect();
    let mask = crate::dbs::masking::Masker::new(crate::dbs::masking::new_key(), masks, &fields);
    loop {
        let cur = r.get_document("cursor").map_err(|_| {
            AppError::new(
                ErrorCode::DbRefused,
                "MongoDB's answer to find had no cursor.",
            )
        })?;
        let batch = if cur.contains_key("firstBatch") {
            "firstBatch"
        } else {
            "nextBatch"
        };
        let mut docs = names(cur, batch);
        if let Some(m) = &mask {
            m.documents(&mut docs);
        }
        for d in docs {
            let line = Bson::Document(d).into_canonical_extjson().to_string();
            out.write_all(line.as_bytes()).map_err(io)?;
            out.write_all(b"\n").map_err(io)?;
            rows += 1.0;
            bytes += line.len() as f64 + 1.0;
        }
        progress(rows, bytes);
        let id = match cur.get("id") {
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => i64::from(*i),
            _ => 0,
        };
        if id == 0 {
            return Ok(rows);
        }
        if stop.load(Ordering::SeqCst) {
            let _ = c
                .command(db, doc! { "killCursors": coll, "cursors": [id] })
                .await;
            return Err(AppError::new(ErrorCode::Internal, "Stopped."));
        }
        r = c
            .command(
                db,
                doc! { "getMore": id, "collection": coll, "batchSize": 1000 },
            )
            .await?;
    }
}

/// One whole document by `_id`, as canonical extended JSON: what the editor opens (never a projected copy).
pub async fn get(c: &MongoConn, db: &str, coll: &str, id: &str) -> AppResult<String> {
    let id = parse_id(id)?;
    let r = c
        .command(
            db,
            doc! { "find": coll, "filter": { "_id": id }, "limit": 1_i64, "singleBatch": true },
        )
        .await?;
    match r
        .get_document("cursor")
        .map(|c| names(c, "firstBatch"))
        .unwrap_or_default()
        .into_iter()
        .next()
    {
        Some(d) => Ok(canonical(&d)),
        None => Err(AppError::new(
            ErrorCode::DbRefused,
            "That document no longer exists.",
        )),
    }
}

pub fn names(reply: &Document, key: &str) -> Vec<Document> {
    reply
        .get_array(key)
        .map(|a| a.iter().filter_map(|b| b.as_document().cloned()).collect())
        .unwrap_or_default()
}

/// `[]` lists databases, `[db]` its collections and views.
pub async fn tree(c: &MongoConn, path: &[String]) -> AppResult<Vec<MgNode>> {
    match path {
        [] => {
            let r = c
                .command("admin", doc! { "listDatabases": 1, "nameOnly": true })
                .await?;
            let mut v: Vec<MgNode> = names(&r, "databases")
                .iter()
                .filter_map(|d| d.get_str("name").ok())
                .map(|n| MgNode {
                    name: n.into(),
                    kind: "database".into(),
                })
                .collect();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(v)
        }
        [db] => {
            let r = c
                .command(db, doc! { "listCollections": 1, "nameOnly": true })
                .await?;
            let mut v: Vec<MgNode> = vec![];
            let first = r
                .get_document("cursor")
                .map(|c| names(c, "firstBatch"))
                .unwrap_or_default();
            for d in first {
                let (Ok(n), kind) = (d.get_str("name"), d.get_str("type").unwrap_or("collection"))
                else {
                    continue;
                };
                // Hidden system collections stay out of the tree.
                if n.starts_with("system.") {
                    continue;
                }
                v.push(MgNode {
                    name: n.into(),
                    kind: if kind == "view" {
                        "view".into()
                    } else {
                        "collection".into()
                    },
                });
            }
            v.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(v)
        }
        _ => Ok(vec![]),
    }
}

fn find_command(req: &MgFindRequest, limit: u32) -> AppResult<Document> {
    let mut cmd = doc! { "find": &req.coll, "filter": parse_doc(&req.filter, "The filter")?, "skip": i64::from(req.skip), "limit": i64::from(limit) };
    let (project, sort, collation) = (
        parse_doc(&req.project, "The projection")?,
        parse_doc(&req.sort, "The sort")?,
        parse_doc(&req.collation, "The collation")?,
    );
    if !project.is_empty() {
        cmd.insert("projection", project);
    }
    if !sort.is_empty() {
        cmd.insert("sort", sort);
    }
    if !collation.is_empty() {
        cmd.insert("collation", collation);
    }
    Ok(cmd)
}

/// One page: `find` with the filter, projection, sort, skip, limit and collation, then `getMore` until the limit is
/// reached or the cursor ends; a cursor left open is killed.
pub async fn find(c: &MongoConn, req: &MgFindRequest) -> AppResult<MgPage> {
    let started = std::time::Instant::now();
    let limit = req.limit.clamp(1, MAX_LIMIT);
    // One more than asked, to know whether a next page exists.
    let want = limit as usize + 1;
    let mut cmd = find_command(req, limit + 1)?;
    cmd.insert("batchSize", i64::from((limit + 1).min(101)));
    let mut r = c.command(&req.db, cmd).await?;
    let mut docs: Vec<Document> = vec![];
    let mut cursor_id;
    loop {
        let cur = r.get_document("cursor").map_err(|_| {
            AppError::new(
                ErrorCode::DbRefused,
                "MongoDB's answer to find had no cursor.",
            )
        })?;
        let batch = if docs.is_empty() && cur.contains_key("firstBatch") {
            "firstBatch"
        } else {
            "nextBatch"
        };
        docs.extend(names(cur, batch));
        cursor_id = match cur.get("id") {
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => i64::from(*i),
            Some(Bson::Double(d)) => *d as i64,
            _ => 0,
        };
        if cursor_id == 0 || docs.len() >= want {
            break;
        }
        r = c.command(&req.db, doc! { "getMore": cursor_id, "collection": &req.coll, "batchSize": (want - docs.len()).max(1) as i64 }).await?;
    }
    if cursor_id != 0 {
        let _ = c
            .command(
                &req.db,
                doc! { "killCursors": &req.coll, "cursors": [cursor_id] },
            )
            .await;
    }
    let more = docs.len() > limit as usize;
    docs.truncate(limit as usize);
    Ok(MgPage {
        docs: docs.iter().map(mg_doc).collect(),
        more,
        took_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    })
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgCount {
    pub n: f64,
    /// `true` when it is the collection's metadata estimate (no filter), not a count.
    pub estimate: bool,
}

/// With no filter the metadata estimate (instant); otherwise an exact count of the matches.
pub async fn count(c: &MongoConn, db: &str, coll: &str, filter: &str) -> AppResult<MgCount> {
    let f = parse_doc(filter, "The filter")?;
    let num = |b: Option<&Bson>| match b {
        Some(Bson::Int32(i)) => f64::from(*i),
        Some(Bson::Int64(i)) => *i as f64,
        Some(Bson::Double(d)) => *d,
        _ => 0.0,
    };
    if f.is_empty() {
        let r = c.command(db, doc! { "count": coll }).await?;
        return Ok(MgCount {
            n: num(r.get("n")),
            estimate: true,
        });
    }
    let r = c.command(db, doc! { "aggregate": coll, "pipeline": [{ "$match": f }, { "$count": "n" }], "cursor": {} }).await?;
    let first = r
        .get_document("cursor")
        .map(|c| names(c, "firstBatch"))
        .unwrap_or_default();
    Ok(MgCount {
        n: first.first().map_or(0.0, |d| num(d.get("n"))),
        estimate: false,
    })
}

/// What a write to one document does (S44's editor, delete, and "new document").
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MgDocOp {
    Insert {
        doc: String,
    },
    /// Replace the document with this `_id` by the edited one.
    Replace {
        id: String,
        doc: String,
    },
    Delete {
        id: String,
    },
}

/// A write held for its confirmation, already parsed.
#[derive(Debug, Clone)]
pub struct MgWrite {
    pub db: String,
    pub coll: String,
    pub op: Parsed,
}

#[derive(Debug, Clone)]
pub enum Parsed {
    Insert(Document),
    Replace(Bson, Document),
    Delete(Bson),
    /// A command built and checked in Rust (an aggregation that writes, an index, a view, validation rules): sent as
    /// it is, after the gate. `class` is what the gate decides on.
    Raw {
        class: Class,
        summary: String,
        cmd: Document,
    },
    /// A `Raw` that changes what a filter matches: the gate counts the matches and shows a sample first.
    Match {
        class: Class,
        summary: String,
        cmd: Document,
        coll: String,
        filter: Document,
    },
}

impl Parsed {
    pub fn class(&self) -> Class {
        match self {
            Parsed::Raw { class, .. } | Parsed::Match { class, .. } => *class,
            _ => Class::Write,
        }
    }
}

/// A document whose top-level keys start with `$` would be sent as update operators, not stored as it is.
fn no_operators(d: &Document) -> AppResult<()> {
    match d.keys().find(|k| k.starts_with('$')) {
        Some(k) => Err(AppError::validation(format!("A document can't have a top-level field named {k}: that is an update operator. Edit the fields themselves."))),
        None => Ok(()),
    }
}

impl Parsed {
    pub fn parse(op: &MgDocOp) -> AppResult<Parsed> {
        Ok(match op {
            MgDocOp::Insert { doc } => {
                let d = parse_doc(doc, "The document")?;
                no_operators(&d)?;
                Parsed::Insert(d)
            }
            MgDocOp::Replace { id, doc } => {
                let (id, d) = (parse_id(id)?, parse_doc(doc, "The document")?);
                no_operators(&d)?;
                // A replacement can't change `_id` (the server refuses; say it before sending).
                if d.get("_id").is_some_and(|x| *x != id) {
                    return Err(AppError::validation(
                        "The _id of a document can't be changed. Insert a new document instead.",
                    ));
                }
                Parsed::Replace(id, d)
            }
            MgDocOp::Delete { id } => Parsed::Delete(parse_id(id)?),
        })
    }
}

/// The words of the confirmation: what will happen.
pub fn describe(w: &MgWrite) -> String {
    match &w.op {
        Parsed::Insert(_) => format!("Insert 1 document into {}.{}.", w.db, w.coll),
        Parsed::Replace(id, _) => format!(
            "Replace the document with _id {} in {}.{}.",
            id.clone().into_relaxed_extjson(),
            w.db,
            w.coll
        ),
        Parsed::Delete(id) => format!(
            "Delete the document with _id {} from {}.{}. This can't be undone.",
            id.clone().into_relaxed_extjson(),
            w.db,
            w.coll
        ),
        Parsed::Raw { summary, .. } | Parsed::Match { summary, .. } => summary.clone(),
    }
}

/// How many documents carry this `_id` now (0 or 1), and the document, for the confirmation's "Show sample".
pub async fn current(c: &MongoConn, w: &MgWrite) -> AppResult<Option<Document>> {
    let id = match &w.op {
        Parsed::Replace(id, _) | Parsed::Delete(id) => id.clone(),
        Parsed::Insert(_) | Parsed::Raw { .. } | Parsed::Match { .. } => return Ok(None),
    };
    let r = c
        .command(
            &w.db,
            doc! { "find": &w.coll, "filter": { "_id": id }, "limit": 1_i64, "singleBatch": true },
        )
        .await?;
    Ok(r.get_document("cursor")
        .map(|c| names(c, "firstBatch"))
        .unwrap_or_default()
        .into_iter()
        .next())
}

/// Runs the held write. Returns how many documents it touched; a replace or delete that matched nothing is an error
/// (someone else removed the document), never a silent success.
pub async fn run(c: &MongoConn, w: &MgWrite) -> AppResult<u32> {
    let (db, coll) = (&w.db, &w.coll);
    match &w.op {
        Parsed::Raw { .. } | Parsed::Match { .. } => {
            run_reply(c, w).await?;
            Ok(1)
        }
        Parsed::Insert(d) => {
            c.command(db, doc! { "insert": coll, "documents": [d.clone()] })
                .await?;
            Ok(1)
        }
        Parsed::Replace(id, d) => {
            let r = c.command(db, doc! { "update": coll, "updates": [{ "q": { "_id": id.clone() }, "u": d.clone(), "multi": false }] }).await?;
            matched(
                &r,
                "That document no longer exists, so nothing was replaced.",
            )
        }
        Parsed::Delete(id) => {
            let r = c.command(db, doc! { "delete": coll, "deletes": [{ "q": { "_id": id.clone() }, "limit": 1 }] }).await?;
            let n = match r.get("n") {
                Some(Bson::Int32(i)) => *i as u32,
                Some(Bson::Int64(i)) => *i as u32,
                _ => 0,
            };
            if n == 0 {
                return Err(AppError::new(
                    ErrorCode::DbRefused,
                    "That document no longer exists, so nothing was deleted.",
                ));
            }
            Ok(n)
        }
    }
}

/// Runs a held command (`Raw` or `Match`) and returns the server's reply.
pub async fn run_reply(c: &MongoConn, w: &MgWrite) -> AppResult<Document> {
    let (Parsed::Raw { cmd, .. } | Parsed::Match { cmd, .. }) = &w.op else {
        return Err(AppError::internal("run_reply is for held commands"));
    };
    let r = c.command(&w.db, cmd.clone()).await?;
    // An aggregation that writes ($out, $merge) leaves a cursor: let it go.
    if let Some(Bson::Int64(id)) = r.get_document("cursor").ok().and_then(|c| c.get("id")) {
        if *id != 0 {
            let _ = c
                .command(&w.db, doc! { "killCursors": &w.coll, "cursors": [*id] })
                .await;
        }
    }
    Ok(r)
}

/// How many documents a filter matches, and up to three of them (canonical JSON), for a confirmation.
pub async fn match_preview(
    c: &MongoConn,
    db: &str,
    coll: &str,
    filter: &Document,
) -> AppResult<(f64, Vec<String>)> {
    let r = c
        .command(db, doc! { "count": coll, "query": filter.clone() })
        .await?;
    let n = match r.get("n") {
        Some(Bson::Int32(i)) => f64::from(*i),
        Some(Bson::Int64(i)) => *i as f64,
        Some(Bson::Double(d)) => *d,
        _ => 0.0,
    };
    let r = c
        .command(
            db,
            doc! { "find": coll, "filter": filter.clone(), "limit": 3_i64, "singleBatch": true },
        )
        .await?;
    let sample = r
        .get_document("cursor")
        .map(|c| names(c, "firstBatch"))
        .unwrap_or_default()
        .iter()
        .map(canonical)
        .collect();
    Ok((n, sample))
}

fn matched(r: &Document, none: &str) -> AppResult<u32> {
    let n = match r.get("n") {
        Some(Bson::Int32(i)) => *i as u32,
        Some(Bson::Int64(i)) => *i as u32,
        _ => 0,
    };
    if n == 0 {
        return Err(AppError::new(ErrorCode::DbRefused, none));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_json_round_trips_every_type_exactly() {
        let text = r#"{"_id":{"$oid":"64b64c3f0000000000000001"},"big":{"$numberLong":"9007199254740993"},"i":{"$numberInt":"7"},"d":{"$numberDouble":"1.5"},"dec":{"$numberDecimal":"1.10"},"when":{"$date":{"$numberLong":"1700000000000"}},"bin":{"$binary":{"base64":"AQID","subType":"00"}},"re":{"$regularExpression":{"pattern":"^a","options":"i"}},"ts":{"$timestamp":{"t":1,"i":2}},"nothing":null,"list":[1,"x",{"k":true}]}"#;
        let d = parse_doc(text, "t").unwrap();
        assert!(matches!(
            d.get("big"),
            Some(Bson::Int64(9_007_199_254_740_993))
        ));
        assert!(matches!(d.get("i"), Some(Bson::Int32(7))));
        assert!(matches!(d.get("dec"), Some(Bson::Decimal128(_))));
        let back = parse_doc(&canonical(&d), "t").unwrap();
        assert_eq!(d, back);
        // Relaxed keeps ObjectId and Date readable and numbers plain.
        let r = relaxed(&d);
        assert!(r.contains("\"$oid\"") && r.contains("\"$date\""));
        assert!(id_text(&d).contains("$oid"));
        assert_eq!(parse_id(&id_text(&d)).unwrap(), *d.get("_id").unwrap());
        // String and compound ids too.
        let s = doc! { "_id": "abc" };
        assert_eq!(parse_id(&id_text(&s)).unwrap(), Bson::String("abc".into()));
    }

    #[test]
    fn field_order_and_compound_ids_keep_their_order() {
        let d = parse_doc(
            r#"{"_id":{"b":1,"a":2},"z":1,"a":2,"m":{"y":1,"x":2}}"#,
            "t",
        )
        .unwrap();
        assert_eq!(d.keys().collect::<Vec<_>>(), ["_id", "z", "a", "m"]);
        let back = parse_doc(&canonical(&d), "t").unwrap();
        assert_eq!(back.keys().collect::<Vec<_>>(), ["_id", "z", "a", "m"]);
        // A compound `_id` matches server-side only in its own order.
        let id = parse_id(&id_text(&d)).unwrap();
        assert_eq!(
            id.as_document().unwrap().keys().collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[test]
    fn operators_cannot_be_smuggled_in_as_a_replacement_or_an_insert() {
        let rep = MgDocOp::Replace {
            id: "1".into(),
            doc: r#"{"$set":{"a":1}}"#.into(),
        };
        assert!(Parsed::parse(&rep).unwrap_err().message.contains("$set"));
        assert!(Parsed::parse(&MgDocOp::Insert {
            doc: r#"{"$inc":{"a":1}}"#.into()
        })
        .is_err());
        assert!(Parsed::parse(&MgDocOp::Insert {
            doc: r#"{"a":{"$gt":1}}"#.into()
        })
        .is_ok());
    }

    #[test]
    fn bad_json_and_non_objects_say_what_is_wrong() {
        assert!(parse_doc("", "The filter").unwrap().is_empty());
        assert!(parse_doc("{ status: 'open' }", "The filter")
            .unwrap_err()
            .message
            .starts_with("The filter isn't valid JSON"));
        assert!(parse_doc("[1]", "The filter")
            .unwrap_err()
            .message
            .contains("must be a JSON object"));
    }

    #[test]
    fn a_replacement_cannot_change_the_id() {
        let op = MgDocOp::Replace {
            id: r#"{"$numberInt":"1"}"#.into(),
            doc: r#"{"_id":2,"a":1}"#.into(),
        };
        assert!(Parsed::parse(&op).is_err());
        let ok = MgDocOp::Replace {
            id: "1".into(),
            doc: r#"{"_id":1,"a":2}"#.into(),
        };
        assert!(Parsed::parse(&ok).is_ok());
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target, TlsMode};
    use crate::dbs::Engine;
    use crate::mongo::conn::MongoLogin;
    use std::sync::Arc;
    use zeroize::Zeroizing;

    async fn conn() -> MongoConn {
        let reopen: crate::sql::pg::Reopen = Arc::new(|| {
            Box::pin(async {
                transport::direct_for(
                    Engine::Mongodb,
                    &Target {
                        address: "127.0.0.1".into(),
                        port: 17017,
                        tls: false,
                        tls_server_name: None,
                    },
                )
                .await
            })
        });
        let stream = (reopen)().await.unwrap();
        let login = MongoLogin {
            user: "app".into(),
            password: Some(Zeroizing::new("test-mongo".into())),
            auth_db: "admin".into(),
            tls: TlsMode::Off,
            server_name: "127.0.0.1".into(),
        };
        MongoConn::connect(stream, login, reopen).await.unwrap().0
    }

    fn req(filter: &str, sort: &str, skip: u32, limit: u32) -> MgFindRequest {
        MgFindRequest {
            db: "d8docs".into(),
            coll: "items".into(),
            filter: filter.into(),
            project: String::new(),
            sort: sort.into(),
            skip,
            limit,
            collation: String::new(),
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_browse_find_count_and_edit_documents() {
        let c = conn().await;
        let _ = c.command("d8docs", doc! { "drop": "items" }).await;
        let many: Vec<Document> = (0..1_200).map(|i| doc! { "_id": i, "n": i64::from(i) * 10_000_000_000, "status": if i % 4 == 0 { "open" } else { "done" }, "name": format!("Item {i}"), "pad": "x".repeat(20_000) }).collect();
        for chunk in many.chunks(300) {
            c.command(
                "d8docs",
                doc! { "insert": "items", "documents": chunk.to_vec() },
            )
            .await
            .unwrap();
        }
        // The tree.
        assert!(tree(&c, &[])
            .await
            .unwrap()
            .iter()
            .any(|n| n.name == "d8docs"));
        assert!(tree(&c, &["d8docs".into()])
            .await
            .unwrap()
            .iter()
            .any(|n| n.name == "items" && n.kind == "collection"));
        // A page of 500 crosses several getMore batches (20 KB documents: the first batch is cut by size).
        let p = find(&c, &req("", r#"{"_id":1}"#, 0, 500)).await.unwrap();
        assert_eq!(p.docs.len(), 500);
        assert!(p.more);
        assert!(
            p.docs[0].relaxed.contains("\"Item 0\"")
                && p.docs[499].relaxed.contains("\"Item 499\"")
        );
        let p3 = find(&c, &req("", r#"{"_id":1}"#, 1_000, 500))
            .await
            .unwrap();
        assert_eq!((p3.docs.len(), p3.more), (200, false));
        // Filter, projection, collation.
        let open = find(
            &c,
            &req(r#"{"status":"open","_id":{"$lte":30}}"#, "", 0, 50),
        )
        .await
        .unwrap();
        assert_eq!(open.docs.len(), 8);
        let mut r = req(r#"{"_id":4}"#, "", 0, 5);
        r.project = r#"{"name":1,"n":1}"#.into();
        let one = find(&c, &r).await.unwrap();
        assert!(
            !one.docs[0].relaxed.contains("pad")
                && get(&c, "d8docs", "items", &one.docs[0].id)
                    .await
                    .unwrap()
                    .contains("$numberLong")
        );
        let mut r = req(r#"{"name":"ITEM 5"}"#, "", 0, 5);
        assert!(find(&c, &r).await.unwrap().docs.is_empty());
        r.collation = r#"{"locale":"en","strength":2}"#.into();
        assert_eq!(find(&c, &r).await.unwrap().docs.len(), 1);
        // The cursor was killed, not left open.
        let cursors = c.command("admin", doc! { "aggregate": 1, "pipeline": [{ "$currentOp": { "idleCursors": true } }, { "$match": { "type": "idleCursor", "ns": "d8docs.items" } }], "cursor": {} }).await.map(|r| r.get_document("cursor").map(|c| c.get_array("firstBatch").map(|a| a.len()).unwrap_or(0)).unwrap_or(0));
        if let Ok(n) = cursors {
            assert_eq!(n, 0, "open cursors left behind");
        }
        // Counts: estimate with no filter, exact with one.
        let all = count(&c, "d8docs", "items", "").await.unwrap();
        assert_eq!((all.n, all.estimate), (1_200.0, true));
        let some = count(&c, "d8docs", "items", r#"{"status":"open"}"#)
            .await
            .unwrap();
        assert_eq!((some.n, some.estimate), (300.0, false));
        // Bad JSON is the window's mistake, not the server's.
        assert_eq!(
            find(&c, &req("{status:1}", "", 0, 5))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
        // Edit with exact types: Int64 stays Int64, Decimal128 stays Decimal128.
        let w = |op: &MgDocOp| MgWrite {
            db: "d8docs".into(),
            coll: "items".into(),
            op: Parsed::parse(op).unwrap(),
        };
        let ins = w(&MgDocOp::Insert { doc: r#"{"_id":5000,"big":{"$numberLong":"9007199254740993"},"dec":{"$numberDecimal":"1.10"}}"#.into() });
        assert_eq!(run(&c, &ins).await.unwrap(), 1);
        let dup = run(&c, &ins).await.unwrap_err();
        assert!(dup.message.contains("E11000"), "{}", dup.message);
        let got = find(&c, &req(r#"{"_id":5000}"#, "", 0, 1)).await.unwrap();
        let whole = get(&c, "d8docs", "items", &got.docs[0].id).await.unwrap();
        let rep = w(&MgDocOp::Replace {
            id: got.docs[0].id.clone(),
            doc: whole.replace("1.10", "2.50"),
        });
        assert_eq!(run(&c, &rep).await.unwrap(), 1);
        let after = find(&c, &req(r#"{"_id":5000}"#, "", 0, 1)).await.unwrap();
        let whole = get(&c, "d8docs", "items", &after.docs[0].id).await.unwrap();
        assert!(whole.contains("2.50") && whole.contains("9007199254740993"));
        assert!(get(&c, "d8docs", "items", "99999").await.is_err());
        let del = w(&MgDocOp::Delete {
            id: got.docs[0].id.clone(),
        });
        assert!(current(&c, &del).await.unwrap().is_some());
        assert_eq!(run(&c, &del).await.unwrap(), 1);
        // Gone already: an error, not a silent success.
        assert!(run(&c, &del).await.is_err());
        assert!(run(&c, &rep).await.is_err());
        let _ = c.command("d8docs", doc! { "drop": "items" }).await;
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::dbs::transport::{self, Target, TlsMode};
    use crate::dbs::Engine;
    use crate::mongo::conn::MongoLogin;
    use std::sync::{atomic::AtomicBool, Arc};
    use zeroize::Zeroizing;

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_export_writes_one_exact_document_per_line() {
        let reopen: crate::sql::pg::Reopen = Arc::new(|| {
            Box::pin(async {
                transport::direct_for(
                    Engine::Mongodb,
                    &Target {
                        address: "127.0.0.1".into(),
                        port: 17017,
                        tls: false,
                        tls_server_name: None,
                    },
                )
                .await
            })
        });
        let stream = (reopen)().await.unwrap();
        let login = MongoLogin {
            user: "app".into(),
            password: Some(Zeroizing::new("test-mongo".into())),
            auth_db: "admin".into(),
            tls: TlsMode::Off,
            server_name: "127.0.0.1".into(),
        };
        let c = MongoConn::connect(stream, login, reopen).await.unwrap().0;
        let _ = c.command("d10x", doc! { "drop": "e" }).await;
        let rows: Vec<Document> = (0..2500).map(|i| doc! { "_id": i, "big": i64::from(i) * 10_000_000_000, "name": format!("né {i}") }).collect();
        for ch in rows.chunks(500) {
            c.command("d10x", doc! { "insert": "e", "documents": ch.to_vec() })
                .await
                .unwrap();
        }
        let mut out: Vec<u8> = vec![];
        let n = export(
            &c,
            "d10x",
            "e",
            "",
            &[],
            &mut out,
            &AtomicBool::new(false),
            &|_, _| {},
        )
        .await
        .unwrap();
        assert_eq!(n, 2500.0);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2500);
        let last = parse_doc(text.lines().last().unwrap(), "t").unwrap();
        assert_eq!(last.get_i64("big").unwrap(), 2499 * 10_000_000_000);
        assert_eq!(last.get_str("name").unwrap(), "né 2499");
        let mut out: Vec<u8> = vec![];
        assert_eq!(
            export(
                &c,
                "d10x",
                "e",
                r#"{"_id":{"$lt":10}}"#,
                &[],
                &mut out,
                &AtomicBool::new(false),
                &|_, _| {}
            )
            .await
            .unwrap(),
            10.0
        );
        assert!(export(
            &c,
            "d10x",
            "e",
            "{bad",
            &[],
            &mut vec![],
            &AtomicBool::new(false),
            &|_, _| {}
        )
        .await
        .is_err());
        // Masked (E5): the names are made up and the ids untouched.
        let mut out: Vec<u8> = vec![];
        let rule = crate::dbs::masking::MaskRule {
            column: "name".into(),
            kind: crate::dbs::masking::MaskKind::FakeName,
            value: None,
        };
        export(
            &c,
            "d10x",
            "e",
            r#"{"_id":{"$lt":50}}"#,
            &[rule],
            &mut out,
            &AtomicBool::new(false),
            &|_, _| {},
        )
        .await
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 50);
        assert!(!text.contains("né "), "{text}");
        let first = parse_doc(text.lines().next().unwrap(), "t").unwrap();
        assert_eq!(first.get_i32("_id").unwrap(), 0);
        assert!(first.get_str("name").unwrap().contains(' '));
        let _ = c.command("d10x", doc! { "drop": "e" }).await;
    }
}
