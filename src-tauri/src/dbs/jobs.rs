//! Saved jobs (D14, E1; replica/databases/restore-copy.md §9): a restore, copy or backup set up once and run again by
//! name. The plan is JSON sealed with the vault's data key (it can hold filters, masking rules and their key, and a
//! file or folder path); it never holds a password. Running a job builds its confirmation fresh from the current
//! state, through the same gate as the first run: nothing approved earlier carries over. Each run is kept (the last
//! 50 per job) with its log, sealed too.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::history::{open_command, seal_command};
use crate::vault::VaultSession;

const KEEP_RUNS: i64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Backup,
    Restore,
    Copy,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            JobKind::Backup => "backup",
            JobKind::Restore => "restore",
            JobKind::Copy => "copy",
        }
    }
    fn parse(s: &str) -> Option<JobKind> {
        Some(match s {
            "backup" => JobKind::Backup,
            "restore" => JobKind::Restore,
            "copy" => JobKind::Copy,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct JobRun {
    pub id: String,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    /// `ok`, `failed`, `stopped` or `missed`.
    pub status: String,
    pub rows: Option<f64>,
    pub bytes: Option<f64>,
    pub warnings: f64,
    pub log: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct JobRow {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    pub source_database_id: Option<String>,
    pub source_name: Option<String>,
    pub target_database_id: Option<String>,
    pub target_name: Option<String>,
    /// What it does, in words ("3 tables from shop to staging, masked"), from the plan.
    pub summary: String,
    pub created_at: f64,
    pub last_run: Option<JobRun>,
}

/// What is sealed in `plan_enc`: the kind's own plan, and the summary shown in the list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sealed {
    pub summary: String,
    pub plan: serde_json::Value,
}

pub struct NewJob {
    pub name: String,
    pub kind: JobKind,
    pub source: Option<String>,
    pub target: Option<String>,
    pub summary: String,
    pub plan: serde_json::Value,
}

fn seal(s: &VaultSession, id: &str, sealed: &Sealed) -> AppResult<Vec<u8>> {
    seal_command(
        s,
        id,
        &serde_json::to_string(sealed).map_err(|e| AppError::internal(e.to_string()))?,
    )
}

fn unseal(s: &VaultSession, id: &str, blob: &[u8]) -> Option<Sealed> {
    serde_json::from_str(&open_command(s, id, blob)?).ok()
}

fn clean_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 80 {
        return Err(AppError::validation(
            "Give the job a name of up to 80 characters.",
        ));
    }
    Ok(n.to_string())
}

pub fn save(conn: &Connection, s: &VaultSession, job: NewJob) -> AppResult<JobRow> {
    let id = uuid::Uuid::now_v7().to_string();
    let name = clean_name(&job.name)?;
    let taken: bool = conn
        .query_row(
            "SELECT 1 FROM db_jobs WHERE vault_id = ?1 AND name = ?2",
            params![s.vault_id, name],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if taken {
        return Err(AppError::validation(format!(
            "A job named “{name}” already exists. Pick another name."
        )));
    }
    let blob = seal(
        s,
        &id,
        &Sealed {
            summary: job.summary,
            plan: job.plan,
        },
    )?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO db_jobs (id, vault_id, name, kind, source_database_id, target_database_id, plan_enc, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![id, s.vault_id, name, job.kind.as_str(), job.source, job.target, blob, now],
    )?;
    list(conn, s)?
        .into_iter()
        .find(|j| j.id == id)
        .ok_or_else(|| AppError::internal("the job wasn't saved"))
}

fn run_row(s: &VaultSession, r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRun> {
    let id: String = r.get(0)?;
    let log: Option<Vec<u8>> = r.get(7)?;
    Ok(JobRun {
        log: log
            .and_then(|b| open_command(s, &id, &b))
            .unwrap_or_default(),
        id,
        started_at: r.get::<_, i64>(1)? as f64,
        ended_at: r.get::<_, Option<i64>>(2)?.map(|x| x as f64),
        status: r.get(3)?,
        rows: r.get::<_, Option<i64>>(4)?.map(|x| x as f64),
        bytes: r.get::<_, Option<i64>>(5)?.map(|x| x as f64),
        warnings: r.get::<_, i64>(6)? as f64,
    })
}

pub fn runs(conn: &Connection, s: &VaultSession, job_id: &str) -> AppResult<Vec<JobRun>> {
    let mut st = conn.prepare(
        "SELECT r.id, r.started_at, r.ended_at, r.status, r.rows, r.bytes, r.warnings, r.log_enc FROM db_job_runs r JOIN db_jobs j ON j.id = r.job_id
         WHERE r.job_id = ?1 AND j.vault_id = ?2 ORDER BY r.started_at DESC, r.rowid DESC",
    )?;
    let rows = st.query_map(params![job_id, s.vault_id], |r| run_row(s, r))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// One db_jobs row joined with its databases' names, before the plan is opened.
type ListRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<u8>,
    i64,
);

pub fn list(conn: &Connection, s: &VaultSession) -> AppResult<Vec<JobRow>> {
    let mut st = conn.prepare(
        "SELECT j.id, j.name, j.kind, j.source_database_id, sd.name, j.target_database_id, td.name, j.plan_enc, j.created_at
         FROM db_jobs j LEFT JOIN databases sd ON sd.id = j.source_database_id LEFT JOIN databases td ON td.id = j.target_database_id
         WHERE j.vault_id = ?1 ORDER BY j.name",
    )?;
    let rows: Vec<ListRow> = st
        .query_map([&s.vault_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, name, kind, sid, sname, tid, tname, blob, created) in rows {
        let Some(kind) = JobKind::parse(&kind) else {
            continue;
        };
        let last = runs(conn, s, &id)?.into_iter().next();
        out.push(JobRow {
            summary: unseal(s, &id, &blob)
                .map(|x| x.summary)
                .unwrap_or_else(|| "Its plan can't be read with this vault's key.".into()),
            id,
            name,
            kind,
            source_database_id: sid,
            source_name: sname,
            target_database_id: tid,
            target_name: tname,
            created_at: created as f64,
            last_run: last,
        });
    }
    Ok(out)
}

/// The job's kind and plan, for running it.
pub fn plan(
    conn: &Connection,
    s: &VaultSession,
    id: &str,
) -> AppResult<(JobKind, serde_json::Value)> {
    let (kind, blob): (String, Vec<u8>) = conn
        .query_row(
            "SELECT kind, plan_enc FROM db_jobs WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That job is no longer saved."))?;
    let kind = JobKind::parse(&kind).ok_or_else(|| AppError::internal("unknown job kind"))?;
    let sealed = unseal(s, id, &blob)
        .ok_or_else(|| AppError::internal("The job's plan can't be read with this vault's key."))?;
    Ok((kind, sealed.plan))
}

pub fn rename(conn: &Connection, s: &VaultSession, id: &str, name: &str) -> AppResult<()> {
    let name = clean_name(name)?;
    let n = conn.execute(
        "UPDATE db_jobs SET name = ?1, updated_at = ?2 WHERE id = ?3 AND vault_id = ?4",
        params![name, now_ms(), id, s.vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That job is no longer saved."));
    }
    Ok(())
}

pub fn delete(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM db_jobs WHERE id = ?1 AND vault_id = ?2",
        params![id, s.vault_id],
    )?;
    Ok(())
}

pub struct RunRecord {
    pub started_at: i64,
    pub status: &'static str,
    pub rows: Option<i64>,
    pub bytes: Option<i64>,
    pub warnings: i64,
    pub log: String,
}

/// Records a run and keeps the last 50.
pub fn record_run(
    conn: &Connection,
    s: &VaultSession,
    job_id: &str,
    r: RunRecord,
) -> AppResult<()> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM db_jobs WHERE id = ?1 AND vault_id = ?2",
            params![job_id, s.vault_id],
            |r| r.get(0),
        )
        .optional()?;
    if found.is_none() {
        return Ok(());
    }
    let id = uuid::Uuid::now_v7().to_string();
    // The log is capped: a run with thousands of warnings keeps its first 64 KB.
    let log: String = r.log.chars().take(64 * 1024).collect();
    let now = now_ms();
    conn.execute(
        "INSERT INTO db_job_runs (id, job_id, started_at, ended_at, status, rows, bytes, warnings, log_enc) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, job_id, r.started_at, now, r.status, r.rows, r.bytes, r.warnings, seal_command(s, &id, &log)?],
    )?;
    conn.execute(
        "UPDATE db_jobs SET last_run_at = ?1 WHERE id = ?2",
        params![now, job_id],
    )?;
    conn.execute(
        "DELETE FROM db_job_runs WHERE job_id = ?1 AND rowid NOT IN (SELECT rowid FROM db_job_runs WHERE job_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT ?2)",
        params![job_id, KEEP_RUNS],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_is_sealed_listed_run_and_refuses_to_lose_its_database() {
        let mut conn = crate::db::open_in_memory().unwrap();
        let s = crate::vault::create(
            &conn,
            "correct-horse-staple",
            crate::vault::kdf::KdfParams::cheap(),
        )
        .unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO databases (id, vault_id, engine, name, route, address, created_at, updated_at) VALUES ('d1', '{0}', 'postgres', 'shop', 'direct', 'localhost', 1, 1), ('d2', '{0}', 'postgres', 'staging', 'direct', 'localhost', 1, 1);",
            s.vault_id
        ))
        .unwrap();
        let job = save(
            &conn,
            &s,
            NewJob {
                name: "Nightly refresh".into(),
                kind: JobKind::Copy,
                source: Some("d1".into()),
                target: Some("d2".into()),
                summary: "2 tables from shop to staging".into(),
                plan: serde_json::json!({ "filter": "id < 10", "key": "secret" }),
            },
        )
        .unwrap();
        assert_eq!(
            (job.source_name.as_deref(), job.target_name.as_deref()),
            (Some("shop"), Some("staging"))
        );
        // The plan is sealed: its text isn't in the database file.
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT plan_enc FROM db_jobs WHERE id = ?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("id < 10"));
        assert_eq!(plan(&conn, &s, &job.id).unwrap().1["filter"], "id < 10");
        assert!(
            save(
                &conn,
                &s,
                NewJob {
                    name: " Nightly refresh ".into(),
                    kind: JobKind::Copy,
                    source: None,
                    target: None,
                    summary: String::new(),
                    plan: serde_json::json!({})
                }
            )
            .is_err(),
            "names are unique"
        );
        for i in 0..55 {
            record_run(
                &conn,
                &s,
                &job.id,
                RunRecord {
                    started_at: i,
                    status: "ok",
                    rows: Some(i),
                    bytes: None,
                    warnings: 0,
                    log: format!("run {i}"),
                },
            )
            .unwrap();
        }
        let r = runs(&conn, &s, &job.id).unwrap();
        assert_eq!(r.len(), 50);
        assert_eq!(r[0].log, "run 54");
        assert_eq!(
            list(&conn, &s).unwrap()[0].last_run.as_ref().unwrap().rows,
            Some(54.0)
        );
        let e = crate::dbs::store::delete(&mut conn, &s, "d2").unwrap_err();
        assert!(e.message.contains("Nightly refresh"), "{}", e.message);
        rename(&conn, &s, &job.id, "Weekly").unwrap();
        delete(&conn, &s, &job.id).unwrap();
        crate::dbs::store::delete(&mut conn, &s, "d2").unwrap();
        assert!(list(&conn, &s).unwrap().is_empty());
    }
}
