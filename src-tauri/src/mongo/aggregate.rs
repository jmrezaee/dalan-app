//! The aggregation builder (S45; F28): stages that can be switched off, the output after any stage, a hint, the
//! pipeline as code, a view made from it, and `$out`/`$merge` held by the gate like any other write.

use bson::{doc, Bson, Document};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::conn::MongoConn;
use super::docs::{names, parse_doc, parse_value, MgDoc, Parsed};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::sql::classify::Class;

/// One stage as the builder holds it: the operator (`$match`) and its argument as extended JSON text.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct MgStage {
    pub op: String,
    pub body: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct MgAggRequest {
    pub db: String,
    pub coll: String,
    pub stages: Vec<MgStage>,
    /// Run up to and including this stage (its index in `stages`); `None` is all of them.
    pub upto: Option<u32>,
    /// How many output documents to bring back.
    pub sample: u32,
    /// An index name or a key pattern like `{"status":1}`; empty for none.
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgAggResult {
    pub docs: Vec<MgDoc>,
    pub more: bool,
    pub took_ms: u32,
    /// The run stopped before a `$out` or `$merge`, which only runs after a confirmation.
    pub stopped_at_write: bool,
}

const WRITES: [&str; 2] = ["$out", "$merge"];

fn is_write(op: &str) -> bool {
    WRITES.iter().any(|w| w.eq_ignore_ascii_case(op))
}

/// A write stage hiding inside another stage's argument ($facet, $lookup and $unionWith take sub-pipelines).
fn hides_write(b: &Bson) -> bool {
    match b {
        Bson::Document(d) => d.iter().any(|(k, v)| is_write(k) || hides_write(v)),
        Bson::Array(a) => a.iter().any(hides_write),
        _ => false,
    }
}

fn stage_doc(s: &MgStage) -> AppResult<Document> {
    if !s.op.starts_with('$') || s.op.len() < 2 || s.op.contains(char::is_whitespace) {
        return Err(AppError::validation(format!("\"{}\" isn't a stage name. Stages look like $match or $group.", s.op)));
    }
    let body = if s.body.trim().is_empty() { Bson::Document(Document::new()) } else { parse_value(&s.body, &format!("The {} stage", s.op))? };
    Ok(doc! { s.op.clone(): body })
}

/// The stages to run: enabled ones up to `upto`. With `preview` a `$out`/`$merge` ends the list (and says so).
pub fn pipeline(stages: &[MgStage], upto: Option<u32>, preview: bool) -> AppResult<(Vec<Document>, bool)> {
    let mut out = vec![];
    for (i, s) in stages.iter().enumerate() {
        if upto.is_some_and(|u| i as u32 > u) {
            break;
        }
        if !s.enabled {
            continue;
        }
        if preview && is_write(&s.op) {
            return Ok((out, true));
        }
        let d = stage_doc(s)?;
        // Only a top-level $out/$merge is a write the gate can show; one inside another stage is refused outright.
        if !is_write(&s.op) && d.values().any(hides_write) {
            return Err(AppError::validation(format!("The {} stage contains $out or $merge. Those are only allowed as a stage of their own, at the end.", s.op)));
        }
        out.push(d);
    }
    Ok((out, false))
}

fn hint_value(h: &str) -> AppResult<Option<Bson>> {
    let h = h.trim();
    if h.is_empty() {
        return Ok(None);
    }
    if h.starts_with('{') {
        return Ok(Some(Bson::Document(parse_doc(h, "The hint")?)));
    }
    Ok(Some(Bson::String(h.trim_matches('"').to_string())))
}

/// The output of the pipeline (up to `upto`): at most `sample` documents. Never writes.
pub async fn run(c: &MongoConn, req: &MgAggRequest) -> AppResult<MgAggResult> {
    let started = std::time::Instant::now();
    let (stages, stopped) = pipeline(&req.stages, req.upto, true)?;
    let sample = req.sample.clamp(1, 500);
    let mut cmd = doc! { "aggregate": &req.coll, "pipeline": stages, "cursor": { "batchSize": i64::from(sample + 1) } };
    if let Some(h) = hint_value(&req.hint)? {
        cmd.insert("hint", h);
    }
    let r = c.command(&req.db, cmd).await?;
    let cur = r.get_document("cursor").map_err(|_| AppError::new(ErrorCode::DbRefused, "MongoDB's answer to aggregate had no cursor."))?;
    let mut docs = names(cur, "firstBatch");
    if let Some(Bson::Int64(id)) = cur.get("id") {
        if *id != 0 {
            let _ = c.command(&req.db, doc! { "killCursors": &req.coll, "cursors": [*id] }).await;
        }
    }
    let more = docs.len() > sample as usize;
    docs.truncate(sample as usize);
    Ok(MgAggResult { docs: docs.iter().map(super::docs::mg_doc).collect(), more, took_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32, stopped_at_write: stopped })
}

/// The whole pipeline as a command the gate can hold: `Write` when it ends in `$out` or `$merge`, otherwise it is
/// only a read and needs no hold (the caller uses `run`).
pub fn write_command(req: &MgAggRequest) -> AppResult<Parsed> {
    let (stages, _) = pipeline(&req.stages, None, false)?;
    let target = stages.iter().find_map(|d| WRITES.iter().find_map(|w| d.iter().find(|(k, _)| k.eq_ignore_ascii_case(w)).map(|(_, v)| (*w, v.clone()))));
    let Some((op, arg)) = target else {
        return Err(AppError::validation("This pipeline doesn't end in $out or $merge, so there is nothing to confirm. Run it to see the output."));
    };
    let into = match &arg {
        Bson::String(s) => s.clone(),
        Bson::Document(d) => d.get_str("into").or_else(|_| d.get_str("coll")).map(str::to_string).or_else(|_| d.get_document("into").and_then(|i| i.get_str("coll")).map(str::to_string)).unwrap_or_else(|_| "another collection".into()),
        _ => "another collection".into(),
    };
    let summary = if op == "$out" { format!("Run the pipeline on {}.{} and replace the collection {into} with its output.", req.db, req.coll) } else { format!("Run the pipeline on {}.{} and merge its output into {into}.", req.db, req.coll) };
    Ok(Parsed::Raw { class: Class::Write, summary, cmd: doc! { "aggregate": &req.coll, "pipeline": stages, "cursor": {} } })
}

/// `createView`: a named, read-only pipeline over a collection.
pub fn view_command(db: &str, coll: &str, name: &str, stages: &[MgStage]) -> AppResult<Parsed> {
    if name.trim().is_empty() || name.contains('$') || name.starts_with("system.") {
        return Err(AppError::validation("Give the view a name without $ and not starting with system."));
    }
    let (p, _) = pipeline(stages, None, false)?;
    if p.iter().any(|d| d.keys().any(|k| is_write(k))) {
        return Err(AppError::validation("A view can't end in $out or $merge."));
    }
    Ok(Parsed::Raw { class: Class::Schema, summary: format!("Create the view {name} on {db}.{coll} with {} stage{}.", p.len(), if p.len() == 1 { "" } else { "s" }), cmd: doc! { "create": name, "viewOn": coll, "pipeline": p } })
}

#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CodeLanguage {
    Mongosh,
    Node,
    Python,
    Json,
}

/// The pipeline as code to paste. It is carried as extended JSON text, so ObjectIds, dates and Long numbers keep
/// their types when the other program reads it.
pub fn code(coll: &str, stages: &[MgStage], language: CodeLanguage) -> AppResult<String> {
    let (p, _) = pipeline(stages, None, false)?;
    let value = Bson::Array(p.into_iter().map(Bson::Document).collect()).into_canonical_extjson();
    let json = serde_json::to_string_pretty(&value).unwrap_or_default();
    // The pipeline goes into the pasted code as one quoted string (a JSON string literal is valid JS and Python), so
    // nothing in a value can end the string or start code, and escapes mean the same thing in every language.
    let text = serde_json::to_string(&value.to_string()).unwrap_or_default();
    let q = serde_json::to_string(coll).unwrap_or_default();
    Ok(match language {
        CodeLanguage::Json => json,
        CodeLanguage::Mongosh => format!("db.getCollection({q}).aggregate(EJSON.parse({text})).toArray()"),
        CodeLanguage::Node => format!("import {{ EJSON }} from \"bson\"\n\nconst pipeline = EJSON.parse({text})\nconst docs = await db.collection({q}).aggregate(pipeline).toArray()"),
        CodeLanguage::Python => format!("from bson import json_util\n\npipeline = json_util.loads({text})\ndocs = list(db[{q}].aggregate(pipeline))"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(op: &str, body: &str, enabled: bool) -> MgStage {
        MgStage { op: op.into(), body: body.into(), enabled }
    }

    #[test]
    fn stages_can_be_switched_off_and_the_run_can_stop_at_one() {
        let s = [st("$match", r#"{"a":1}"#, true), st("$sort", r#"{"a":1}"#, false), st("$limit", "5", true), st("$out", r#""copy""#, true)];
        assert_eq!(pipeline(&s, None, true).unwrap(), (vec![doc! {"$match": {"a": 1}}, doc! {"$limit": 5}], true));
        assert_eq!(pipeline(&s, Some(0), true).unwrap().0.len(), 1);
        assert_eq!(pipeline(&s, None, false).unwrap().0.len(), 3);
        assert!(pipeline(&[st("match", "{}", true)], None, true).is_err());
        assert!(pipeline(&[st("$match", "{a:1}", true)], None, true).unwrap_err().message.contains("$match stage isn't valid JSON"));
    }

    #[test]
    fn writes_and_views_are_built_for_the_gate() {
        let req = |stages| MgAggRequest { db: "d".into(), coll: "c".into(), stages, upto: None, sample: 20, hint: String::new() };
        assert!(write_command(&req(vec![st("$match", "{}", true)])).is_err());
        let w = write_command(&req(vec![st("$match", "{}", true), st("$merge", r#"{"into":"totals"}"#, true)])).unwrap();
        assert!(matches!(&w, Parsed::Raw { class: Class::Write, summary, .. } if summary.contains("totals")));
        assert!(view_command("d", "c", "v", &[st("$out", r#""x""#, true)]).is_err());
        assert!(matches!(view_command("d", "c", "v", &[st("$match", "{}", true)]).unwrap(), Parsed::Raw { class: Class::Schema, .. }));
    }

    #[test]
    fn a_write_hidden_in_another_stage_is_refused_and_casing_does_not_help() {
        let nested = [st("$facet", r#"{"a":[{"$match":{}},{"$out":"x"}]}"#, true)];
        assert!(pipeline(&nested, None, true).unwrap_err().message.contains("$out or $merge"));
        let lookup = [st("$lookup", r#"{"from":"c","pipeline":[{"$merge":{"into":"x"}}],"as":"j"}"#, true)];
        assert!(pipeline(&lookup, None, false).is_err());
        // `$Out` is a write for the gate (the server would refuse it, but the gate does not rely on that).
        assert_eq!(pipeline(&[st("$Out", r#""x""#, true)], None, true).unwrap(), (vec![], true));
        assert!(view_command("d", "c", "v", &[st("$MERGE", "{}", true)]).is_err());
    }

    #[test]
    fn code_cannot_be_broken_out_of() {
        let s = [st("$match", r#"{"note":"a`b${process.exit()}\\ \"q\" \n"}"#, true)];
        for l in [CodeLanguage::Mongosh, CodeLanguage::Node, CodeLanguage::Python] {
            let c = code("o\"rders", &s, l).unwrap();
            // The text is one string literal: its parsed value is the compact pipeline, whatever it holds.
            let lit = c.lines().find(|l| l.contains("$match")).unwrap();
            let start = lit.find("parse(").or_else(|| lit.find("loads(")).unwrap() + lit[lit.find("parse(").or_else(|| lit.find("loads(")).unwrap()..].find('(').unwrap() + 1;
            let parsed: String = serde_json::Deserializer::from_str(&lit[start..]).into_iter::<String>().next().unwrap().unwrap_or_else(|e| panic!("{l:?}: {e}: {lit}"));
            assert!(parsed.contains("process.exit()"));
        }
    }

    #[test]
    fn code_keeps_types_and_names_the_collection() {
        let s = [st("$match", r#"{"n":{"$numberLong":"9007199254740993"}}"#, true)];
        for l in [CodeLanguage::Mongosh, CodeLanguage::Node, CodeLanguage::Python, CodeLanguage::Json] {
            assert!(code("orders", &s, l).unwrap().contains("9007199254740993"), "{l:?}");
        }
        assert!(code("orders", &s, CodeLanguage::Python).unwrap().contains("db[\"orders\"]"));
    }
}
