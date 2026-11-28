//! Bulk import's file handling (D17, E6): JSON files turned into CSV for D6's import, a file's name matched to a
//! table, and column types inferred from the values for "create missing tables".

use serde::Serialize;
use specta::Type;

use crate::dbs::schemadiff::{quote_my, quote_pg, Family};
use crate::error::{AppError, AppResult};

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// JSON lines, or a JSON array of objects, as CSV with a header: the keys in the order they first appear, nested
/// values as JSON text, `null` and missing keys as empty (D6 loads empty as NULL).
pub fn json_to_csv(text: &str) -> AppResult<String> {
    let t = text.trim_start();
    let rows: Vec<serde_json::Map<String, serde_json::Value>> = if t.starts_with('[') {
        serde_json::from_str::<Vec<serde_json::Value>>(t)
            .map_err(|e| AppError::validation(format!("That isn't a JSON array of objects: {e}")))?
            .into_iter()
            .map(|v| match v {
                serde_json::Value::Object(m) => Ok(m),
                _ => Err(AppError::validation(
                    "Every item of the array must be an object.",
                )),
            })
            .collect::<AppResult<_>>()?
    } else {
        t.lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(
                |(i, l)| match serde_json::from_str::<serde_json::Value>(l) {
                    Ok(serde_json::Value::Object(m)) => Ok(m),
                    Ok(_) => Err(AppError::validation(format!(
                        "Line {} isn't a JSON object.",
                        i + 1
                    ))),
                    Err(e) => Err(AppError::validation(format!(
                        "Line {} isn't JSON: {e}",
                        i + 1
                    ))),
                },
            )
            .collect::<AppResult<_>>()?
    };
    let mut keys: Vec<String> = Vec::new();
    for r in &rows {
        for k in r.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    let mut out = keys
        .iter()
        .map(|k| csv_field(k))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for r in &rows {
        let line: Vec<String> = keys
            .iter()
            .map(|k| match r.get(k) {
                None | Some(serde_json::Value::Null) => String::new(),
                Some(serde_json::Value::String(s)) => csv_field(s),
                Some(v) => csv_field(&v.to_string()),
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    Ok(out)
}

/// The table a file is for: `schema.table.ext` or `table.ext`, compared without case to the tables there
/// (`tables` are `(schema, table)`). `None` when nothing has that name.
pub fn match_table(
    file: &str,
    tables: &[(String, String)],
    default_schema: &str,
) -> (String, String, bool) {
    let stem = file.split('.').collect::<Vec<_>>();
    let (schema, table) = match stem.len() {
        0 | 1 => (default_schema.to_string(), file.to_string()),
        2 => (default_schema.to_string(), stem[0].to_string()),
        _ => (
            stem[stem.len() - 3].to_string(),
            stem[stem.len() - 2].to_string(),
        ),
    };
    if let Some((s, t)) = tables
        .iter()
        .find(|(s, t)| s.eq_ignore_ascii_case(&schema) && t.eq_ignore_ascii_case(&table))
    {
        return (s.clone(), t.clone(), true);
    }
    if let Some((s, t)) = tables
        .iter()
        .find(|(s, t)| s.eq_ignore_ascii_case(default_schema) && t.eq_ignore_ascii_case(&table))
    {
        return (s.clone(), t.clone(), true);
    }
    // A schema from the file name that isn't on this server: the table goes into the default schema.
    let schema = if tables.iter().any(|(s, _)| s.eq_ignore_ascii_case(&schema)) {
        schema
    } else {
        default_schema.to_string()
    };
    (schema, table, false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guess {
    Int,
    Num,
    Bool,
    Date,
    Time,
    Text,
}

fn guess(v: &str) -> Guess {
    let v = v.trim();
    if v.parse::<i64>().is_ok() {
        Guess::Int
    } else if v.parse::<f64>().is_ok_and(f64::is_finite) {
        Guess::Num
    } else if matches!(
        v.to_ascii_lowercase().as_str(),
        "true" | "false" | "t" | "f"
    ) {
        Guess::Bool
    } else if chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok() {
        Guess::Date
    } else if chrono::NaiveDateTime::parse_from_str(
        &v.replacen('T', " ", 1),
        "%Y-%m-%d %H:%M:%S%.f",
    )
    .is_ok()
        || chrono::DateTime::parse_from_rfc3339(v).is_ok()
    {
        Guess::Time
    } else {
        Guess::Text
    }
}

fn widen(a: Guess, b: Guess) -> Guess {
    use Guess::*;
    match (a, b) {
        (x, y) if x == y => x,
        (Int, Num) | (Num, Int) => Num,
        (Date, Time) | (Time, Date) => Time,
        _ => Text,
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct InferredColumn {
    pub name: String,
    pub type_name: String,
}

/// Each column's type from its values (empty values say nothing), for this engine.
pub fn infer(family: Family, columns: &[String], rows: &[Vec<String>]) -> Vec<InferredColumn> {
    columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let g = rows
                .iter()
                .filter_map(|r| r.get(i))
                .filter(|v| !v.trim().is_empty())
                .take(5000)
                .map(|v| guess(v))
                .reduce(widen)
                .unwrap_or(Guess::Text);
            let type_name = match (family, g) {
                (Family::Pg, Guess::Int) => "bigint",
                (Family::Pg, Guess::Num) => "numeric",
                (Family::Pg, Guess::Bool) => "boolean",
                (Family::Pg, Guess::Date) => "date",
                (Family::Pg, Guess::Time) => "timestamp",
                (Family::Pg, Guess::Text) => "text",
                (Family::My, Guess::Int) => "bigint",
                (Family::My, Guess::Num) => "decimal(38,10)",
                (Family::My, Guess::Bool) => "tinyint(1)",
                (Family::My, Guess::Date) => "date",
                (Family::My, Guess::Time) => "datetime(6)",
                (Family::My, Guess::Text) => "text",
            };
            InferredColumn {
                name: name.clone(),
                type_name: type_name.into(),
            }
        })
        .collect()
}

/// A CREATE TABLE for these columns (and primary key, when the export's manifest names one).
pub fn create_ddl(
    family: Family,
    schema: &str,
    table: &str,
    cols: &[InferredColumn],
    pk: &[String],
) -> String {
    let q = |n: &str| match family {
        Family::Pg => quote_pg(n),
        Family::My => quote_my(n),
    };
    let mut lines: Vec<String> = cols
        .iter()
        .map(|c| format!("  {} {}", q(&c.name), c.type_name))
        .collect();
    if !pk.is_empty() {
        lines.push(format!(
            "  PRIMARY KEY ({})",
            pk.iter().map(|k| q(k)).collect::<Vec<_>>().join(", ")
        ));
    }
    format!(
        "CREATE TABLE {}.{} (\n{}\n)",
        q(schema),
        q(table),
        lines.join(",\n")
    )
}

/// A reviewed DDL must be exactly one CREATE TABLE, of this table.
pub fn check_create(family: Family, ddl: &str, table: &str) -> AppResult<()> {
    use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect};
    let parsed = match family {
        Family::Pg => sqlparser::parser::Parser::parse_sql(&PostgreSqlDialect {}, ddl),
        Family::My => sqlparser::parser::Parser::parse_sql(&MySqlDialect {}, ddl),
    }
    .map_err(|e| AppError::validation(format!("The CREATE TABLE for {table} doesn't read: {e}")))?;
    match parsed.as_slice() {
        [sqlparser::ast::Statement::CreateTable(c)] => {
            let name = c.name.to_string();
            let last = name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .trim_matches(['"', '`']);
            if last.eq_ignore_ascii_case(table) {
                Ok(())
            } else {
                Err(AppError::validation(format!(
                    "The CREATE TABLE must create {table}, not {name}."
                )))
            }
        }
        _ => Err(AppError::validation(format!(
            "The DDL for {table} must be one CREATE TABLE and nothing else."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_becomes_csv_and_types_are_inferred() {
        let csv = json_to_csv("{\"id\": 1, \"name\": \"Ada, L\", \"tags\": [1, 2]}\n\n{\"id\": 2, \"active\": true, \"name\": null}\n").unwrap();
        assert_eq!(
            csv,
            "id,name,tags,active\n1,\"Ada, L\",\"[1,2]\",\n2,,,true\n"
        );
        assert_eq!(
            json_to_csv("[{\"a\": \"x\"}, {\"a\": \"y\"}]").unwrap(),
            "a\nx\ny\n"
        );
        assert!(json_to_csv("{\"a\": 1}\n[1]").is_err());
        let rows: Vec<Vec<String>> = vec![
            vec![
                "1".into(),
                "1.5".into(),
                "true".into(),
                "2026-01-02".into(),
                "2026-01-02T10:00:00".into(),
                "x".into(),
            ],
            vec![
                "2".into(),
                "2".into(),
                "f".into(),
                "2026-01-03".into(),
                "2026-01-02".into(),
                "".into(),
            ],
        ];
        let cols: Vec<String> = ["id", "amount", "ok", "day", "at", "note"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pg: Vec<String> = infer(Family::Pg, &cols, &rows)
            .into_iter()
            .map(|c| c.type_name)
            .collect();
        assert_eq!(
            pg,
            vec!["bigint", "numeric", "boolean", "date", "timestamp", "text"]
        );
        let my = infer(Family::My, &cols, &rows);
        assert_eq!(my[1].type_name, "decimal(38,10)");
        let ddl = create_ddl(
            Family::Pg,
            "public",
            "New Table",
            &infer(Family::Pg, &cols, &rows),
            &["id".into()],
        );
        assert!(
            ddl.starts_with("CREATE TABLE public.\"New Table\" (\n  id bigint,"),
            "{ddl}"
        );
        assert!(ddl.ends_with("  PRIMARY KEY (id)\n)"), "{ddl}");
        check_create(Family::Pg, &ddl, "New Table").unwrap();
        assert!(check_create(Family::Pg, "CREATE TABLE t (a int); DROP TABLE x", "t").is_err());
        assert!(check_create(Family::Pg, "CREATE TABLE other (a int)", "t").is_err());
        assert!(check_create(Family::My, "CREATE TABLE `shop`.`t` (a int)", "t").is_ok());
    }

    #[test]
    fn file_names_find_their_tables() {
        let t: Vec<(String, String)> = vec![
            ("public".into(), "Customers".into()),
            ("billing".into(), "invoices".into()),
        ];
        assert_eq!(
            match_table("customers.csv", &t, "public"),
            ("public".into(), "Customers".into(), true)
        );
        assert_eq!(
            match_table("billing.invoices.jsonl", &t, "public"),
            ("billing".into(), "invoices".into(), true)
        );
        assert_eq!(
            match_table("shop.customers.csv", &t, "public"),
            ("public".into(), "Customers".into(), true)
        );
        assert_eq!(
            match_table("billing.new.csv", &t, "public"),
            ("billing".into(), "new".into(), false)
        );
        assert_eq!(
            match_table("elsewhere.new.csv", &t, "public"),
            ("public".into(), "new".into(), false)
        );
    }
}
