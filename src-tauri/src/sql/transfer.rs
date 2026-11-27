//! Export and import of a table (S43, F26). Rust does the work and the file: the window only chooses and watches.
//! Export streams to the file the person picked (CSV and TSV straight from the server's `COPY … TO STDOUT`, JSON and
//! SQL inserts row by row from a cursor) inside a READ ONLY transaction, so a big table never sits in the window.
//! Import reads a CSV, maps its columns to the table's, and loads it with `COPY … FROM STDIN` in ONE transaction: if
//! any row is refused, nothing is kept.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::pg::{check_filter, map_error, quote, PgConn};
use super::run::kind_of;
use super::values::CellKind;
use crate::error::{AppError, AppResult, ErrorCode};

/// Largest CSV Dalan reads into memory to import.
pub const MAX_IMPORT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Tsv,
    Json,
    Sql,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Tsv => "tsv",
            ExportFormat::Json => "json",
            ExportFormat::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TransferProgress {
    pub rows: f64,
    pub bytes: f64,
}

pub(super) fn json_value(kind: CellKind, v: Option<&str>) -> serde_json::Value {
    use serde_json::Value;
    let Some(v) = v else { return Value::Null };
    match kind {
        CellKind::Bool => Value::Bool(v == "t" || v == "true"),
        // A bigint beyond 2^53 stays text, so no digit is lost on the way into other tools.
        CellKind::Int => v
            .parse::<i64>()
            .ok()
            .filter(|n| n.abs() < (1 << 53))
            .map_or_else(|| Value::String(v.into()), Value::from),
        CellKind::Float => v
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| Value::String(v.into()), Value::Number),
        CellKind::Json => serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.into())),
        _ => Value::String(v.into()),
    }
}

fn sql_literal(kind: CellKind, v: Option<&str>) -> String {
    match v {
        None => "NULL".into(),
        Some(t) => match kind {
            CellKind::Int | CellKind::Decimal | CellKind::Float
                if t.parse::<f64>().is_ok_and(f64::is_finite) =>
            {
                t.into()
            }
            CellKind::Bool if t == "t" || t == "true" => "true".into(),
            CellKind::Bool if t == "f" || t == "false" => "false".into(),
            _ => format!("'{}'", t.replace('\'', "''")),
        },
    }
}

impl PgConn {
    #[allow(clippy::too_many_arguments)]
    /// Writes the table to `out` in `format`. `stop` ends it early (the file then holds what was written).
    /// Returns the rows and bytes written.
    pub async fn export_table(
        &self,
        schema: &str,
        table: &str,
        filter: &str,
        format: ExportFormat,
        header: bool,
        masks: &[crate::dbs::masking::MaskRule],
        out: &mut (dyn Write + Send),
        progress: &(dyn Fn(f64, f64) + Sync),
        stop: &AtomicBool,
    ) -> AppResult<(f64, f64)> {
        let filter = filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        // The table must exist (and its name is quoted from what the catalog says).
        let info = self.table_info(schema, table).await?;
        // Masking (E5): a new key per export, rules by column; masked rows go through the row-by-row writer.
        let names: Vec<String> = info.columns.iter().map(|c| c.name.clone()).collect();
        let mask = crate::dbs::masking::Masker::new(crate::dbs::masking::new_key(), masks, &names);
        let from = format!("{}.{}", quote(schema), quote(table));
        let select = if filter.is_empty() {
            format!("select * from {from}")
        } else {
            format!("select * from {from} where ({filter})")
        };
        let _scope = self.scope().await;
        self.dirty.store(true, Ordering::SeqCst);
        self.client
            .batch_execute("BEGIN READ ONLY")
            .await
            .map_err(|e| map_error(&e))?;
        let result = self
            .export_inner(
                &from,
                &select,
                format,
                header,
                mask.as_ref(),
                out,
                progress,
                stop,
            )
            .await;
        if self.client.batch_execute("ROLLBACK").await.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn export_inner(
        &self,
        from: &str,
        select: &str,
        format: ExportFormat,
        header: bool,
        mask: Option<&crate::dbs::masking::Masker>,
        out: &mut (dyn Write + Send),
        progress: &(dyn Fn(f64, f64) + Sync),
        stop: &AtomicBool,
    ) -> AppResult<(f64, f64)> {
        let io = |e: std::io::Error| AppError::internal(format!("Couldn't write the file: {e}"));
        let (mut rows, mut bytes) = (0f64, 0f64);
        match format {
            ExportFormat::Csv | ExportFormat::Tsv if mask.is_none() => {
                let delim = if format == ExportFormat::Tsv {
                    ", DELIMITER E'\\t'"
                } else {
                    ""
                };
                let sink = self
                    .client
                    .copy_out(
                        format!(
                            "COPY ({select}) TO STDOUT WITH (FORMAT csv, HEADER {header}{delim})"
                        )
                        .as_str(),
                    )
                    .await
                    .map_err(|e| map_error(&e))?;
                futures_util::pin_mut!(sink);
                // Rows end at a line break outside quotes (a quoted value can hold line breaks of its own).
                let mut quoted = false;
                while let Some(chunk) = sink.next().await {
                    let chunk = chunk.map_err(|e| map_error(&e))?;
                    out.write_all(&chunk).map_err(io)?;
                    bytes += chunk.len() as f64;
                    for b in chunk.iter() {
                        match b {
                            b'"' => quoted = !quoted,
                            b'\n' if !quoted => rows += 1.0,
                            _ => {}
                        }
                    }
                    progress(rows, bytes);
                    if stop.load(Ordering::SeqCst) {
                        return Err(AppError::new(ErrorCode::Internal, "Stopped."));
                    }
                }
                if header {
                    rows = (rows - 1.0).max(0.0);
                }
            }
            _ => {
                let delim = if format == ExportFormat::Tsv {
                    '\t'
                } else {
                    ','
                };
                let stmt = self
                    .client
                    .prepare(select)
                    .await
                    .map_err(|e| map_error(&e))?;
                let cols: Vec<(String, CellKind)> = stmt
                    .columns()
                    .iter()
                    .map(|c| (c.name().to_string(), kind_of(c.type_())))
                    .collect();
                self.client
                    .batch_execute(&format!(
                        "DECLARE dalan_export NO SCROLL CURSOR FOR\n{select}"
                    ))
                    .await
                    .map_err(|e| map_error(&e))?;
                let names = cols
                    .iter()
                    .map(|(n, _)| quote(n))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut put = |s: &str, bytes: &mut f64| -> AppResult<()> {
                    out.write_all(s.as_bytes()).map_err(io)?;
                    *bytes += s.len() as f64;
                    Ok(())
                };
                match format {
                    ExportFormat::Json => put("[", &mut bytes)?,
                    ExportFormat::Csv | ExportFormat::Tsv if header => {
                        let line = cols
                            .iter()
                            .map(|(n, _)| super::mytransfer::field(&Some(n.clone()), delim))
                            .collect::<Vec<_>>()
                            .join(&delim.to_string());
                        put(&format!("{line}\n"), &mut bytes)?;
                    }
                    _ => {}
                }
                'outer: loop {
                    let msgs = self
                        .client
                        .simple_query("FETCH FORWARD 500 FROM dalan_export")
                        .await
                        .map_err(|e| map_error(&e))?;
                    let mut got = 0;
                    let mut batch: Vec<Vec<Option<String>>> = msgs
                        .iter()
                        .filter_map(|m| match m {
                            tokio_postgres::SimpleQueryMessage::Row(r) => Some(
                                (0..cols.len())
                                    .map(|i| r.get(i).map(str::to_string))
                                    .collect(),
                            ),
                            _ => None,
                        })
                        .collect();
                    if let Some(m) = mask {
                        m.batch(&mut batch);
                    }
                    for row in &batch {
                        let r = RowView(row);
                        got += 1;
                        let line = if matches!(format, ExportFormat::Csv | ExportFormat::Tsv) {
                            format!(
                                "{}\n",
                                row.iter()
                                    .map(|v| super::mytransfer::field(v, delim))
                                    .collect::<Vec<_>>()
                                    .join(&delim.to_string())
                            )
                        } else if format == ExportFormat::Json {
                            let obj: serde_json::Map<String, serde_json::Value> = cols
                                .iter()
                                .enumerate()
                                .map(|(i, (n, k))| (n.clone(), json_value(*k, r.get(i))))
                                .collect();
                            format!(
                                "{}\n  {}",
                                if rows == 0.0 { "" } else { "," },
                                serde_json::Value::Object(obj)
                            )
                        } else {
                            let vals = cols
                                .iter()
                                .enumerate()
                                .map(|(i, (_, k))| sql_literal(*k, r.get(i)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            // The table's own name goes in the statements: importing it elsewhere starts from there.
                            format!("insert into {from} ({names}) values ({vals});\n")
                        };
                        put(&line, &mut bytes)?;
                        rows += 1.0;
                    }
                    progress(rows, bytes);
                    if stop.load(Ordering::SeqCst) {
                        return Err(AppError::new(ErrorCode::Internal, "Stopped."));
                    }
                    if got < 500 {
                        break 'outer;
                    }
                }
                if format == ExportFormat::Json {
                    put("\n]\n", &mut bytes)?;
                }
                let _ = self.client.batch_execute("CLOSE dalan_export").await;
            }
        }
        out.flush().map_err(io)?;
        Ok((rows, bytes))
    }
}

/// A masked row, read like a server row (`get(i)`).
struct RowView<'a>(&'a [Option<String>]);

impl RowView<'_> {
    fn get(&self, i: usize) -> Option<&str> {
        self.0.get(i).and_then(|v| v.as_deref())
    }
}

// ---- Import

/// Splits CSV text into records (RFC 4180: quoted fields may hold the delimiter, quotes doubled, line breaks).
pub fn parse_csv(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut field) = (Vec::new(), String::new());
    let (mut quoted, mut started) = (false, false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                started = true;
            }
            c if c == delim => {
                row.push(std::mem::take(&mut field));
                started = true;
            }
            '\r' => {}
            '\n' => {
                if started || !field.is_empty() || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                started = false;
            }
            c => {
                field.push(c);
                started = true;
            }
        }
    }
    if started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// One field written for `COPY … FORMAT csv`: an empty value is NULL (unquoted), anything else is quoted.
fn copy_field(v: &str) -> String {
    if v.is_empty() {
        String::new()
    } else {
        format!("\"{}\"", v.replace('"', "\"\""))
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CsvColumn {
    pub name: String,
    pub type_name: String,
    /// Identity and generated columns are filled by the server and can't be mapped to.
    pub generated: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CsvPreview {
    pub file_columns: Vec<String>,
    pub sample: Vec<Vec<String>>,
    pub total_rows: f64,
    pub table_columns: Vec<CsvColumn>,
    /// For each file column, the table column with the same name (case aside), if any.
    pub suggested: Vec<Option<String>>,
}

/// A prepared import: the rows to load, already mapped and written as CSV for `COPY`.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub statement: String,
    pub csv: Vec<u8>,
    pub rows: u64,
    /// MySQL and MariaDB load by batched `INSERT`: schema, table and the mapped columns. `None` for PostgreSQL's COPY.
    pub target: Option<(String, String, Vec<String>)>,
}

pub fn read_limited(path: &std::path::Path) -> AppResult<String> {
    let len = std::fs::metadata(path)
        .map_err(|e| AppError::validation(format!("Couldn't read that file: {e}")))?
        .len();
    if len > MAX_IMPORT {
        return Err(AppError::validation(
            "That file is over 256 MB. Split it, or load it with psql.",
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::validation(format!("Couldn't read that file: {e}")))?;
    String::from_utf8(bytes).map_err(|_| AppError::validation("That file isn't UTF-8 text."))
}

/// Checks a mapping against the table's columns and the file, and writes the mapped columns as CSV (an empty value is
/// NULL). Returns the target column names in file order, the CSV and the row count. Shared by every engine.
pub fn map_csv(
    pv: &CsvPreview,
    text: &str,
    delim: char,
    header: bool,
    mapping: &[Option<String>],
) -> AppResult<(Vec<String>, String, u64)> {
    if mapping.len() != pv.file_columns.len() {
        return Err(AppError::validation(
            "The mapping doesn't match the file's columns.",
        ));
    }
    let mut targets: Vec<(usize, &str)> = Vec::new();
    for (i, m) in mapping.iter().enumerate() {
        let Some(name) = m else { continue };
        let col = pv
            .table_columns
            .iter()
            .find(|c| &c.name == name)
            .ok_or_else(|| {
                AppError::validation(format!("There is no column {name} in this table."))
            })?;
        if col.generated {
            return Err(AppError::validation(format!(
                "{name} is filled in by the server and can't be loaded."
            )));
        }
        if targets.iter().any(|(_, n)| n == name) {
            return Err(AppError::validation(format!("{name} is mapped twice.")));
        }
        targets.push((i, name));
    }
    if targets.is_empty() {
        return Err(AppError::validation("Map at least one column."));
    }
    let records = parse_csv(text, delim);
    let body = if header {
        &records[1.min(records.len())..]
    } else {
        &records[..]
    };
    let mut csv = String::new();
    for (n, r) in body.iter().enumerate() {
        if r.len() != pv.file_columns.len() {
            return Err(AppError::validation(format!(
                "Row {} has {} fields, the first has {}. Nothing was loaded.",
                n + 1 + usize::from(header),
                r.len(),
                pv.file_columns.len()
            )));
        }
        csv.push_str(
            &targets
                .iter()
                .map(|(i, _)| copy_field(&r[*i]))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    Ok((
        targets.iter().map(|(_, n)| n.to_string()).collect(),
        csv,
        body.len() as u64,
    ))
}

impl PgConn {
    pub async fn import_preview(
        &self,
        schema: &str,
        table: &str,
        text: &str,
        delim: char,
        header: bool,
    ) -> AppResult<CsvPreview> {
        let info = self.table_info(schema, table).await?;
        let st = self.structure(schema, table).await?;
        let records = parse_csv(text, delim);
        let first = records.first().cloned().unwrap_or_default();
        let width = first.len();
        let file_columns: Vec<String> = if header {
            first.clone()
        } else {
            (1..=width).map(|i| format!("column {i}")).collect()
        };
        let body = if header {
            &records[1.min(records.len())..]
        } else {
            &records[..]
        };
        let table_columns: Vec<CsvColumn> = info
            .columns
            .iter()
            .map(|c| CsvColumn {
                name: c.name.clone(),
                type_name: c.type_name.clone(),
                generated: st
                    .columns
                    .iter()
                    .any(|s| s.name == c.name && s.generated.is_some()),
            })
            .collect();
        let suggested = file_columns
            .iter()
            .map(|f| {
                table_columns
                    .iter()
                    .find(|c| !c.generated && c.name.eq_ignore_ascii_case(f.trim()))
                    .map(|c| c.name.clone())
            })
            .collect();
        Ok(CsvPreview {
            file_columns,
            sample: body.iter().take(20).cloned().collect(),
            total_rows: body.len() as f64,
            table_columns,
            suggested,
        })
    }

    /// Checks the mapping against the table and builds the CSV for `COPY`. Every row must have the file's width.
    pub async fn import_plan(
        &self,
        schema: &str,
        table: &str,
        text: &str,
        delim: char,
        header: bool,
        mapping: &[Option<String>],
    ) -> AppResult<ImportPlan> {
        let pv = self
            .import_preview(schema, table, text, delim, header)
            .await?;
        let (targets, csv, rows) = map_csv(&pv, text, delim, header, mapping)?;
        let cols = targets
            .iter()
            .map(|n| quote(n))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(ImportPlan {
            statement: format!(
                "COPY {}.{} ({cols}) FROM STDIN WITH (FORMAT csv)",
                quote(schema),
                quote(table)
            ),
            rows,
            csv: csv.into_bytes(),
            target: None,
        })
    }

    /// Loads the plan in one transaction. Any refusal by the server undoes everything.
    pub async fn import_commit(&self, plan: &ImportPlan) -> AppResult<u64> {
        let _scope = self.scope().await;
        self.dirty.store(true, Ordering::SeqCst);
        self.client
            .batch_execute("BEGIN")
            .await
            .map_err(|e| map_error(&e))?;
        let out: Result<u64, tokio_postgres::Error> = async {
            let sink = self
                .client
                .copy_in::<_, Bytes>(plan.statement.as_str())
                .await?;
            futures_util::pin_mut!(sink);
            for chunk in plan.csv.chunks(64 * 1024) {
                sink.send(Bytes::copy_from_slice(chunk)).await?;
            }
            sink.finish().await
        }
        .await;
        let ended = self
            .client
            .batch_execute(if out.is_ok() { "COMMIT" } else { "ROLLBACK" })
            .await;
        if ended.is_ok() {
            self.dirty.store(false, Ordering::SeqCst);
        }
        let n = out.map_err(|e| {
            AppError::new(
                ErrorCode::DbRefused,
                format!(
                    "{} Nothing was loaded.",
                    e.as_db_error()
                        .map_or_else(|| e.to_string(), |d| d.message().to_string())
                ),
            )
        })?;
        ended.map_err(|e| map_error(&e))?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_is_read_with_quotes_delimiters_and_line_breaks() {
        let t = "id,name,note\r\n1,\"Sara, \"\"S\"\"\",\"two\nlines\"\n2,,x\n\n";
        let r = parse_csv(t, ',');
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], ["1", "Sara, \"S\"", "two\nlines"]);
        assert_eq!(r[2], ["2", "", "x"]);
        assert_eq!(parse_csv("a\tb\n1\t2", '\t')[1], ["1", "2"]);
        assert_eq!(parse_csv("\u{feff}a,b", ',')[0], ["a", "b"]);
        assert!(parse_csv("", ',').is_empty());
        // No final newline: the last row still counts.
        assert_eq!(parse_csv("a\nb", ',').len(), 2);
    }

    #[test]
    fn csv_quirks_are_what_the_import_documents() {
        // A quote in the middle of a field is just a character; an empty field and "" both load as NULL.
        assert_eq!(parse_csv("a\"b,c", ',')[0], ["a\"b", "c"]);
        assert_eq!(parse_csv("a,\"\",c", ',')[0], ["a", "", "c"]);
        assert_eq!(parse_csv("only\n\nmore", ',').len(), 2);
    }

    #[test]
    fn copy_writes_empty_as_null_and_everything_else_quoted() {
        assert_eq!(copy_field(""), "");
        assert_eq!(copy_field("a\"b,c"), "\"a\"\"b,c\"");
    }

    #[test]
    fn values_keep_their_types_in_json_and_sql() {
        assert_eq!(json_value(CellKind::Int, Some("42")), serde_json::json!(42));
        assert_eq!(
            json_value(CellKind::Int, Some("9223372036854775807")),
            serde_json::json!("9223372036854775807")
        );
        assert_eq!(
            json_value(CellKind::Bool, Some("t")),
            serde_json::json!(true)
        );
        assert_eq!(
            json_value(CellKind::Json, Some("{\"a\":1}")),
            serde_json::json!({"a": 1})
        );
        assert_eq!(json_value(CellKind::Text, None), serde_json::Value::Null);
        assert_eq!(sql_literal(CellKind::Text, Some("it's")), "'it''s'");
        assert_eq!(sql_literal(CellKind::Int, Some("7")), "7");
        assert_eq!(sql_literal(CellKind::Decimal, Some("1; drop")), "'1; drop'");
        assert_eq!(sql_literal(CellKind::Bool, Some("f")), "false");
        assert_eq!(sql_literal(CellKind::Text, None), "NULL");
        // NaN and Infinity are not numbers SQL accepts bare.
        assert_eq!(sql_literal(CellKind::Float, Some("NaN")), "'NaN'");
        assert_eq!(
            sql_literal(CellKind::Float, Some("-Infinity")),
            "'-Infinity'"
        );
    }
}
