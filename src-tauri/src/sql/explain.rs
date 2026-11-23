//! Explain (S39, F25): the server's plan as a tree with times, estimate against actual, and a plain-language hint.
//! `explain analyze` runs the statement, so it only runs for reads, inside a transaction that is rolled back (a
//! READ ONLY one unless the editor has a manual transaction open, whose own state it must not end).

use std::sync::atomic::Ordering;

use serde::Serialize;
use serde_json::Value;
use specta::Type;
use tokio_postgres::types::{FromSql, Type as PgType};

use super::pg::{map_error, quote, PgConn};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Type)]
pub struct PlanNode {
    /// `Seq Scan on orders o`.
    pub name: String,
    /// The condition or key that matters: `filter: …`, `group by …`.
    pub detail: Option<String>,
    pub est_rows: f64,
    pub actual_rows: Option<f64>,
    /// This node's own time, children not counted (ms), and its share of the whole.
    pub self_ms: Option<f64>,
    pub pct: Option<f64>,
    pub total_ms: Option<f64>,
    pub children: Vec<PlanNode>,
    /// Schema and relation, for hints.
    #[serde(skip)]
    #[specta(skip)]
    pub loops: f64,
    #[serde(skip)]
    #[specta(skip)]
    pub scan: Option<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HintAction {
    pub label: String,
    /// SQL to open in the editor (never run from here).
    pub sql: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PlanHint {
    pub text: String,
    pub actions: Vec<HintAction>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ExplainPlan {
    pub root: PlanNode,
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    pub buffers_read: Option<f64>,
    pub hint: Option<PlanHint>,
}

/// The raw text of a `json` column.
struct RawJson(String);
impl<'a> FromSql<'a> for RawJson {
    fn from_sql(_: &PgType, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(RawJson(String::from_utf8(raw.to_vec())?))
    }
    fn accepts(ty: &PgType) -> bool {
        *ty == PgType::JSON
    }
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn node(v: &Value, analyzed: bool) -> PlanNode {
    let kind = s(v, "Node Type").unwrap_or_else(|| "Node".into());
    let rel = s(v, "Relation Name");
    let alias = s(v, "Alias").filter(|a| Some(a) != rel.as_ref());
    let name = match (&rel, &alias) {
        (Some(r), Some(a)) => format!("{kind} on {r} {a}"),
        (Some(r), None) => format!("{kind} on {r}"),
        _ => kind,
    };
    let list = |k: &str| v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
    let detail = s(v, "Filter")
        .map(|x| format!("filter: {x}"))
        .or_else(|| s(v, "Index Cond").map(|x| format!("index: {x}")))
        .or_else(|| s(v, "Hash Cond").or_else(|| s(v, "Merge Cond")).or_else(|| s(v, "Join Filter")))
        .or_else(|| list("Sort Key").map(|x| format!("sort by {x}")))
        .or_else(|| list("Group Key").map(|x| format!("group by {x}")));
    let loops = f(v, "Actual Loops").unwrap_or(1.0);
    let total = analyzed.then(|| f(v, "Actual Total Time").map(|t| t * loops)).flatten();
    let children: Vec<PlanNode> = v.get("Plans").and_then(Value::as_array).map(|a| a.iter().map(|c| node(c, analyzed)).collect()).unwrap_or_default();
    let kids: f64 = children.iter().filter_map(|c| c.total_ms).sum();
    PlanNode {
        name,
        detail,
        est_rows: f(v, "Plan Rows").unwrap_or(0.0),
        actual_rows: analyzed.then(|| f(v, "Actual Rows").map(|r| r * loops)).flatten(),
        self_ms: total.map(|t| (t - kids).max(0.0)),
        pct: None,
        total_ms: total,
        loops,
        scan: rel.map(|r| (s(v, "Schema").unwrap_or_default(), r, s(v, "Filter").unwrap_or_default())),
        children,
    }
}

/// Fills each node's share of the whole time.
fn shares(n: &mut PlanNode, whole: f64) {
    n.pct = n.self_ms.map(|t| if whole > 0.0 { t / whole * 100.0 } else { 0.0 });
    for c in &mut n.children {
        shares(c, whole);
    }
}

fn walk<'a>(n: &'a PlanNode, out: &mut Vec<&'a PlanNode>) {
    out.push(n);
    for c in &n.children {
        walk(c, out);
    }
}

/// The columns a filter compares (`status = 'paid' and placed_at > …`), in order, without repeats. A condition that
/// is a call (`lower(name) = …`) or part of an OR can't be served by a plain index on its column and is skipped.
fn filter_columns(filter: &str) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for part in filter.split(" AND ") {
        if part.contains(" OR ") {
            continue;
        }
        let p = part.trim_start_matches(['(', ' ']);
        let quoted = p.starts_with('"');
        let body = if quoted { &p[1..] } else { p };
        let id: String = body.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let rest = body[id.len()..].trim_start_matches('"').trim_start_matches(')').trim_start();
        let is_col = id.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
        let compared = rest.starts_with(['=', '<', '>']) || rest.starts_with("::") && rest.contains(['=', '<', '>']);
        if is_col && compared && !cols.contains(&id) {
            cols.push(id);
        }
    }
    cols
}

fn hint(root: &PlanNode, analyzed: bool) -> Option<PlanHint> {
    let mut all = Vec::new();
    walk(root, &mut all);
    let worst = all
        .iter()
        .filter(|n| n.name.starts_with("Seq Scan") && n.detail.as_deref().is_some_and(|d| d.starts_with("filter")))
        .max_by(|a, b| a.self_ms.unwrap_or(a.est_rows).total_cmp(&b.self_ms.unwrap_or(b.est_rows)))?;
    let (schema, rel, filter) = worst.scan.clone()?;
    let big = if analyzed { worst.pct.unwrap_or(0.0) >= 20.0 } else { worst.est_rows >= 10_000.0 };
    // Estimates are per loop; the actual count was multiplied by the loops, so compare per loop.
    let off = worst.actual_rows.is_some_and(|a| {
        let a = a / worst.loops.max(1.0);
        worst.est_rows >= 1.0 && (a > worst.est_rows * 10.0 || a * 10.0 < worst.est_rows)
    });
    if !big && !off {
        return None;
    }
    let mut text = format!("The scan on {rel} reads every row");
    if let (true, Some(p)) = (analyzed, worst.pct) {
        text.push_str(&format!(" and takes {p:.0}% of the time"));
    }
    text.push('.');
    if let (true, Some(a)) = (off, worst.actual_rows) {
        text.push_str(&format!(" The planner expected {} rows and got {}: statistics look stale.", worst.est_rows as i64, a as i64));
    }
    let table = format!("{}.{}", quote(&schema), quote(&rel));
    let mut actions = vec![HintAction { label: format!("Open ANALYZE {rel}"), sql: format!("analyze {table}") }];
    let cols = filter_columns(&filter);
    if !cols.is_empty() {
        let list = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        text.push_str(&format!(" An index on {rel} ({}) would let it skip the rest.", cols.join(", ")));
        actions.push(HintAction { label: "Open an index draft".into(), sql: format!("create index on {table} ({list})") });
    }
    Some(PlanHint { text, actions })
}

/// From the server's `EXPLAIN (FORMAT JSON)` text.
pub fn parse(json: &str, analyzed: bool) -> AppResult<ExplainPlan> {
    let v: Value = serde_json::from_str(json).map_err(|_| AppError::internal("The server's plan wasn't readable."))?;
    let top = v.get(0).ok_or_else(|| AppError::internal("The server sent no plan."))?;
    let plan = top.get("Plan").ok_or_else(|| AppError::internal("The server sent no plan."))?;
    let mut root = node(plan, analyzed);
    let whole = root.total_ms.unwrap_or(0.0);
    if analyzed {
        shares(&mut root, whole);
    }
    let mut all = Vec::new();
    walk(&root, &mut all);
    let _ = all;
    fn blocks(v: &Value) -> f64 {
        f(v, "Shared Read Blocks").unwrap_or(0.0) + v.get("Plans").and_then(Value::as_array).map_or(0.0, |a| a.iter().map(blocks).sum())
    }
    let h = hint(&root, analyzed);
    Ok(ExplainPlan { root, analyzed, planning_ms: f(top, "Planning Time"), execution_ms: f(top, "Execution Time"), buffers_read: analyzed.then(|| blocks(plan)), hint: h })
}

impl PgConn {
    /// Plans (or, with `analyze`, runs and times) one statement. The caller checked that `analyze` is for a read.
    pub async fn explain(&self, text: &str, analyze: bool, timeout_ms: u32) -> AppResult<ExplainPlan> {
        let _scope = self.scope().await;
        let manual = self.has_txn();
        if manual {
            // The person's transaction must not be ended or written to: work inside a savepoint that is read-only
            // and timed, and roll back to it.
            self.client
                .batch_execute(&format!("SAVEPOINT dalan_explain; SET LOCAL transaction_read_only = on; SET LOCAL statement_timeout = {timeout_ms}"))
                .await
                .map_err(|e| map_error(&e))?;
        } else {
            self.dirty.store(true, Ordering::SeqCst);
            self.client
                .batch_execute(&format!("BEGIN READ ONLY; SET LOCAL statement_timeout = {timeout_ms}"))
                .await
                .map_err(|e| map_error(&e))?;
        }
        let options = if analyze { "FORMAT JSON, ANALYZE, BUFFERS" } else { "FORMAT JSON" };
        // The extended protocol: exactly one statement, whatever the splitter thought.
        let out = self.client.query_one(&format!("EXPLAIN ({options}) {text}"), &[]).await;
        if manual {
            let _ = self.client.batch_execute("ROLLBACK TO SAVEPOINT dalan_explain; RELEASE SAVEPOINT dalan_explain").await;
        } else if self.client.batch_execute("ROLLBACK").await.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        let row = out.map_err(|e| map_error(&e))?;
        let RawJson(json) = row.get(0);
        parse(&json, analyze)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = r#"[{"Plan": {"Node Type": "Hash Join", "Plan Rows": 100, "Actual Rows": 90, "Actual Loops": 1, "Actual Total Time": 80.0, "Hash Cond": "(c.id = o.customer_id)", "Shared Read Blocks": 5,
      "Plans": [
        {"Node Type": "Seq Scan", "Relation Name": "orders", "Schema": "public", "Alias": "o", "Plan Rows": 1200, "Actual Rows": 30877, "Actual Loops": 1, "Actual Total Time": 60.0, "Filter": "((status = 'paid'::text) AND (placed_at > now()))", "Shared Read Blocks": 100},
        {"Node Type": "Hash", "Plan Rows": 5, "Actual Rows": 5, "Actual Loops": 1, "Actual Total Time": 4.0, "Plans": [{"Node Type": "Seq Scan", "Relation Name": "customers", "Alias": "c", "Plan Rows": 5, "Actual Rows": 5, "Actual Loops": 1, "Actual Total Time": 3.0}]}]},
      "Planning Time": 0.3, "Execution Time": 80.5}]"#;

    #[test]
    fn a_plan_becomes_a_tree_with_self_times_and_shares() {
        let p = parse(PLAN, true).unwrap();
        assert_eq!(p.root.name, "Hash Join");
        assert_eq!(p.root.detail.as_deref(), Some("(c.id = o.customer_id)"));
        // 80 total, children 60 + 4: the join's own time is 16 ms, 20%.
        assert!((p.root.self_ms.unwrap() - 16.0).abs() < 1e-9 && (p.root.pct.unwrap() - 20.0).abs() < 1e-9);
        assert_eq!(p.root.children[0].name, "Seq Scan on orders o");
        assert_eq!(p.root.children[0].actual_rows, Some(30877.0));
        assert_eq!(p.execution_ms, Some(80.5));
        assert_eq!(p.buffers_read, Some(105.0));
    }

    #[test]
    fn the_hint_names_the_scan_the_stale_statistics_and_an_index() {
        let h = parse(PLAN, true).unwrap().hint.unwrap();
        assert!(h.text.contains("The scan on orders reads every row and takes 75% of the time."), "{}", h.text);
        assert!(h.text.contains("expected 1200 rows and got 30877"));
        assert!(h.text.contains("An index on orders (status, placed_at)"));
        assert_eq!(h.actions[0].sql, "analyze \"public\".\"orders\"");
        assert_eq!(h.actions[1].sql, "create index on \"public\".\"orders\" (\"status\", \"placed_at\")");
        // A call or an OR is not a column an index could serve; a loop's actual rows are per loop.
        assert_eq!(filter_columns("(lower(name) = 'x'::text) AND (a = 1) AND ((b = 1) OR (c = 2)) AND ((\"d\")::text = 'y'::text)"), ["a", "d"]);
        assert_eq!(filter_columns("(status = 'paid'::text)"), ["status"]);
        let looped = PLAN.replace("\"Actual Rows\": 30877, \"Actual Loops\": 1", "\"Actual Rows\": 1200, \"Actual Loops\": 25");
        assert!(!parse(&looped, true).unwrap().hint.is_some_and(|h| h.text.contains("statistics look stale")));
        // A plan without a costly filtered scan has no hint.
        let none = r#"[{"Plan": {"Node Type": "Result", "Plan Rows": 1}}]"#;
        assert!(parse(none, false).unwrap().hint.is_none());
    }
}
