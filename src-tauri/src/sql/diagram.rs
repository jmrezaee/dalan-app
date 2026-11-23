//! The diagram (S40): tables and their foreign keys, a number of steps out from the tables picked.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use specta::Type;

use super::pg::{map_error, PgConn};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiagramColumn {
    pub name: String,
    pub type_name: String,
    pub pk: bool,
    pub fk: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiagramTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<DiagramColumn>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DiagramEdge {
    pub name: String,
    pub from_schema: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_schema: String,
    pub to_table: String,
    pub to_columns: Vec<String>,
    pub on_delete: String,
    pub on_update: String,
    /// Some index starts with the key's first column.
    pub indexed: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Diagram {
    pub tables: Vec<DiagramTable>,
    pub edges: Vec<DiagramEdge>,
    pub total_tables: u32,
}

fn action(c: &str) -> &'static str {
    match c {
        "c" => "cascade",
        "r" => "restrict",
        "n" => "set null",
        "d" => "set default",
        _ => "no action",
    }
}

/// Tables reachable from `start` in at most `steps` foreign-key hops, either direction. A pure function so the walk is tested.
pub fn reach(
    start: &[(String, String)],
    edges: &[DiagramEdge],
    steps: u32,
) -> Vec<(String, String)> {
    let mut seen: HashSet<(String, String)> = start.iter().cloned().collect();
    let mut order: Vec<(String, String)> = start.to_vec();
    let mut frontier: Vec<(String, String)> = start.to_vec();
    for _ in 0..steps {
        let mut next = Vec::new();
        for e in edges {
            let (a, b) = (
                (e.from_schema.clone(), e.from_table.clone()),
                (e.to_schema.clone(), e.to_table.clone()),
            );
            for (x, y) in [(&a, &b), (&b, &a)] {
                if frontier.contains(x) && seen.insert(y.clone()) {
                    order.push(y.clone());
                    next.push(y.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    order
}

impl PgConn {
    pub async fn diagram(&self, start: &[(String, String)], steps: u32) -> AppResult<Diagram> {
        let _scope = self.scope().await;
        let rows = self
            .client
            .query(
                "select cn.nspname::text, c.relname::text, rn.nspname::text, r.relname::text, con.conname::text,
                        array(select a.attname::text from unnest(con.conkey) k join pg_attribute a on a.attrelid = con.conrelid and a.attnum = k),
                        array(select a.attname::text from unnest(con.confkey) k join pg_attribute a on a.attrelid = con.confrelid and a.attnum = k),
                        con.confdeltype::text, con.confupdtype::text,
                        exists(select 1 from pg_index i where i.indrelid = con.conrelid and i.indkey[0] = con.conkey[1])
                 from pg_constraint con
                 join pg_class c on c.oid = con.conrelid join pg_namespace cn on cn.oid = c.relnamespace
                 join pg_class r on r.oid = con.confrelid join pg_namespace rn on rn.oid = r.relnamespace
                 where con.contype = 'f' and cn.nspname not in ('pg_catalog', 'information_schema') order by 1, 2, 5 limit 5000",
                &[],
            )
            .await
            .map_err(|e| map_error(&e))?;
        let edges: Vec<DiagramEdge> = rows
            .iter()
            .map(|r| DiagramEdge {
                from_schema: r.get(0),
                from_table: r.get(1),
                to_schema: r.get(2),
                to_table: r.get(3),
                name: r.get(4),
                from_columns: r.get(5),
                to_columns: r.get(6),
                on_delete: action(&r.get::<_, String>(7)).into(),
                on_update: action(&r.get::<_, String>(8)).into(),
                indexed: r.get(9),
            })
            .collect();
        let total: i64 = self
            .client
            .query_one("select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace where c.relkind in ('r','p') and n.nspname not in ('pg_catalog','information_schema') and n.nspname not like 'pg\\_%'", &[])
            .await
            .map_err(|e| map_error(&e))?
            .get(0);
        let names = reach(start, &edges, steps.min(5));
        let (schemas, tnames): (Vec<String>, Vec<String>) = names.iter().cloned().unzip();
        let cols = self
            .client
            .query(
                "select n.nspname::text, c.relname::text, a.attname::text, format_type(a.atttypid, a.atttypmod),
                        a.attnum = any(coalesce((select conkey from pg_constraint where conrelid = c.oid and contype = 'p' limit 1), '{}'::int2[])),
                        exists(select 1 from pg_constraint f where f.conrelid = c.oid and f.contype = 'f' and a.attnum = any(f.conkey))
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped
                 where (n.nspname::text, c.relname::text) in (select * from unnest($1::text[], $2::text[])) order by 1, 2, a.attnum",
                &[&schemas, &tnames],
            )
            .await
            .map_err(|e| map_error(&e))?;
        let mut by: HashMap<(String, String), Vec<DiagramColumn>> = HashMap::new();
        for r in &cols {
            by.entry((r.get(0), r.get(1)))
                .or_default()
                .push(DiagramColumn {
                    name: r.get(2),
                    type_name: r.get(3),
                    pk: r.get(4),
                    fk: r.get(5),
                });
        }
        if !start.is_empty() && !by.keys().any(|k| start.contains(k)) {
            return Err(crate::error::AppError::not_found(
                "That table wasn't found. Pick one from the list.",
            ));
        }
        let tables: Vec<DiagramTable> = names
            .iter()
            .filter_map(|k| {
                by.remove(k).map(|columns| DiagramTable {
                    schema: k.0.clone(),
                    name: k.1.clone(),
                    columns,
                })
            })
            .collect();
        let shown: HashSet<(String, String)> = tables
            .iter()
            .map(|t| (t.schema.clone(), t.name.clone()))
            .collect();
        let edges = edges
            .into_iter()
            .filter(|e| {
                shown.contains(&(e.from_schema.clone(), e.from_table.clone()))
                    && shown.contains(&(e.to_schema.clone(), e.to_table.clone()))
            })
            .collect();
        Ok(Diagram {
            tables,
            edges,
            total_tables: total.max(0) as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(a: &str, b: &str) -> DiagramEdge {
        DiagramEdge {
            name: format!("{a}_{b}"),
            from_schema: "public".into(),
            from_table: a.into(),
            from_columns: vec![],
            to_schema: "public".into(),
            to_table: b.into(),
            to_columns: vec![],
            on_delete: "no action".into(),
            on_update: "no action".into(),
            indexed: true,
        }
    }

    #[test]
    fn cycles_and_self_references_end() {
        let edges = [
            edge("a", "b"),
            edge("b", "a"),
            edge("a", "a"),
            edge("b", "c"),
            edge("c", "a"),
        ];
        let start = [("public".to_string(), "a".to_string())];
        assert_eq!(reach(&start, &edges, 5).len(), 3);
        assert_eq!(reach(&[], &edges, 3).len(), 0);
    }

    #[test]
    fn the_walk_goes_both_ways_and_stops_at_the_steps() {
        // items -> orders -> customers ; payments -> orders ; far -> customers
        let edges = [
            edge("items", "orders"),
            edge("orders", "customers"),
            edge("payments", "orders"),
            edge("far", "customers"),
        ];
        let start = [("public".to_string(), "orders".to_string())];
        let t = |v: Vec<(String, String)>| v.into_iter().map(|x| x.1).collect::<Vec<_>>();
        assert_eq!(t(reach(&start, &edges, 0)), ["orders"]);
        let one = t(reach(&start, &edges, 1));
        assert_eq!(one.len(), 4);
        assert!(
            one.contains(&"items".into())
                && one.contains(&"customers".into())
                && one.contains(&"payments".into())
                && !one.contains(&"far".into())
        );
        assert!(t(reach(&start, &edges, 2)).contains(&"far".into()));
    }
}
