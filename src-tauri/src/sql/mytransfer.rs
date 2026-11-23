//! Export and import of a table on MySQL and MariaDB (S43, F26), the counterpart of `transfer.rs`. Export streams the
//! rows to the file as they arrive (never kept in memory); import loads by batched `INSERT` inside one transaction
//! (`LOAD DATA LOCAL` is refused: the server would read a file from this computer, which Dalan never offers).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use super::mydb::{check_filter, lit, q, MyDb};
use super::mysql::MyColumn;
use super::transfer::{json_value, map_csv, parse_csv, CsvColumn, CsvPreview, ExportFormat, ImportPlan};
use super::values::CellKind;
use crate::error::{AppError, AppResult, ErrorCode};

/// One field of a CSV or TSV line: NULL is empty, an empty string is `""`, anything with a separator, quote or line
/// break is quoted.
fn field(v: &Option<String>, delim: char) -> String {
    match v {
        None => String::new(),
        Some(t) if t.is_empty() => "\"\"".into(),
        Some(t) if t.contains(delim) || t.contains('"') || t.contains('\n') || t.contains('\r') => format!("\"{}\"", t.replace('"', "\"\"")),
        Some(t) => t.clone(),
    }
}

/// A value in an `INSERT`: numbers as they are, binary as `0x…`, everything else quoted.
fn literal(kind: CellKind, v: &Option<String>) -> String {
    match v {
        None => "NULL".into(),
        Some(t) => match kind {
            CellKind::Int | CellKind::Decimal | CellKind::Float if t.parse::<f64>().is_ok_and(f64::is_finite) => t.clone(),
            CellKind::Bytes if t.starts_with("\\x") && t.len() > 2 => format!("0x{}", &t[2..]),
            CellKind::Bytes if t == "\\x" => "''".into(),
            _ => lit(t),
        },
    }
}

impl MyDb {
    /// Names of the columns the server computes (`GENERATED`).
    async fn generated_columns(&self, schema: &str, table: &str) -> AppResult<Vec<String>> {
        Ok(self
            .read_only(async |c| c.query(&format!("select column_name from information_schema.columns where table_schema = {} and table_name = {} and extra like '%GENERATED%'", lit(schema), lit(table)), 0).await)
            .await?
            .rows
            .into_iter()
            .filter_map(|r| r[0].clone())
            .collect())
    }

    /// Writes the table to `out`. Stopping (or any error) returns `Err`, and the caller removes the half-written file.
    #[allow(clippy::too_many_arguments)]
    pub async fn export_table(&self, schema: &str, table: &str, filter: &str, format: ExportFormat, header: bool, out: &mut (dyn Write + Send), progress: &(dyn Fn(f64, f64) + Sync), stop: &AtomicBool) -> AppResult<(f64, f64)> {
        let filter = filter.trim();
        if !filter.is_empty() {
            check_filter(filter)?;
        }
        self.table_info(schema, table).await?;
        let from = format!("{}.{}", q(schema), q(table));
        // A generated column can't be given a value on reload: the SQL format leaves them out.
        let list = if format == ExportFormat::Sql {
            let info = self.table_info(schema, table).await?;
            let generated = self.generated_columns(schema, table).await?;
            info.columns.iter().filter(|c| !generated.contains(&c.name)).map(|c| q(&c.name)).collect::<Vec<_>>().join(", ")
        } else {
            "*".to_string()
        };
        let sql = if filter.is_empty() { format!("select {list} from {from}") } else { format!("select {list} from {from} where ({filter})") };
        let _scope = self.gate.lock().await;
        self.conn.query("START TRANSACTION READ ONLY", 0).await?;
        let (mut rows, mut bytes) = (0f64, 0f64);
        let mut failure: Option<AppError> = None;
        let io = |e: std::io::Error| AppError::internal(format!("Couldn't write the file: {e}"));
        let put = |text: &str, bytes: &mut f64, out: &mut (dyn Write + Send)| -> Result<(), AppError> {
            out.write_all(text.as_bytes()).map_err(io)?;
            *bytes += text.len() as f64;
            Ok(())
        };
        let mut first = true;
        let delim = if format == ExportFormat::Tsv { '\t' } else { ',' };
        let result = self
            .conn
            .stream(&sql, |cols: &[MyColumn], row: Vec<Option<String>>| {
                let step = (|| -> Result<(), AppError> {
                    if first {
                        first = false;
                        match format {
                            ExportFormat::Csv | ExportFormat::Tsv if header => {
                                let line = cols.iter().map(|c| field(&Some(c.name.clone()), delim)).collect::<Vec<_>>().join(&delim.to_string());
                                put(&format!("{line}\n"), &mut bytes, out)?;
                            }
                            ExportFormat::Json => put("[", &mut bytes, out)?,
                            _ => {}
                        }
                    }
                    let line = match format {
                        ExportFormat::Csv | ExportFormat::Tsv => format!("{}\n", row.iter().map(|v| field(v, delim)).collect::<Vec<_>>().join(&delim.to_string())),
                        ExportFormat::Json => {
                            let obj: serde_json::Map<String, serde_json::Value> = cols.iter().zip(&row).map(|(c, v)| (c.name.clone(), json_value(c.kind, v.as_deref()))).collect();
                            format!("{}\n  {}", if rows == 0.0 { "" } else { "," }, serde_json::Value::Object(obj))
                        }
                        ExportFormat::Sql => format!("insert into {from} ({}) values ({});\n", cols.iter().map(|c| q(&c.name)).collect::<Vec<_>>().join(", "), cols.iter().zip(&row).map(|(c, v)| literal(c.kind, v)).collect::<Vec<_>>().join(", ")),
                    };
                    put(&line, &mut bytes, out)?;
                    rows += 1.0;
                    if rows % 500.0 == 0.0 {
                        progress(rows, bytes);
                    }
                    Ok(())
                })();
                if let Err(e) = step {
                    failure = Some(e);
                    return false;
                }
                !stop.load(Ordering::SeqCst)
            })
            .await;
        let _ = self.conn.query("ROLLBACK", 0).await;
        let (cols, _) = result?;
        if let Some(e) = failure {
            return Err(e);
        }
        if stop.load(Ordering::SeqCst) {
            return Err(AppError::new(ErrorCode::Internal, "Stopped."));
        }
        // A table with no rows still has its header (CSV) and its brackets (JSON).
        if first {
            match format {
                ExportFormat::Csv | ExportFormat::Tsv if header => put(&format!("{}\n", cols.iter().map(|c| field(&Some(c.name.clone()), delim)).collect::<Vec<_>>().join(&delim.to_string())), &mut bytes, out)?,
                ExportFormat::Json => put("[", &mut bytes, out)?,
                _ => {}
            }
        }
        if format == ExportFormat::Json {
            put("\n]\n", &mut bytes, out)?;
        }
        out.flush().map_err(io)?;
        progress(rows, bytes);
        Ok((rows, bytes))
    }

    pub async fn import_preview(&self, schema: &str, table: &str, text: &str, delim: char, header: bool) -> AppResult<CsvPreview> {
        let info = self.table_info(schema, table).await?;
        let generated = self.generated_columns(schema, table).await?;
        let records = parse_csv(text, delim);
        let first = records.first().cloned().unwrap_or_default();
        let file_columns: Vec<String> = if header { first.clone() } else { (1..=first.len()).map(|i| format!("column {i}")).collect() };
        let body = if header { &records[1.min(records.len())..] } else { &records[..] };
        let table_columns: Vec<CsvColumn> = info.columns.iter().map(|c| CsvColumn { name: c.name.clone(), type_name: c.type_name.clone(), generated: generated.contains(&c.name) }).collect();
        let suggested = file_columns.iter().map(|f| table_columns.iter().find(|c| !c.generated && c.name.eq_ignore_ascii_case(f.trim())).map(|c| c.name.clone())).collect();
        Ok(CsvPreview { file_columns, sample: body.iter().take(20).cloned().collect(), total_rows: body.len() as f64, table_columns, suggested })
    }

    pub async fn import_plan(&self, schema: &str, table: &str, text: &str, delim: char, header: bool, mapping: &[Option<String>]) -> AppResult<ImportPlan> {
        let pv = self.import_preview(schema, table, text, delim, header).await?;
        let (targets, csv, rows) = map_csv(&pv, text, delim, header, mapping)?;
        // A table that can't roll back (MyISAM, Aria) would keep the batches loaded before a failure.
        let engine = self.read_only(async |c| c.query(&format!("select engine from information_schema.tables where table_schema = {} and table_name = {}", lit(schema), lit(table)), 1).await).await?;
        if engine.rows.first().and_then(|r| r[0].as_deref()).is_some_and(|e| !e.eq_ignore_ascii_case("InnoDB")) {
            return Err(AppError::validation("This table isn't InnoDB, so a failed load could leave part of the file in it. Load it with a query or convert the table first."));
        }
        let cols = targets.iter().map(|n| q(n)).collect::<Vec<_>>().join(", ");
        Ok(ImportPlan { statement: format!("INSERT INTO {}.{} ({cols}) VALUES (…), (…), …  (batches of up to 500 rows, one transaction)", q(schema), q(table)), rows, csv: csv.into_bytes(), target: Some((schema.to_string(), table.to_string(), targets)) })
    }

    /// Loads the rows in batches inside one transaction; the server refusing any row undoes everything. An empty value
    /// is NULL (as in the CSV the plan was built from).
    pub async fn import_commit(&self, plan: &ImportPlan) -> AppResult<u64> {
        let (schema, table, cols) = plan.target.as_ref().ok_or_else(|| AppError::internal("This import was prepared for another engine."))?;
        let text = String::from_utf8_lossy(&plan.csv).into_owned();
        let records = parse_csv(&text, ',');
        let head = format!("insert into {}.{} ({}) values ", q(schema), q(table), cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "));
        let _scope = self.gate.lock().await;
        // Strict mode for the load, so a NULL for a NOT NULL column is an error and not a silent default.
        self.conn.query("SET @dalan_mode = @@sql_mode", 0).await?;
        let _ = self.conn.query("SET SESSION sql_mode = CONCAT_WS(',', @@sql_mode, 'STRICT_TRANS_TABLES')", 0).await;
        self.conn.query("START TRANSACTION", 0).await?;
        let mut loaded = 0u64;
        let mut failure = None;
        let mut batch: Vec<String> = Vec::new();
        let mut size = 0usize;
        let flush = |batch: &mut Vec<String>, size: &mut usize| -> Option<String> {
            if batch.is_empty() {
                return None;
            }
            let sql = format!("{head}{}", batch.join(", "));
            batch.clear();
            *size = 0;
            Some(sql)
        };
        for (n, r) in records.iter().enumerate() {
            let tuple = format!("({})", r.iter().map(|v| if v.is_empty() { "NULL".to_string() } else { lit(v) }).collect::<Vec<_>>().join(", "));
            size += tuple.len();
            batch.push(tuple);
            if batch.len() >= 500 || size >= 1 << 20 || n + 1 == records.len() {
                let rows_in = batch.len() as u64;
                if let Some(sql) = flush(&mut batch, &mut size) {
                    match self.conn.query(&sql, 0).await {
                        Ok(_) => loaded += rows_in,
                        Err(e) => {
                            failure = Some(AppError::new(ErrorCode::DbRefused, format!("{} Nothing was loaded.", e.message)));
                            break;
                        }
                    }
                }
            }
        }
        let ended = self.conn.query(if failure.is_none() { "COMMIT" } else { "ROLLBACK" }, 0).await;
        let _ = self.conn.query("SET SESSION sql_mode = @dalan_mode", 0).await;
        if let Some(f) = failure {
            return Err(f);
        }
        ended?;
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_tell_null_from_an_empty_string_and_quote_what_needs_it() {
        assert_eq!(field(&None, ','), "");
        assert_eq!(field(&Some(String::new()), ','), "\"\"");
        assert_eq!(field(&Some("a,b".into()), ','), "\"a,b\"");
        assert_eq!(field(&Some("a,b".into()), '\t'), "a,b");
        assert_eq!(field(&Some("say \"hi\"".into()), ','), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn insert_values_keep_their_types() {
        assert_eq!(literal(CellKind::Int, &Some("7".into())), "7");
        assert_eq!(literal(CellKind::Decimal, &Some("1.50".into())), "1.50");
        assert_eq!(literal(CellKind::Float, &Some("NaN".into())), "'NaN'");
        assert_eq!(literal(CellKind::Bytes, &Some("\\xdead".into())), "0xdead");
        assert_eq!(literal(CellKind::Text, &Some("it's".into())), "'it''s'");
        assert_eq!(literal(CellKind::Text, &None), "NULL");
    }
}
