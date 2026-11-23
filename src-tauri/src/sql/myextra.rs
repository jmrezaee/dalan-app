//! MySQL and MariaDB: structure, explain, diagram and activity (D7 part 2c). The same results the window shows for
//! PostgreSQL, built from MySQL's catalog and its own tools. Read-only except `structure_commit` and `kill`, which the
//! gate (in the commands) has already approved.

use std::collections::HashMap;

use super::activity::{chains, Activity, ActivityTable, ActivityTiles, ActivityView, SessionRow};
use super::diagram::{reach, Diagram, DiagramColumn, DiagramEdge, DiagramTable};
use super::explain::{ExplainPlan, PlanHint, PlanNode};
use super::mydb::{lit, q, MyDb, MyEditor, SYSTEM};
use super::mysql::Flavour;
use super::pg::SqlTableInfo;
use super::structure::{safe_expr, StructureColumn, StructureConstraint, StructureIndex, StructureOp, StructureStats, StructureTrigger, TableStructure};
use crate::error::{AppError, AppResult};

fn s(r: &[Option<String>], i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}
fn n(r: &[Option<String>], i: usize) -> f64 {
    r.get(i).and_then(|v| v.as_deref()).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

// ---- Structure (S38)

/// Statements for the operations on MySQL or MariaDB. `info` supplies what `modify column` must repeat (it replaces
/// the whole definition), and which constraint names are foreign keys.
pub fn statements_my(schema: &str, table: &str, ops: &[StructureOp], rows: Option<i64>, info: &SqlTableInfo, ddl: &str) -> AppResult<(Vec<String>, Vec<String>)> {
    if ops.is_empty() {
        return Err(AppError::validation("There are no changes to apply."));
    }
    let t = format!("{}.{}", q(schema), q(table));
    let col = |n: &str| info.columns.iter().find(|c| c.name == n).ok_or_else(|| AppError::validation(format!("There is no column {n} in this table.")));
    let rows_text = rows.filter(|r| *r > 0).map_or(String::new(), |r| format!(" (about {r} rows)"));
    let mut out = vec![];
    let mut warn = vec!["MySQL and MariaDB apply structure changes one statement at a time and can't undo them: if a later one fails, the earlier ones stay.".to_string()];
    // `modify column` replaces the whole definition (default, NOT NULL, AUTO_INCREMENT, ON UPDATE, charset, comment,
    // generation expression), so it is built from the column's own line in SHOW CREATE TABLE, changing only the part asked.
    let definition = |name: &str| -> AppResult<(String, String)> {
        let c = col(name)?;
        let prefix = format!("`{}` ", name.replace('`', "``"));
        let line = ddl.lines().map(str::trim).find(|l| l.starts_with(&prefix)).ok_or_else(|| AppError::validation(format!("Couldn't read the definition of {name}. Change it with a query.")))?;
        let rest = line[prefix.len()..].trim_end_matches(',').to_string();
        if !rest.starts_with(&c.type_name) {
            return Err(AppError::validation(format!("Couldn't read the definition of {name}. Change it with a query.")));
        }
        Ok((c.type_name.clone(), rest[c.type_name.len()..].to_string()))
    };
    let modify = |name: &str, ty: &str, tail: &str| -> String { format!("alter table {t} modify column {} {ty}{tail}", q(name)) };
    // Where "NOT NULL" goes: after the type and its charset, before DEFAULT, COMMENT and the other attributes.
    let insert_not_null = |tail: &str| -> String {
        let at = [" DEFAULT", " COMMENT", " AUTO_INCREMENT", " ON UPDATE", " GENERATED", " AS (", " VISIBLE", " INVISIBLE", " /*"].iter().filter_map(|m| tail.find(m)).min().unwrap_or(tail.len());
        format!("{} NOT NULL{}", &tail[..at], &tail[at..])
    };
    for op in ops {
        match op {
            StructureOp::AddColumn { name, type_name, nullable, default } => {
                let mut x = format!("alter table {t} add column {} {}", q(name), safe_expr("The type", type_name, true)?);
                if !nullable {
                    x.push_str(" not null");
                }
                if let Some(d) = default {
                    x.push_str(&format!(" default {}", safe_expr("The default", d, false)?));
                }
                out.push(x);
            }
            StructureOp::DropColumn { name } => {
                out.push(format!("alter table {t} drop column {}", q(name)));
                warn.push(format!("Dropping {name} deletes everything stored in it."));
            }
            StructureOp::RenameColumn { from, to } => out.push(format!("alter table {t} rename column {} to {}", q(from), q(to))),
            StructureOp::ChangeType { column, type_name } => {
                let (_, tail) = definition(column)?;
                out.push(modify(column, &safe_expr("The type", type_name, true)?, &tail));
                warn.push(format!("Changing the type of {column} rewrites the table{rows_text}. Its other attributes (default, comment, auto_increment) are kept; a character set or collation stays as it was."));
            }
            StructureOp::SetNullable { column, nullable } => {
                let (ty, tail) = definition(column)?;
                let tail = if *nullable { tail.replace(" NOT NULL", "") } else if tail.contains(" NOT NULL") { tail } else { insert_not_null(&tail) };
                out.push(modify(column, &ty, &tail));
            }
            StructureOp::SetDefault { column, default } => out.push(match default {
                Some(d) => format!("alter table {t} alter column {} set default {}", q(column), safe_expr("The default", d, false)?),
                None => format!("alter table {t} alter column {} drop default", q(column)),
            }),
            StructureOp::CreateIndex { name, columns, unique } => {
                out.push(format!("create {}index {} on {t} ({})", if *unique { "unique " } else { "" }, q(name), columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")));
                warn.push(format!("Building an index can slow writes to the table while it is built{rows_text}."));
            }
            StructureOp::DropIndex { name } => out.push(format!("drop index {} on {t}", q(name))),
            StructureOp::DropConstraint { name } => {
                if name == "PRIMARY" {
                    out.push(format!("alter table {t} drop primary key"));
                } else if info.references.iter().any(|k| &k.name == name) {
                    out.push(format!("alter table {t} drop foreign key {}", q(name)));
                } else {
                    out.push(format!("alter table {t} drop index {}", q(name)));
                }
            }
            StructureOp::AddForeignKey { name, columns, ref_schema, ref_table, ref_columns, on_delete } => {
                let mut x = format!("alter table {t} add constraint {} foreign key ({}) references {}.{} ({})", q(name), columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "), q(ref_schema), q(ref_table), ref_columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "));
                if let Some(a) = on_delete {
                    if !["cascade", "restrict", "no action", "set null", "set default"].contains(&a.as_str()) {
                        return Err(AppError::validation("on delete must be cascade, restrict, no action, set null or set default."));
                    }
                    x.push_str(&format!(" on delete {a}"));
                }
                out.push(x);
            }
        }
    }
    Ok((out, warn))
}

impl MyDb {
    pub async fn structure(&self, schema: &str, table: &str) -> AppResult<TableStructure> {
        let info = self.table_info(schema, table).await?;
        let (sc, tb) = (lit(schema), lit(table));
        self.read_only(async |c| {
            let st = c.query(&format!("select table_rows, data_length, index_length, update_time from information_schema.tables where table_schema = {sc} and table_name = {tb}"), 1).await?;
            let r = st.rows.first().cloned().unwrap_or_default();
            let stats = StructureStats { rows_estimate: r.first().and_then(|v| v.as_deref()).and_then(|v| v.parse().ok()), table_bytes: n(&r, 1) as i64, index_bytes: n(&r, 2) as i64, last_vacuum: None };
            let extra = c.query(&format!("select column_name, extra from information_schema.columns where table_schema = {sc} and table_name = {tb}"), 0).await?.rows;
            let columns = info
                .columns
                .iter()
                .map(|col| {
                    let e = extra.iter().find(|r| s(r, 0) == col.name).map(|r| s(r, 1)).unwrap_or_default();
                    let generated = (e.contains("auto_increment") || e.contains("GENERATED")).then(|| e.clone());
                    StructureColumn { name: col.name.clone(), type_name: col.type_name.clone(), nullable: col.nullable, default: if generated.is_some() { None } else { col.default.clone() }, generated }
                })
                .collect();
            // Constraints: primary key and unique keys from the index list, foreign keys from the catalog.
            let idx_rows = c.query(&format!("select index_name, non_unique, column_name, seq_in_index, index_type from information_schema.statistics where table_schema = {sc} and table_name = {tb} order by index_name, seq_in_index"), 0).await?.rows;
            let mut indexes: Vec<(String, bool, Vec<String>, String)> = vec![];
            for r in &idx_rows {
                match indexes.iter_mut().find(|i| i.0 == s(r, 0)) {
                    Some(i) => i.2.push(if s(r, 2).is_empty() { "(expression)".into() } else { s(r, 2) }),
                    None => indexes.push((s(r, 0), s(r, 1) == "0", vec![if s(r, 2).is_empty() { "(expression)".into() } else { s(r, 2) }], s(r, 4))),
                }
            }
            let mut constraints: Vec<StructureConstraint> = vec![];
            for (name, unique, cols, _) in &indexes {
                if name == "PRIMARY" {
                    constraints.push(StructureConstraint { name: name.clone(), kind: "p".into(), definition: format!("PRIMARY KEY ({})", cols.join(", ")), columns: cols.clone() });
                } else if *unique {
                    constraints.push(StructureConstraint { name: name.clone(), kind: "u".into(), definition: format!("UNIQUE ({})", cols.join(", ")), columns: cols.clone() });
                }
            }
            for k in &info.references {
                constraints.push(StructureConstraint { name: k.name.clone(), kind: "f".into(), definition: format!("FOREIGN KEY ({}) REFERENCES {}.{}({})", k.columns.join(", "), k.schema, k.table, k.target_columns.join(", ")), columns: k.columns.clone() });
            }
            let indexes = indexes
                .into_iter()
                .map(|(name, unique, cols, ty)| StructureIndex { primary: name == "PRIMARY", definition: format!("{} ({})", ty, cols.join(", ")), name, unique, bytes: 0, scans: -1 })
                .collect();
            let triggers = c
                .query(&format!("select trigger_name, action_timing, event_manipulation, action_statement from information_schema.triggers where event_object_schema = {sc} and event_object_table = {tb} order by trigger_name"), 0)
                .await?
                .rows
                .iter()
                .map(|r| StructureTrigger { name: s(r, 0), definition: format!("{} {} {}", s(r, 1), s(r, 2), s(r, 3)), enabled: true })
                .collect();
            let ddl = c.query(&format!("show create table {}.{}", q(schema), q(table)), 1).await.ok().and_then(|r| r.rows.first().map(|r| format!("{};\n", s(r, 1)))).unwrap_or_default();
            Ok(TableStructure { stats, columns, constraints, indexes, triggers, ddl })
        })
        .await
    }

    /// One statement at a time (structure changes commit by themselves here). A failure says which statements had
    /// already been applied, because nothing can undo them.
    pub async fn structure_commit(&self, stmts: &[String]) -> AppResult<()> {
        let _scope = self.gate.lock().await;
        // A table another transaction holds locked must not make this wait for the server's default (a year).
        let _ = self.conn.query("SET SESSION lock_wait_timeout = 10", 0).await;
        let out = self.run_structure(stmts).await;
        let _ = self.conn.query("SET SESSION lock_wait_timeout = DEFAULT", 0).await;
        // Whatever happened, something may have changed: the completion list is read again.
        *self.catalog.lock().await = None;
        out
    }

    async fn run_structure(&self, stmts: &[String]) -> AppResult<()> {
        for (i, st) in stmts.iter().enumerate() {
            if let Err(e) = self.conn.query(st, 0).await {
                let done = if i == 0 { "Nothing was changed.".to_string() } else { format!("The first {i} statement{} {} already applied and can't be undone.", if i == 1 { "" } else { "s" }, if i == 1 { "was" } else { "were" }) };
                return Err(AppError::new(e.code, format!("Statement {} failed: {} {done}", i + 1, e.message)));
            }
        }
        Ok(())
    }

    // ---- Diagram (S40)
    pub async fn diagram(&self, start: &[(String, String)], steps: u32) -> AppResult<Diagram> {
        self.read_only(async |c| {
            let rows = c
                .query(&format!("select k.table_schema, k.table_name, k.constraint_name, k.column_name, k.referenced_table_schema, k.referenced_table_name, k.referenced_column_name, r.delete_rule, r.update_rule
                    from information_schema.key_column_usage k join information_schema.referential_constraints r on r.constraint_schema = k.constraint_schema and r.constraint_name = k.constraint_name and r.table_name = k.table_name
                    where k.referenced_table_name is not null and k.table_schema not in {SYSTEM} order by 1, 2, 3, k.ordinal_position limit 20000"), 0)
                .await?
                .rows;
            let first_cols: Vec<(String, String, String)> = c.query(&format!("select table_schema, table_name, column_name from information_schema.statistics where seq_in_index = 1 and table_schema not in {SYSTEM} limit 50000"), 0).await?.rows.iter().map(|r| (s(r, 0), s(r, 1), s(r, 2))).collect();
            let mut edges: Vec<DiagramEdge> = vec![];
            for r in &rows {
                match edges.iter_mut().find(|e| e.name == s(r, 2) && e.from_schema == s(r, 0) && e.from_table == s(r, 1)) {
                    Some(e) => {
                        e.from_columns.push(s(r, 3));
                        e.to_columns.push(s(r, 6));
                    }
                    None => edges.push(DiagramEdge { name: s(r, 2), from_schema: s(r, 0), from_table: s(r, 1), from_columns: vec![s(r, 3)], to_schema: s(r, 4), to_table: s(r, 5), to_columns: vec![s(r, 6)], on_delete: s(r, 7).to_lowercase(), on_update: s(r, 8).to_lowercase(), indexed: false }),
                }
            }
            for e in &mut edges {
                e.indexed = first_cols.iter().any(|(sc, t, col)| *sc == e.from_schema && *t == e.from_table && Some(col) == e.from_columns.first());
            }
            let total = c.query(&format!("select count(*) from information_schema.tables where table_type = 'BASE TABLE' and table_schema not in {SYSTEM}"), 1).await?.rows.first().map(|r| n(r, 0)).unwrap_or(0.0);
            let names = reach(start, &edges, steps.min(5));
            let mut tables = vec![];
            for (sc, t) in &names {
                let cols = c.query(&format!("select column_name, column_type, column_key from information_schema.columns where table_schema = {} and table_name = {} order by ordinal_position", lit(sc), lit(t)), 0).await?.rows;
                if cols.is_empty() {
                    continue;
                }
                tables.push(DiagramTable {
                    schema: sc.clone(),
                    name: t.clone(),
                    columns: cols.iter().map(|r| DiagramColumn { name: s(r, 0), type_name: s(r, 1), pk: s(r, 2) == "PRI", fk: edges.iter().any(|e| e.from_schema == *sc && e.from_table == *t && e.from_columns.contains(&s(r, 0))) }).collect(),
                });
            }
            if !start.is_empty() && !tables.iter().any(|t| start.contains(&(t.schema.clone(), t.name.clone()))) {
                return Err(AppError::not_found("That table wasn't found. Pick one from the list."));
            }
            let shown: Vec<(String, String)> = tables.iter().map(|t| (t.schema.clone(), t.name.clone())).collect();
            let edges = edges.into_iter().filter(|e| shown.contains(&(e.from_schema.clone(), e.from_table.clone())) && shown.contains(&(e.to_schema.clone(), e.to_table.clone()))).collect();
            Ok(Diagram { tables, edges, total_tables: total as u32 })
        })
        .await
    }

    // ---- Activity (S41)
    pub async fn activity(&self, view: ActivityView) -> AppResult<Activity> {
        let own: Vec<u32> = self.own.lock().map(|o| o.clone()).unwrap_or_default();
        self.read_only(async |c| {
            let status: HashMap<String, f64> = c
                .query("show global status where variable_name in ('Threads_connected', 'Com_commit', 'Com_rollback', 'Innodb_buffer_pool_reads', 'Innodb_buffer_pool_read_requests')", 0)
                .await?
                .rows
                .iter()
                .map(|r| (s(r, 0), n(r, 1)))
                .collect();
            let max = c.query("select @@max_connections", 1).await?.rows.first().map(|r| n(r, 0)).unwrap_or(0.0);
            let pl = c.query("select id, user, host, db, command, time, state, info from information_schema.processlist order by (command = 'Sleep'), time desc limit 500", 0).await?.rows;
            let waits_all: Vec<(u32, u32)> = match c.query("select waiting_pid, blocking_pid from sys.innodb_lock_waits", 0).await {
                    Ok(r) => r.rows.iter().map(|r| (n(r, 0) as u32, n(r, 1) as u32)).collect(),
                    Err(_) => c
                        .query("select r.trx_mysql_thread_id, b.trx_mysql_thread_id from information_schema.innodb_lock_waits w join information_schema.innodb_trx r on r.trx_id = w.requesting_trx_id join information_schema.innodb_trx b on b.trx_id = w.blocking_trx_id", 0)
                        .await
                        .map(|r| r.rows.iter().map(|r| (n(r, 0) as u32, n(r, 1) as u32)).collect())
                        .unwrap_or_default(),
                };
            let active = pl.iter().filter(|r| s(r, 4) != "Sleep" && s(r, 4) != "Daemon" && !own.contains(&(n(r, 0) as u32))).count() as u32;
            let waiting = pl.iter().filter(|r| s(r, 6).to_lowercase().contains("lock")).count() as u32;
            let waiting = waiting.max(waits_all.iter().map(|w| w.0).collect::<std::collections::HashSet<_>>().len() as u32);
            let reads = status.get("Innodb_buffer_pool_reads").copied().unwrap_or(0.0);
            let reqs = status.get("Innodb_buffer_pool_read_requests").copied().unwrap_or(0.0);
            let replicas = pl.iter().filter(|r| s(r, 4).starts_with("Binlog Dump")).count() as u32;
            let size = c.query("select coalesce(sum(data_length + index_length), 0) from information_schema.tables where table_schema = database()", 1).await?.rows.first().map(|r| n(r, 0)).unwrap_or(0.0);
            let tiles = ActivityTiles {
                connections: status.get("Threads_connected").copied().unwrap_or(pl.len() as f64) as u32,
                max_connections: max as u32,
                active,
                waiting,
                transactions_total: status.get("Com_commit").copied().unwrap_or(0.0) + status.get("Com_rollback").copied().unwrap_or(0.0),
                cache_hit_pct: (reqs > 0.0).then(|| (1.0 - reads / reqs) * 100.0),
                replicas,
                replication_lag_s: None,
                db_bytes: size,
            };
            let mut sessions = vec![];
            let mut table = None;
            if view == ActivityView::Sessions {
                // Who waits for whom: MySQL 8's sys view, or MariaDB's InnoDB tables; needs the PROCESS privilege, so
                // no answer just means no chains are shown.
                let waits = waits_all.clone();
                sessions = pl
                    .iter()
                    .filter(|r| s(r, 4) != "Daemon")
                    .map(|r| {
                        let id = n(r, 0) as u32;
                        let state = if s(r, 4) == "Sleep" { "idle".to_string() } else if matches!(s(r, 4).as_str(), "Query" | "Execute") { "active".to_string() } else { s(r, 4).to_lowercase() };
                        SessionRow {
                            pid: i32::try_from(id).unwrap_or(i32::MAX),
                            user: s(r, 1),
                            application: s(r, 2),
                            state,
                            age_s: n(r, 5),
                            wait: s(r, 6),
                            query: s(r, 7),
                            blocked_by: waits.iter().filter(|w| w.0 == id).map(|w| w.1 as i32).collect(),
                            mine: own.contains(&id),
                        }
                    })
                    .collect();
            } else {
                let (columns, rows): (&[&str], Vec<Vec<Option<String>>>) = match view {
                    ActivityView::Locks => (&["Transaction", "Type", "Relation", "Mode", "Granted"], match c.query("select ENGINE_TRANSACTION_ID, LOCK_TYPE, OBJECT_NAME, LOCK_MODE, LOCK_STATUS from performance_schema.data_locks limit 500", 0).await {
                        Ok(r) => r.rows,
                        Err(_) => c.query("select lock_trx_id, lock_type, lock_table, lock_mode, 'GRANTED' from information_schema.innodb_locks limit 500", 0).await.map(|r| r.rows).unwrap_or_default(),
                    }),
                    ActivityView::Slow => (&["PID", "User", "Running for", "Query"], pl.iter().filter(|r| s(r, 4) == "Query" && !own.contains(&(n(r, 0) as u32))).map(|r| vec![Some(s(r, 0)), Some(s(r, 1)), Some(format!("{} s", s(r, 5))), Some(s(r, 7))]).collect()),
                    ActivityView::Tables => (&["Table", "Rows", "Size", "Engine", "Updated"], c.query(&format!("select concat(table_schema, '.', table_name), table_rows, concat(round((data_length + index_length) / 1048576, 1), ' MB'), engine, coalesce(update_time, '') from information_schema.tables where table_type = 'BASE TABLE' and table_schema not in {SYSTEM} order by data_length + index_length desc limit 100"), 0).await?.rows),
                    ActivityView::Roles => (&["Account", "Privileges"], c.query("select grantee, count(*) from information_schema.user_privileges group by grantee order by 1", 0).await.map(|r| r.rows).unwrap_or_default()),
                    _ => {
                        // `show replica status` (8.0.22+) or `show slave status`; a source server has none.
                        let r = match c.query("show replica status", 0).await {
                            Ok(r) => Ok(r),
                            Err(_) => c.query("show slave status", 0).await,
                        };
                        let rows = r
                            .map(|r| {
                                let at = |names: &[&str]| r.columns.iter().position(|c| names.contains(&c.name.as_str()));
                                let host = at(&["Source_Host", "Master_Host"]);
                                let io = at(&["Replica_IO_Running", "Slave_IO_Running"]);
                                let sql = at(&["Replica_SQL_Running", "Slave_SQL_Running"]);
                                let lag = at(&["Seconds_Behind_Source", "Seconds_Behind_Master"]);
                                r.rows.iter().map(|row| vec![host.and_then(|i| row[i].clone()), io.and_then(|i| row[i].clone()), sql.and_then(|i| row[i].clone()), lag.and_then(|i| row[i].clone())]).collect()
                            })
                            .unwrap_or_default();
                        (&["Source", "IO running", "SQL running", "Seconds behind"], rows)
                    }
                };
                let w = columns.len();
                table = Some(ActivityTable { columns: columns.iter().map(|x| x.to_string()).collect(), rows: rows.iter().map(|r| (0..w).map(|i| s(r, i)).collect()).collect() });
            }
            let chains = chains(&sessions);
            Ok(Activity { tiles, sessions, chains, table })
        })
        .await
    }

    /// A session as the confirmation shows it, and who it belongs to (user and host) for the check at kill time.
    pub async fn session_info(&self, id: u32) -> AppResult<Option<(String, String, String, String)>> {
        if self.own.lock().map(|o| o.contains(&id)).unwrap_or(true) {
            return Ok(None);
        }
        let r = self.read_only(async |c| c.query(&format!("select user, command, left(coalesce(info, ''), 300), concat(user, '@', host) from information_schema.processlist where id = {id} and command <> 'Daemon' and host <> ''"), 1).await).await?;
        Ok(r.rows.first().map(|r| (s(r, 0), s(r, 1), s(r, 2), s(r, 3))))
    }

    /// `KILL QUERY` or `KILL` of someone else's connection. `owner` is the `user@host` the confirmation showed: a
    /// connection id that now belongs to someone else is left alone. `None`: no such session.
    pub async fn signal_session(&self, id: u32, end: bool, owner: &str) -> AppResult<Option<bool>> {
        let Some((_, _, _, now)) = self.session_info(id).await? else { return Ok(None) };
        if now != owner {
            return Ok(None);
        }
        let _scope = self.gate.lock().await;
        self.conn.query(&format!("KILL {}{id}", if end { "" } else { "QUERY " }), 0).await?;
        Ok(Some(true))
    }
}

// ---- Explain (S39)

/// One `EXPLAIN FORMAT=TREE` / `EXPLAIN ANALYZE` line's numbers: `(cost=.. rows=N)` and `(actual time=a..b rows=R loops=L)`.
fn parse_line(text: &str) -> (String, f64, Option<(f64, f64, f64)>) {
    let num = |hay: &str, key: &str| -> Option<f64> { hay.split(key).nth(1).map(|t| t.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == '+').collect::<String>()).and_then(|t| t.parse().ok()) };
    let name = text.split("  (cost=").next().unwrap_or(text).split(" (cost=").next().unwrap_or(text).trim().to_string();
    let first = text.split("(actual").next().unwrap_or(text);
    let est = num(first.split("(cost=").nth(1).unwrap_or(""), "rows=").unwrap_or(0.0);
    let actual = text.split("(actual time=").nth(1).and_then(|a| {
        let (time, rest) = a.split_once(" rows=")?;
        let total: f64 = time.split("..").nth(1)?.parse().ok()?;
        let (rows, loops) = rest.split_once(" loops=")?;
        Some((total, rows.parse().ok()?, loops.trim_end_matches(')').trim().parse().ok()?))
    });
    (name, est, actual)
}

/// The tree from the tool's indented lines (four spaces a level).
pub fn parse_tree(text: &str) -> AppResult<PlanNode> {
    let mut stack: Vec<(usize, PlanNode)> = vec![];
    let mut root: Option<PlanNode> = None;
    let close = |stack: &mut Vec<(usize, PlanNode)>, root: &mut Option<PlanNode>, level: usize| {
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            let (_, node) = stack.pop().unwrap_or_else(|| unreachable!());
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(node),
                // A second top-level node (the tool printed two trees) hangs under the first.
                None => match root {
                    Some(r) => r.children.push(node),
                    None => *root = Some(node),
                },
            }
        }
    };
    for line in text.lines().filter(|l| l.trim_start().starts_with("-> ")) {
        let indent = line.len() - line.trim_start().len();
        let level = indent / 4;
        let (name, est, actual) = parse_line(line.trim_start().trim_start_matches("-> "));
        close(&mut stack, &mut root, level);
        let total = actual.map(|(t, _, l)| t * l);
        let scan = name.strip_prefix("Table scan on ").map(|t| (String::new(), t.trim().to_string(), String::new()));
        stack.push((level, PlanNode { name, detail: None, est_rows: est, actual_rows: actual.map(|(_, r, l)| r * l), self_ms: None, pct: None, total_ms: total, children: vec![], loops: actual.map_or(1.0, |(_, _, l)| l), scan }));
    }
    close(&mut stack, &mut root, 0);
    root.ok_or_else(|| AppError::internal("The server's plan wasn't readable."))
}

fn shares(node: &mut PlanNode, whole: f64) {
    let kids: f64 = node.children.iter().filter_map(|c| c.total_ms).sum();
    node.self_ms = node.total_ms.map(|t| (t - kids).max(0.0));
    node.pct = node.self_ms.map(|t| if whole > 0.0 { t / whole * 100.0 } else { 0.0 });
    for c in &mut node.children {
        shares(c, whole);
    }
}

fn flat<'a>(n: &'a PlanNode, out: &mut Vec<&'a PlanNode>) {
    out.push(n);
    n.children.iter().for_each(|c| flat(c, out));
}

fn hint(root: &PlanNode, analyzed: bool) -> Option<PlanHint> {
    let mut all = vec![];
    flat(root, &mut all);
    let worst = all.iter().filter(|n| n.scan.is_some()).max_by(|a, b| a.self_ms.unwrap_or(a.est_rows).total_cmp(&b.self_ms.unwrap_or(b.est_rows)))?;
    let big = if analyzed { worst.pct.unwrap_or(0.0) >= 20.0 } else { worst.est_rows >= 10_000.0 };
    if !big {
        return None;
    }
    let table = worst.scan.as_ref()?.1.split_whitespace().next()?.to_string();
    let mut text = format!("The scan on {table} reads every row");
    if let (true, Some(p)) = (analyzed, worst.pct) {
        text.push_str(&format!(" and takes {p:.0}% of the time"));
    }
    text.push_str(". An index on the columns you filter by would let it skip the rest.");
    // The tree names the table as the query wrote it (often an alias), so no ready-made statement is offered.
    Some(PlanHint { text, actions: vec![] })
}

impl MyEditor {
    /// MySQL 8: `EXPLAIN FORMAT=TREE` and `EXPLAIN ANALYZE` (a tree of text). MariaDB: its classic table, one step per
    /// row (no timings: its `ANALYZE` answers in another shape that isn't read yet).
    pub async fn explain(&self, text: &str, analyze: bool, timeout_ms: u32) -> AppResult<ExplainPlan> {
        let _scope = self.gate.lock().await;
        let _ = self.conn.set_timeout(timeout_ms).await;
        let manual = self.has_txn();
        // EXPLAIN ANALYZE runs the statement: inside the person's own transaction it would run with their write rights.
        if manual && analyze {
            return Err(AppError::validation("Explain with timings runs the statement, so it isn't done inside a transaction. Commit or roll back first, or use plain Explain."));
        }
        if !manual {
            self.conn.query("START TRANSACTION READ ONLY", 0).await?;
        }
        let started = std::time::Instant::now();
        let out = match self.conn.flavour {
            Flavour::MySql => {
                let sql = if analyze { format!("EXPLAIN ANALYZE {text}") } else { format!("EXPLAIN FORMAT=TREE {text}") };
                self.conn.query(&sql, 0).await.and_then(|r| {
                    let tree = r.rows.iter().map(|r| s(r, 0)).collect::<Vec<_>>().join("\n");
                    parse_tree(&tree)
                })
            }
            Flavour::MariaDb => {
                if analyze {
                    Err(AppError::validation("Explain with timings isn't available for MariaDB yet. Plain Explain shows the plan."))
                } else {
                    self.conn.query(&format!("EXPLAIN {text}"), 0).await.map(|r| {
                        let at = |name: &str| r.columns.iter().position(|c| c.name == name);
                        let children = r
                            .rows
                            .iter()
                            .map(|row| {
                                let get = |name: &str| at(name).and_then(|i| row[i].clone()).unwrap_or_default();
                                let table = get("table");
                                let kind = get("type");
                                let label = match kind.as_str() {
                                    "ALL" => "Table scan",
                                    "index" => "Index scan",
                                    "range" => "Index range scan",
                                    "ref" | "ref_or_null" => "Index lookup",
                                    "eq_ref" => "Unique lookup",
                                    "const" | "system" => "Constant row",
                                    _ => "Step",
                                };
                                let key = get("key");
                                let extra = get("Extra");
                                PlanNode { name: format!("{label} on {table}"), detail: Some([if key.is_empty() { String::new() } else { format!("key: {key}") }, extra].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ")), est_rows: get("rows").parse().unwrap_or(0.0), actual_rows: None, self_ms: None, pct: None, total_ms: None, children: vec![], loops: 1.0, scan: (kind == "ALL").then(|| (String::new(), table, String::new())) }
                            })
                            .collect();
                        PlanNode { name: "Query".into(), detail: None, est_rows: 0.0, actual_rows: None, self_ms: None, pct: None, total_ms: None, children, loops: 1.0, scan: None }
                    })
                }
            }
        };
        if !manual {
            let _ = self.conn.query("ROLLBACK", 0).await;
        }
        let mut root = out?;
        let whole = root.total_ms.unwrap_or(0.0);
        if analyze {
            shares(&mut root, whole);
        }
        let h = hint(&root, analyze);
        Ok(ExplainPlan { analyzed: analyze, planning_ms: None, execution_ms: analyze.then(|| started.elapsed().as_secs_f64() * 1000.0), buffers_read: None, hint: h, root })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "-> Limit: 50 row(s)  (cost=100 rows=50) (actual time=0.5..80.0 rows=50 loops=1)\n    -> Nested loop inner join  (cost=90 rows=300) (actual time=0.4..79.0 rows=300 loops=1)\n        -> Table scan on o  (cost=60 rows=1200) (actual time=0.1..60.0 rows=30877 loops=1)\n        -> Single-row index lookup on c using PRIMARY (id=o.customer_id)  (cost=0.25 rows=1) (actual time=0.001..0.001 rows=1 loops=30877)\n";

    #[test]
    fn the_trees_lines_become_nodes_with_their_numbers() {
        let t = parse_tree(TREE).unwrap();
        assert_eq!(t.name, "Limit: 50 row(s)");
        assert_eq!((t.est_rows, t.actual_rows, t.total_ms), (50.0, Some(50.0), Some(80.0)));
        let join = &t.children[0];
        assert_eq!(join.children.len(), 2);
        let scan = &join.children[0];
        assert_eq!((scan.name.as_str(), scan.est_rows, scan.actual_rows), ("Table scan on o", 1200.0, Some(30877.0)));
        assert!(scan.scan.is_some());
        // Loops multiply the actual rows and the time of the inner lookup.
        assert_eq!(join.children[1].actual_rows, Some(30877.0));
        assert!(parse_tree("nothing here").is_err());
    }

    #[test]
    fn shares_and_the_hint_name_the_costly_scan() {
        let mut t = parse_tree(TREE).unwrap();
        let whole = t.total_ms.unwrap();
        shares(&mut t, whole);
        let scan = &t.children[0].children[0];
        assert!(scan.pct.unwrap() > 70.0);
        let h = hint(&t, true).unwrap();
        assert!(h.text.starts_with("The scan on o reads every row and takes"));
        assert!(h.actions.is_empty());
    }

    #[test]
    fn operations_become_mysql_statements_that_repeat_what_modify_would_lose() {
        use super::super::pg::{SqlColumn, SqlForeignKey};
        use super::super::values::CellKind;
        let info = SqlTableInfo {
            columns: vec![SqlColumn { name: "qty".into(), type_name: "int".into(), kind: CellKind::Int, nullable: false, default: Some("1".into()), pk_position: None }],
            references: vec![SqlForeignKey { name: "fk_o".into(), columns: vec!["o".into()], schema: "s".into(), table: "orders".into(), target_columns: vec!["id".into()] }],
            referenced_by: vec![],
        };
        let ops = vec![
            StructureOp::ChangeType { column: "qty".into(), type_name: "bigint".into() },
            StructureOp::SetNullable { column: "qty".into(), nullable: true },
            StructureOp::DropIndex { name: "i".into() },
            StructureOp::DropConstraint { name: "fk_o".into() },
            StructureOp::DropConstraint { name: "uq".into() },
            StructureOp::RenameColumn { from: "a".into(), to: "b".into() },
        ];
        let ddl = "CREATE TABLE `t` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  `qty` int NOT NULL DEFAULT '1' COMMENT 'how many',\n  `s` varchar(40) CHARACTER SET latin1 DEFAULT '2024-01-01',\n  PRIMARY KEY (`id`)\n)";
        let (s, w) = statements_my("shop", "t", &ops, Some(500), &info, ddl).unwrap();
        // The column's own line is kept: its default as the server prints it, its comment.
        assert_eq!(s[0], "alter table `shop`.`t` modify column `qty` bigint NOT NULL DEFAULT '1' COMMENT 'how many'");
        assert_eq!(s[1], "alter table `shop`.`t` modify column `qty` int DEFAULT '1' COMMENT 'how many'");
        assert_eq!(s[2], "drop index `i` on `shop`.`t`");
        assert_eq!(s[3], "alter table `shop`.`t` drop foreign key `fk_o`");
        assert_eq!(s[4], "alter table `shop`.`t` drop index `uq`");
        assert!(w[0].contains("can't undo them") && w.iter().any(|x| x.contains("rewrites the table (about 500 rows)")));
        assert!(statements_my("s", "t", &[StructureOp::ChangeType { column: "nope".into(), type_name: "int".into() }], None, &info, ddl).is_err());
        assert!(statements_my("s", "t", &[StructureOp::SetDefault { column: "qty".into(), default: Some("1; drop table t".into()) }], None, &info, ddl).is_err());
        // A column whose line can't be found is not guessed at.
        assert!(statements_my("s", "t", &[StructureOp::ChangeType { column: "qty".into(), type_name: "int".into() }], None, &info, "CREATE TABLE `t` (\n  `other` int\n)").is_err());
    }
}
