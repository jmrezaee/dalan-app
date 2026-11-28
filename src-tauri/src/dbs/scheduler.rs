//! The job scheduler (D15, E2; replica/databases/restore-copy.md §9).
//!
//! A tokio task started when the vault unlocks. It wakes every 20 seconds and runs the scheduled jobs that are due,
//! one at a time, and ends at the first wake after the vault locked (or when a newer unlock started another one).
//! Jobs run only while Dalan is open and unlocked, because the passwords come from the vault. Slots that passed while
//! Dalan was closed or locked are not run one by one: the job runs once, and a "missed" run records how many there
//! were. A run never asks anything: an unknown host key or an unsaved password fails it with the reason.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;

use crate::db::now_ms;
use crate::dbs::jobs::{self, Due, JobKind, RunRecord};
use crate::dbs::schedule;
use crate::error::{AppError, AppResult};
use crate::events::JobsChanged;
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(20);
/// A slot this much in the past counts as missed (the app was closed or locked), not just due.
const LATE_MS: i64 = 2 * 60 * 1000;

/// Starts the scheduler for this unlock. A loop from an earlier unlock sees the newer generation and ends.
pub fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    let generation = state.scheduler_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // A run that was going when Dalan last quit is recorded as stopped; its slot isn't done, so it runs again.
        {
            let st = app.state::<AppState>();
            if let Ok(ids) = st
                .with_db_session(|c, s| jobs::record_interrupted(c, s))
                .await
            {
                for job_id in ids {
                    let _ = JobsChanged {
                        job_id,
                        running: false,
                        failed: true,
                    }
                    .emit(&app);
                }
            }
        }
        loop {
            {
                let st = app.state::<AppState>();
                if st.scheduler_gen.load(Ordering::SeqCst) != generation || !st.is_unlocked() {
                    break;
                }
            }
            tick(&app).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

/// One pass: every scheduled job with a slot in `(done_until, now]` runs once.
pub async fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(due) = state.with_db_session(|c, s| jobs::scheduled(c, s)).await else {
        return;
    };
    for d in due {
        let Ok(cron) = d.stored.schedule.every.cron() else {
            continue;
        };
        let now = now_ms();
        let (n, last) = schedule::slots_in(&cron, &chrono::Local, d.stored.done_until, now, 1000);
        let Some(last) = last else { continue };
        if !state.is_unlocked() {
            return;
        }
        let missed = missed(n, last, now);
        if missed > 0 {
            record(
                &state,
                &d.id,
                now,
                "missed",
                i64::from(missed),
                format!(
                    "Missed {missed} {} while Dalan was closed or locked; ran once at unlock.",
                    if missed == 1 { "run" } else { "runs" }
                ),
            )
            .await;
        }
        let _ = JobsChanged {
            job_id: d.id.clone(),
            running: true,
            failed: false,
        }
        .emit(app);
        let started = now_ms();
        // Marked in the database for the length of the run (found at the next unlock if Dalan quits first), and
        // counted so quitting asks first.
        let id = d.id.clone();
        let _ = state
            .with_db(move |c| jobs::mark_running(c, &id, Some(started)))
            .await;
        state
            .running_jobs
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = run(app, &d).await;
        state
            .running_jobs
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        let id = d.id.clone();
        let _ = state
            .with_db(move |c| jobs::mark_running(c, &id, None))
            .await;
        // Only after the run: a run cut short by locking is due again at the next unlock.
        let id = d.id.clone();
        let marked = state
            .with_db_session(move |c, _| jobs::done_until(c, &id, now))
            .await;
        let failed = match &result {
            Ok(Outcome::Recorded { ok }) => !ok,
            Ok(Outcome::Backup(done)) => {
                record(&state, &d.id, started, "ok", 0, done.log.join("\n")).await;
                false
            }
            Err(e) => {
                if marked.is_ok() {
                    record(
                        &state,
                        &d.id,
                        started,
                        "failed",
                        0,
                        crate::dbs::runner::failure_text(e),
                    )
                    .await;
                }
                true
            }
        };
        let _ = JobsChanged {
            job_id: d.id.clone(),
            running: false,
            failed,
        }
        .emit(app);
        if failed || d.stored.schedule.notify_ok {
            let body = match &result {
                Err(e) => crate::dbs::runner::failure_text(e),
                Ok(Outcome::Recorded { ok: false }) => {
                    "Some of it failed. See the job's history.".into()
                }
                Ok(_) => "Done.".into(),
            };
            let _ = app
                .notification()
                .builder()
                .title(if failed {
                    format!("The job “{}” failed", d.name)
                } else {
                    format!("The job “{}” ran", d.name)
                })
                .body(body)
                .show();
        }
    }
}

/// How many slots count as missed: all of them when even the last is late (Dalan was closed or locked), otherwise
/// all but the one that is just due.
fn missed(slots: u32, last: i64, now: i64) -> u32 {
    if now - last > LATE_MS {
        slots
    } else {
        slots.saturating_sub(1)
    }
}

enum Outcome {
    /// A backup that went well (the scheduler records it).
    Backup(crate::dbs::runner::BackupDone),
    /// A copy, recorded by the copy itself.
    Recorded { ok: bool },
}

async fn record(
    state: &AppState,
    job_id: &str,
    started: i64,
    status: &'static str,
    warnings: i64,
    log: String,
) {
    let id = job_id.to_string();
    let _ = state
        .with_db_session(move |c, s| {
            jobs::record_run(
                c,
                s,
                &id,
                RunRecord {
                    started_at: started,
                    status,
                    rows: None,
                    bytes: None,
                    warnings,
                    log,
                },
            )
        })
        .await;
}

/// The databases a job reads and writes, opened with no window; closed again when the run ends (also on errors).
struct Opened<'a> {
    state: &'a AppState,
    ids: Vec<String>,
}

impl Opened<'_> {
    async fn open(&mut self, db_id: &str) -> AppResult<String> {
        let id = self
            .state
            .sql
            .open_headless(self.state.store(), db_id.to_string())
            .await?;
        self.ids.push(id.clone());
        Ok(id)
    }
}

impl Drop for Opened<'_> {
    fn drop(&mut self) {
        for id in &self.ids {
            self.state.sql.close(id);
        }
    }
}

async fn run(app: &AppHandle, d: &Due) -> AppResult<Outcome> {
    let state = app.state::<AppState>();
    let (source, target) = {
        let id = d.id.clone();
        state
            .with_db_session(move |c, s| {
                Ok(jobs::list(c, s)?
                    .into_iter()
                    .find(|j| j.id == id)
                    .map(|j| (j.source_database_id, j.target_database_id)))
            })
            .await?
            .ok_or_else(|| AppError::not_found("That job is no longer saved."))?
    };
    let mut opened = Opened {
        state: &state,
        ids: Vec::new(),
    };
    match d.kind {
        JobKind::Backup => {
            let db = source.ok_or_else(|| AppError::validation("This job has no database."))?;
            let conn = opened.open(&db).await?;
            let transfer = uuid::Uuid::now_v7().to_string();
            let done =
                crate::dbs::runner::backup_into(&state, &conn, &d.id, &transfer, &|_| {}).await?;
            Ok(Outcome::Backup(done))
        }
        JobKind::Copy => {
            // Checked when the schedule was set, and again now: the target may have become production since.
            if let Some(why) = jobs::no_schedule(d.kind, d.target_env.as_deref()) {
                return Err(AppError::validation(why));
            }
            let (src, dst) = match (source, target) {
                (Some(s), Some(t)) => (s, t),
                _ => return Err(AppError::validation("This job has lost a database.")),
            };
            let id = d.id.clone();
            let (_, plan) = state
                .with_db_session(move |c, s| jobs::plan(c, s, &id))
                .await?;
            let mut req: crate::sql::copy::CopyRequest = plan
                .get("request")
                .and_then(|r| serde_json::from_value(r.clone()).ok())
                .ok_or_else(|| AppError::internal("The job's copy can't be read."))?;
            let src_conn = opened.open(&src).await?;
            let dst_conn = opened.open(&dst).await?;
            req.target_conn_id = dst_conn;
            let plan = crate::commands::copy::copy_prepare(
                app.state::<AppState>(),
                src_conn.clone(),
                req,
                Some(d.id.clone()),
            )
            .await?;
            if let Some(why) = plan.blocked {
                return Err(AppError::validation(why));
            }
            let pending = plan
                .pending_id
                .ok_or_else(|| AppError::internal("The copy wasn't held."))?;
            // The schedule is the confirmation; a typed name is only ever asked of a non-production target here.
            let typed = (plan.level == "typed").then_some(plan.typed_name);
            let report = crate::commands::copy::copy_commit(
                app.state::<AppState>(),
                src_conn,
                pending,
                typed,
                uuid::Uuid::now_v7().to_string(),
                tauri::ipc::Channel::new(|_| Ok(())),
            )
            .await?;
            Ok(Outcome::Recorded {
                ok: report.outcome == "done",
            })
        }
        JobKind::Restore => Err(AppError::validation(
            "Restores run only when you start them.",
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_due_slot_is_not_missed_but_late_ones_are() {
        let now = 10 * 60 * 60 * 1000;
        assert_eq!(super::missed(1, now - 20_000, now), 0);
        assert_eq!(super::missed(3, now - 20_000, now), 2);
        // Locked from 06:30 to 09:12 with an hourly job: 07:00, 08:00 and 09:00 were all missed; it runs once.
        assert_eq!(super::missed(3, now - 12 * 60 * 1000, now), 3);
    }
}
