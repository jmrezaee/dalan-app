//! Data compare (D16, E4; replica/databases/restore-copy.md §9): two tables, read sorted by a key and merged, in
//! constant memory (a page of rows per side at a time).
//!
//! Both sides are read as text and put in one canonical form per kind of column before they are compared, so the
//! same value read from PostgreSQL and from MySQL compares equal (`t` and `1`, `1.50` and `1.5`, `2026-01-01T10:00`
//! and `2026-01-01 10:00:00`, JSON with its keys in any order). Keys are ordered the same way on both servers: text
//! keys bytewise (`COLLATE "C"`, `BINARY`), numbers as numbers; `cmp_keys` here orders them the same way.

use std::cmp::Ordering;

use serde::Serialize;
use specta::Type;

use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Bool,
    Num,
    Time,
    Json,
    Text,
    /// Not compared (binary text forms differ between servers).
    Binary,
}

/// The kind of a column from its `information_schema` data type (either engine).
pub fn kind_of(data_type: &str, column_type: &str) -> ValueKind {
    let t = data_type.to_ascii_lowercase();
    if t == "boolean" || column_type.eq_ignore_ascii_case("tinyint(1)") {
        return ValueKind::Bool;
    }
    match t.as_str() {
        "smallint" | "integer" | "bigint" | "int" | "tinyint" | "mediumint" | "numeric"
        | "decimal" | "real" | "double precision" | "double" | "float" | "year" => ValueKind::Num,
        "date" | "datetime" | "timestamp" | "time" => ValueKind::Time,
        "json" | "jsonb" => ValueKind::Json,
        "bytea" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary"
        | "bit" => ValueKind::Binary,
        _ if t.starts_with("timestamp") || t.starts_with("time ") => ValueKind::Time,
        _ => ValueKind::Text,
    }
}

fn trim_fraction(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

fn sort_json(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut pairs: Vec<_> = m.into_iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(pairs.into_iter().map(|(k, v)| (k, sort_json(v))).collect())
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(sort_json).collect())
        }
        other => other,
    }
}

/// One value in its canonical text form.
pub fn canon(kind: ValueKind, v: &str) -> String {
    match kind {
        ValueKind::Bool => match v {
            "t" | "true" | "TRUE" | "1" => "1".into(),
            "f" | "false" | "FALSE" | "0" => "0".into(),
            other => other.into(),
        },
        ValueKind::Num => {
            let t = trim_fraction(v.trim());
            if t == "-0" {
                "0".into()
            } else {
                t.into()
            }
        }
        ValueKind::Time => {
            let mut s = v.trim().replacen('T', " ", 1);
            for tz in ["+00:00", "+00", "Z"] {
                if let Some(x) = s.strip_suffix(tz) {
                    s = x.to_string();
                    break;
                }
            }
            trim_fraction(&s).to_string()
        }
        ValueKind::Json => serde_json::from_str::<serde_json::Value>(v)
            .map(|j| sort_json(j).to_string())
            .unwrap_or_else(|_| v.into()),
        ValueKind::Text | ValueKind::Binary => v.into(),
    }
}

/// Key order: integers as numbers, everything else bytewise; NULL keys first (they shouldn't exist).
pub fn cmp_keys(a: &[Option<String>], b: &[Option<String>]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = match (x, y) {
            (None, None) => Ordering::Equal,
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<i128>(), y.parse::<i128>()) {
                (Ok(p), Ok(q)) => p.cmp(&q),
                _ => x.as_bytes().cmp(y.as_bytes()),
            },
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// One row: its key and its compared values (canonical), in the order of the compared columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: Vec<Option<String>>,
    pub values: Vec<Option<String>>,
}

/// A side's rows in key order, a page at a time (an empty page is the end).
pub trait RowSource {
    fn page(&mut self) -> impl std::future::Future<Output = AppResult<Vec<Row>>> + Send;
}

pub enum Event<'a> {
    OnlySource(&'a Row),
    OnlyTarget(&'a Row),
    /// Both rows and the indexes of the values that differ.
    Different(&'a Row, &'a Row, Vec<usize>),
    Same,
}

/// Merges both sides; `on` sees every row once. `stop` is checked between pages.
pub async fn merge<A: RowSource + Send, B: RowSource + Send>(
    a: &mut A,
    b: &mut B,
    stop: &std::sync::atomic::AtomicBool,
    on: &mut (dyn FnMut(Event<'_>) -> AppResult<()> + Send),
) -> AppResult<()> {
    use std::collections::VecDeque;
    let (mut pa, mut pb): (VecDeque<Row>, VecDeque<Row>) = (VecDeque::new(), VecDeque::new());
    let (mut end_a, mut end_b) = (false, false);
    loop {
        if pa.is_empty() && !end_a {
            pa = a.page().await?.into();
            end_a = pa.is_empty();
        }
        if pb.is_empty() && !end_b {
            pb = b.page().await?.into();
            end_b = pb.is_empty();
        }
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::error::AppError::new(
                crate::error::ErrorCode::Internal,
                "Stopped.",
            ));
        }
        match (pa.front(), pb.front()) {
            (None, None) => return Ok(()),
            (Some(x), None) => {
                on(Event::OnlySource(x))?;
                pa.pop_front();
            }
            (None, Some(y)) => {
                on(Event::OnlyTarget(y))?;
                pb.pop_front();
            }
            (Some(x), Some(y)) => match cmp_keys(&x.key, &y.key) {
                Ordering::Less => {
                    on(Event::OnlySource(x))?;
                    pa.pop_front();
                }
                Ordering::Greater => {
                    on(Event::OnlyTarget(y))?;
                    pb.pop_front();
                }
                Ordering::Equal => {
                    let changed: Vec<usize> = x
                        .values
                        .iter()
                        .zip(&y.values)
                        .enumerate()
                        .filter(|(_, (p, q))| p != q)
                        .map(|(i, _)| i)
                        .collect();
                    on(if changed.is_empty() {
                        Event::Same
                    } else {
                        Event::Different(x, y, changed)
                    })?;
                    pa.pop_front();
                    pb.pop_front();
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pages(Vec<Vec<Row>>);
    impl RowSource for Pages {
        async fn page(&mut self) -> AppResult<Vec<Row>> {
            Ok(if self.0.is_empty() {
                vec![]
            } else {
                self.0.remove(0)
            })
        }
    }
    fn row(k: i64, v: &str) -> Row {
        Row {
            key: vec![Some(k.to_string())],
            values: vec![Some(k.to_string()), Some(v.into())],
        }
    }

    #[test]
    fn values_from_either_server_compare_in_one_form() {
        assert_eq!(canon(ValueKind::Bool, "t"), canon(ValueKind::Bool, "1"));
        assert_eq!(canon(ValueKind::Num, "1.50"), "1.5");
        assert_eq!(canon(ValueKind::Num, "10"), "10");
        assert_eq!(canon(ValueKind::Num, "2.000"), "2");
        assert_eq!(
            canon(ValueKind::Time, "2026-01-01T10:00:00.500000+00"),
            "2026-01-01 10:00:00.5"
        );
        assert_eq!(
            canon(ValueKind::Time, "2026-01-01 10:00:00.000000"),
            "2026-01-01 10:00:00"
        );
        assert_eq!(
            canon(
                ValueKind::Json,
                "{\"b\": 1, \"a\": [2, {\"d\": 1, \"c\": 0}]}"
            ),
            canon(ValueKind::Json, "{\"a\":[2,{\"c\":0,\"d\":1}],\"b\":1}")
        );
        assert_eq!(kind_of("tinyint", "tinyint(1)"), ValueKind::Bool);
        assert_eq!(kind_of("timestamp with time zone", ""), ValueKind::Time);
        assert_eq!(kind_of("bytea", ""), ValueKind::Binary);
        use std::cmp::Ordering::*;
        assert_eq!(cmp_keys(&[Some("9".into())], &[Some("10".into())]), Less);
        assert_eq!(
            cmp_keys(&[Some("B".into())], &[Some("a".into())]),
            Less,
            "bytewise, like COLLATE \"C\""
        );
        assert_eq!(
            cmp_keys(
                &[Some("1".into()), Some("b".into())],
                &[Some("1".into()), Some("a".into())]
            ),
            Greater
        );
    }

    #[tokio::test]
    async fn a_merge_finds_each_kind_of_difference_across_page_edges() {
        let mut a = Pages(vec![
            vec![row(1, "x"), row(2, "y")],
            vec![row(4, "z"), row(5, "same")],
            vec![row(9, "a")],
        ]);
        let mut b = Pages(vec![
            vec![row(2, "Y")],
            vec![row(3, "q"), row(5, "same"), row(9, "a"), row(10, "t")],
        ]);
        let (mut only_a, mut only_b, mut diff, mut same) = (vec![], vec![], vec![], 0);
        let stop = std::sync::atomic::AtomicBool::new(false);
        merge(&mut a, &mut b, &stop, &mut |e| {
            match e {
                Event::OnlySource(r) => only_a.push(r.key[0].clone().unwrap()),
                Event::OnlyTarget(r) => only_b.push(r.key[0].clone().unwrap()),
                Event::Different(r, _, cols) => diff.push((r.key[0].clone().unwrap(), cols)),
                Event::Same => same += 1,
            }
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(only_a, vec!["1", "4"]);
        assert_eq!(only_b, vec!["3", "10"]);
        assert_eq!(diff, vec![("2".to_string(), vec![1])]);
        assert_eq!(same, 2);
    }
}
