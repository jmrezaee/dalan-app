//! Saved jobs (D14, E1; replica/databases/restore-copy.md §9): a restore, copy or backup set up once and run again by
//! name. The plan is JSON sealed with the vault's data key (it can hold filters, masking rules and their key, and a
//! file or folder path); it never holds a password. Running a job builds its confirmation fresh from the current
//! state, through the same gate as the first run: nothing approved earlier carries over. Each run is kept (the last
//! 50 per job) with its log, sealed too.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::dbs::schedule::{self, Retention, Schedule, Stored};
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
    /// D15: the schedule (backup jobs, and copy jobs whose target isn't production), in words, and the next run.
    pub schedule: Option<Schedule>,
    pub schedule_text: Option<String>,
    pub next_run_at: Option<f64>,
    pub retention: Option<Retention>,
    /// Why this job can't have a schedule (`None` when it can).
    pub no_schedule: Option<String>,
    /// Slots missed while Dalan was closed or locked, when the last run made up for them.
    pub missed_before_last: Option<f64>,
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
    /// D15: where a backup goes, when it is a host (`{"host_id": …}`), so deleting that host is refused.
    pub destination: Option<serde_json::Value>,
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
        "INSERT INTO db_jobs (id, vault_id, name, kind, source_database_id, target_database_id, plan_enc, destination, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![id, s.vault_id, name, job.kind.as_str(), job.source, job.target, blob, job.destination.map(|d| d.to_string()), now],
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
    (String, String, String),
    (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    (Vec<u8>, i64),
    (Option<String>, Option<String>, Option<String>),
);

pub fn list(conn: &Connection, s: &VaultSession) -> AppResult<Vec<JobRow>> {
    let mut st = conn.prepare(
        "SELECT j.id, j.name, j.kind, j.source_database_id, sd.name, j.target_database_id, td.name, j.plan_enc, j.created_at,
                td.environment, j.schedule, j.retention
         FROM db_jobs j LEFT JOIN databases sd ON sd.id = j.source_database_id LEFT JOIN databases td ON td.id = j.target_database_id
         WHERE j.vault_id = ?1 ORDER BY j.name",
    )?;
    let rows: Vec<ListRow> = st
        .query_map([&s.vault_id], |r| {
            Ok((
                (r.get(0)?, r.get(1)?, r.get(2)?),
                (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?),
                (r.get(7)?, r.get(8)?),
                (r.get(9)?, r.get(10)?, r.get(11)?),
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for ((id, name, kind), (sid, sname, tid, tname), (blob, created), (env, sched, keep)) in rows {
        let Some(kind) = JobKind::parse(&kind) else {
            continue;
        };
        let recent = runs(conn, s, &id)?;
        let last = recent.first().cloned();
        let missed_before_last = match (recent.first(), recent.get(1)) {
            (Some(a), Some(b)) if a.status != "missed" && b.status == "missed" => Some(b.warnings),
            _ => None,
        };
        let stored: Option<Stored> = sched.and_then(|j| serde_json::from_str(&j).ok());
        let next_run_at = stored.as_ref().and_then(|st| {
            let cron = st.schedule.every.cron().ok()?;
            schedule::next(&cron, st.done_until.max(now_ms())).map(|x| x as f64)
        });
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
            schedule_text: stored.as_ref().map(|st| st.schedule.every.describe()),
            schedule: stored.map(|st| st.schedule),
            next_run_at,
            retention: keep.and_then(|j| serde_json::from_str(&j).ok()),
            no_schedule: no_schedule(kind, env.as_deref()),
            missed_before_last,
        });
    }
    Ok(out)
}

/// Why a job of this kind, with a target in this environment, can't be scheduled.
pub fn no_schedule(kind: JobKind, target_env: Option<&str>) -> Option<String> {
    match kind {
        JobKind::Restore => Some("Restores run only when you start them.".into()),
        JobKind::Copy if target_env == Some("production") => Some(
            "Its target is production, which always asks for the typed name: it can't run on a schedule.".into(),
        ),
        _ => None,
    }
}

/// Sets or clears a job's schedule and retention. A new schedule counts its slots from now.
pub fn set_schedule(
    conn: &Connection,
    s: &VaultSession,
    id: &str,
    sched: Option<Schedule>,
    retention: Option<Retention>,
) -> AppResult<()> {
    let (kind, env): (String, Option<String>) = conn
        .query_row(
            "SELECT j.kind, td.environment FROM db_jobs j LEFT JOIN databases td ON td.id = j.target_database_id
             WHERE j.id = ?1 AND j.vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That job is no longer saved."))?;
    let kind = JobKind::parse(&kind).ok_or_else(|| AppError::internal("unknown job kind"))?;
    let stored = match sched {
        Some(sc) => {
            if let Some(why) = no_schedule(kind, env.as_deref()) {
                return Err(AppError::validation(why));
            }
            sc.every.cron()?;
            Some(
                serde_json::to_string(&Stored {
                    schedule: sc,
                    done_until: now_ms(),
                })
                .map_err(|e| AppError::internal(e.to_string()))?,
            )
        }
        None => None,
    };
    let retention = retention
        .filter(|r| kind == JobKind::Backup && (r.keep_last.is_some() || r.keep_days.is_some()))
        .map(|r| serde_json::to_string(&r).unwrap_or_default());
    conn.execute(
        "UPDATE db_jobs SET schedule = ?1, retention = ?2, updated_at = ?3 WHERE id = ?4 AND vault_id = ?5",
        params![stored, retention, now_ms(), id, s.vault_id],
    )?;
    Ok(())
}

/// A scheduled job, as the scheduler sees it.
pub struct Due {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    pub stored: Stored,
    pub target_env: Option<String>,
}

pub fn scheduled(conn: &Connection, s: &VaultSession) -> AppResult<Vec<Due>> {
    let mut st = conn.prepare(
        "SELECT j.id, j.name, j.kind, j.schedule, td.environment FROM db_jobs j LEFT JOIN databases td ON td.id = j.target_database_id
         WHERE j.vault_id = ?1 AND j.schedule IS NOT NULL ORDER BY j.name",
    )?;
    let rows = st.query_map([&s.vault_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, kind, sched, target_env) = row?;
        if let (Some(kind), Ok(stored)) = (JobKind::parse(&kind), serde_json::from_str(&sched)) {
            out.push(Due {
                id,
                name,
                kind,
                stored,
                target_env,
            });
        }
    }
    Ok(out)
}

/// Marks a job's slots handled up to `until` (the schedule itself is left as it is).
pub fn done_until(conn: &Connection, id: &str, until: i64) -> AppResult<()> {
    let sched: Option<String> = conn
        .query_row("SELECT schedule FROM db_jobs WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .flatten();
    if let Some(mut st) = sched.and_then(|j| serde_json::from_str::<Stored>(&j).ok()) {
        st.done_until = until;
        conn.execute(
            "UPDATE db_jobs SET schedule = ?1 WHERE id = ?2",
            params![serde_json::to_string(&st).unwrap_or_default(), id],
        )?;
    }
    Ok(())
}

/// The job's retention (backups only).
pub fn retention(conn: &Connection, id: &str) -> AppResult<Retention> {
    let r: Option<String> = conn
        .query_row("SELECT retention FROM db_jobs WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .flatten();
    Ok(r.and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

/// The saved jobs that back up to this host (for refusing to delete it).
pub fn names_using_host(conn: &Connection, host_id: &str) -> AppResult<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT name FROM db_jobs WHERE json_extract(destination, '$.host_id') = ?1 ORDER BY name",
    )?;
    let rows = st.query_map([host_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
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
                destination: None,
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
                    plan: serde_json::json!({}),
                    destination: None
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

    #[test]
    fn schedules_only_on_backups_and_copies_into_non_production() {
        let conn = crate::db::open_in_memory().unwrap();
        let s = crate::vault::create(
            &conn,
            "correct-horse-staple",
            crate::vault::kdf::KdfParams::cheap(),
        )
        .unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO databases (id, vault_id, engine, name, environment, route, address, created_at, updated_at) VALUES
             ('d1', '{0}', 'postgres', 'shop', 'none', 'direct', 'localhost', 1, 1),
             ('d2', '{0}', 'postgres', 'live', 'production', 'direct', 'localhost', 1, 1),
             ('d3', '{0}', 'postgres', 'staging', 'staging', 'direct', 'localhost', 1, 1);",
            s.vault_id
        ))
        .unwrap();
        let job = |name: &str, kind, source: Option<&str>, target: Option<&str>| {
            save(
                &conn,
                &s,
                NewJob {
                    name: name.into(),
                    kind,
                    source: source.map(String::from),
                    target: target.map(String::from),
                    summary: name.into(),
                    plan: serde_json::json!({}),
                    destination: None,
                },
            )
            .unwrap()
        };
        let daily = Some(Schedule {
            every: schedule::Every::Day { hour: 2, minute: 0 },
            notify_ok: false,
        });
        let keep = Some(Retention {
            keep_last: Some(3),
            keep_days: None,
        });
        let b = job("b", JobKind::Backup, Some("d1"), None);
        set_schedule(&conn, &s, &b.id, daily.clone(), keep.clone()).unwrap();
        let row = list(&conn, &s)
            .unwrap()
            .into_iter()
            .find(|j| j.id == b.id)
            .unwrap();
        assert_eq!(row.schedule_text.as_deref(), Some("Every day at 02:00"));
        assert!(row.next_run_at.is_some());
        assert_eq!(row.retention, keep);
        assert_eq!(scheduled(&conn, &s).unwrap().len(), 1);
        let to_live = job("to live", JobKind::Copy, Some("d1"), Some("d2"));
        let err = set_schedule(&conn, &s, &to_live.id, daily.clone(), None).unwrap_err();
        assert!(err.message.contains("production"), "{}", err.message);
        let to_staging = job("to staging", JobKind::Copy, Some("d1"), Some("d3"));
        // A copy keeps no files: retention is dropped.
        set_schedule(&conn, &s, &to_staging.id, daily.clone(), keep).unwrap();
        let row = list(&conn, &s)
            .unwrap()
            .into_iter()
            .find(|j| j.id == to_staging.id)
            .unwrap();
        assert!(row.schedule.is_some() && row.retention.is_none());
        let r = job("r", JobKind::Restore, None, Some("d1"));
        assert!(set_schedule(&conn, &s, &r.id, daily.clone(), None).is_err());
        let bad = Some(Schedule {
            every: schedule::Every::Cron {
                expr: "61 * * * *".into(),
            },
            notify_ok: false,
        });
        assert!(set_schedule(&conn, &s, &b.id, bad, None).is_err());
        set_schedule(&conn, &s, &b.id, None, None).unwrap();
        assert_eq!(
            scheduled(&conn, &s).unwrap().len(),
            1,
            "only the staging copy is left"
        );
        // The missed-runs line: a `missed` run right before the last one.
        record_run(
            &conn,
            &s,
            &to_staging.id,
            RunRecord {
                started_at: 1,
                status: "missed",
                rows: None,
                bytes: None,
                warnings: 3,
                log: String::new(),
            },
        )
        .unwrap();
        record_run(
            &conn,
            &s,
            &to_staging.id,
            RunRecord {
                started_at: 2,
                status: "ok",
                rows: None,
                bytes: None,
                warnings: 0,
                log: String::new(),
            },
        )
        .unwrap();
        let row = list(&conn, &s)
            .unwrap()
            .into_iter()
            .find(|j| j.id == to_staging.id)
            .unwrap();
        assert_eq!(row.missed_before_last, Some(3.0));
    }
}
