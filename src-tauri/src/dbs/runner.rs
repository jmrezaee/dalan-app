//! Running a saved backup into its folder (D14, D15; replica/databases/restore-copy.md §9, E2 and E8).
//!
//! The folder is on this computer or on a saved host. Either way the dump tool's output is streamed into
//! `<name>.part` through the same file-pane code the SFTP view uses (`sftp::Fs`), and renamed to its final name only
//! when the tool finished well, so a cut-off backup never looks complete. Retention then removes this job's older
//! files: only names that match the job's own pattern, only in its own folder.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::dbs::jobs;
use crate::dbs::schedule::{self, Retention};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::sftp::{self, EntryKind, Fs};
use crate::ssh::{Prompts, SessionEvent, SessionIo};
use crate::state::{AppState, Store};

const CHUNK: usize = 64 * 1024;

/// A plain `Write` (what the dump tool's runner writes to) that hands 64 KB chunks to an async writer task. The
/// channel holds 16 chunks: when the destination is slower than the tool, `write` waits, so memory stays at ~1 MB.
struct Pipe {
    tx: Option<mpsc::Sender<Vec<u8>>>,
    buf: Vec<u8>,
}

impl Pipe {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK));
        let tx = self.tx.as_ref().ok_or(std::io::ErrorKind::BrokenPipe)?;
        // `write` is called from async code, but this trait is blocking. `block_in_place` tells tokio this worker
        // may wait here, so the writer task (on another worker) keeps running and frees room in the channel.
        tokio::task::block_in_place(|| tx.blocking_send(chunk)).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the destination stopped taking data",
            )
        })
    }

    /// Sends what is left and closes the channel, so the writer task finishes.
    fn close(mut self) -> std::io::Result<()> {
        self.send()?;
        self.tx = None;
        Ok(())
    }
}

impl Write for Pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.send()?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

/// Opens an SFTP session on a saved host without asking anything: the host key must already be trusted and the
/// password or key saved. Used by saved and scheduled jobs.
pub async fn open_host(store: &Store, host_id: &str) -> AppResult<sftp::RemoteFs> {
    let said: Arc<Mutex<Option<String>>> = Arc::default();
    let io = {
        let said = Arc::clone(&said);
        Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::Error { message, .. } = e {
                    if let Ok(mut s) = said.lock() {
                        *s = Some(message);
                    }
                }
            }),
            data: Box::new(|_| {}),
        })
    };
    let prompts = Arc::new(Prompts::non_interactive());
    sftp::open_remote(store, &io, &prompts, host_id)
        .await
        .map_err(|e| {
            let why = said.lock().ok().and_then(|mut s| s.take()).unwrap_or(e.message);
            AppError::new(
                e.code,
                format!("{why} A saved job can't answer questions: connect to the host once from SFTP, trust its key and save its password or key."),
            )
        })
}

/// Where a backup job writes: the folder and how to reach it.
pub struct Dest {
    pub fs: Fs,
    pub dir: String,
    /// "this computer" or the host's label.
    pub place: String,
}

/// The job's folder, from its plan (`folder` here, or `host_id` + `dir` on a host).
pub async fn dest(store: &Store, plan: &serde_json::Value) -> AppResult<Dest> {
    if let Some(folder) = plan.get("folder").and_then(|p| p.as_str()) {
        if !std::path::Path::new(folder).is_dir() {
            return Err(AppError::validation(format!(
                "The folder {folder} isn't there any more."
            )));
        }
        return Ok(Dest {
            fs: Fs::Local,
            dir: folder.to_string(),
            place: "this computer".into(),
        });
    }
    let host_id = plan
        .get("host_id")
        .and_then(|p| p.as_str())
        .ok_or_else(|| AppError::validation("This job has no folder."))?;
    let dir = plan
        .get("dir")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let remote = Arc::new(open_host(store, host_id).await?);
    let place = remote.label.clone();
    let fs = Fs::Remote(remote);
    match fs.stat(&dir).await {
        Ok(st) if st.kind == EntryKind::Dir => {}
        _ => {
            return Err(AppError::validation(format!(
                "The folder {dir} on {place} isn't there any more."
            )))
        }
    }
    Ok(Dest { fs, dir, place })
}

/// The file extension of a backup of this engine.
pub fn ext(engine: crate::dbs::Engine) -> &'static str {
    if crate::sql::backup::is_my(engine) {
        "sql"
    } else if crate::sql::backup::is_mongo(engine) {
        "archive.gz"
    } else {
        "dump"
    }
}

pub struct BackupDone {
    pub path: String,
    pub bytes: f64,
    /// What happened, for the run's log (the file, and what retention removed).
    pub log: Vec<String>,
    /// The read check (D17) of the file as it landed.
    pub check: crate::dbs::backupcheck::ReadCheck,
}

/// Runs a saved backup of the database open on `conn_id` into the job's folder, then applies retention.
/// Recording the run is the caller's job.
pub async fn backup_into(
    state: &AppState,
    conn_id: &str,
    job_id: &str,
    transfer_id: &str,
    progress: &(dyn Fn(f64) + Sync),
) -> AppResult<BackupDone> {
    let id = job_id.to_string();
    let (_, plan) = state
        .with_db_session(move |c, s| jobs::plan(c, s, &id))
        .await?;
    let (name, keep) = {
        let id = job_id.to_string();
        state
            .with_db_session(move |c, s| {
                let name = jobs::list(c, s)?
                    .into_iter()
                    .find(|x| x.id == id)
                    .map(|x| x.name)
                    .unwrap_or_default();
                Ok((name, jobs::retention(c, &id)?))
            })
            .await?
    };
    let db_id = state.sql.db_id(conn_id)?;
    let cfg = crate::sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(conn_id).await?;
    let dest = dest(&state.store(), &plan).await?;
    let ext = ext(cfg.engine);
    let db_name = cfg.database.clone().unwrap_or_else(|| cfg.name.clone());
    let file = schedule::backup_name(&db_name, &name, chrono::Local::now(), ext);
    let path = sftp::join(&dest.fs, &dest.dir, &file);
    let part = format!("{path}.part");

    let writer = dest.fs.open_write(&part).await?;
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
    // The writer task owns the file (or the SFTP handle) and ends when the pipe closes.
    let sink = tokio::spawn(async move {
        let mut w = writer;
        let mut n = 0u64;
        while let Some(chunk) = rx.recv().await {
            w.write_all(&chunk)
                .await
                .map_err(|e| AppError::internal(format!("Couldn't write the backup: {e}")))?;
            n += chunk.len() as u64;
        }
        w.shutdown()
            .await
            .map_err(|e| AppError::internal(format!("Couldn't finish the backup file: {e}")))?;
        Ok::<u64, AppError>(n)
    });
    let mut pipe = Pipe {
        tx: Some(tx),
        buf: Vec::with_capacity(CHUNK),
    };
    let stop = state.sql.begin_transfer(transfer_id);
    let ran = crate::sql::backup::run(crate::sql::backup::Run {
        cfg: &cfg,
        ssh: ssh.as_deref(),
        tool: crate::sql::backup::Tool::Dump,
        stdout: Some(&mut pipe),
        stdin_file: None,
        progress,
        log: &|l| log::debug!("dump tool: {l}"),
        stop: &stop,
    })
    .await;
    state.sql.end_transfer(transfer_id);
    let closed = pipe.close();
    let written = sink
        .await
        .map_err(|e| AppError::internal(e.to_string()))
        .and_then(|r| r);
    let result = match (ran, closed, written) {
        (Ok(()), Ok(()), Ok(bytes)) => Ok(bytes),
        // A failure to write explains a broken pipe better than the tool does.
        (_, _, Err(e)) => Err(e),
        (Err(e), _, _) => Err(e),
        (_, Err(e), _) => Err(AppError::internal(format!(
            "Couldn't write the backup: {e}"
        ))),
    };
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            let _ = dest.fs.remove(&part).await;
            return Err(e);
        }
    };
    if dest.fs.exists(&path).await {
        dest.fs.remove(&path).await?;
    }
    dest.fs.rename(&part, &path).await?;
    // D17: read it back where it landed (from a host, through a private local copy).
    let check = match &dest.fs {
        Fs::Local => crate::dbs::backupcheck::read_check(std::path::Path::new(&path)).await,
        Fs::Remote(_) => match fetch(&dest.fs, &path).await {
            Ok((_tmp, local)) => crate::dbs::backupcheck::read_check(&local).await,
            Err(e) => crate::dbs::backupcheck::ReadCheck {
                ok: false,
                summary: "Not read back".into(),
                problem: Some(e.message),
            },
        },
    };
    if !check.ok {
        // A file that doesn't read back isn't a backup: it is set aside (`.bad`), out of retention's way.
        let _ = dest.fs.rename(&path, &format!("{path}.bad")).await;
        return Err(AppError::validation(format!(
            "The backup was written but doesn't read back ({}): it was kept as {path}.bad.",
            check.problem.clone().unwrap_or_default()
        )));
    }
    let mut log = vec![
        format!("Saved {path} on {}", dest.place),
        format!("Read back: {}", check.summary),
    ];
    log.extend(apply_retention(&dest, &db_name, &name, ext, &keep).await);
    Ok(BackupDone {
        path,
        bytes: bytes as f64,
        log,
        check,
    })
}

/// Removes this job's older files as its retention says, and says what it removed (or couldn't).
pub async fn apply_retention(
    dest: &Dest,
    db: &str,
    job: &str,
    ext: &str,
    keep: &Retention,
) -> Vec<String> {
    if keep.keep_last.is_none() && keep.keep_days.is_none() {
        return Vec::new();
    }
    let entries = match dest.fs.list(&dest.dir, true).await {
        Ok(e) => e,
        Err(e) => return vec![format!("Retention skipped: {}", e.message)],
    };
    let ours: Vec<_> = entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::File && !e.symlink)
        .filter_map(|e| schedule::stamp_of(&e.name, db, job, ext).map(|t| (e.name, t)))
        .collect();
    let mut log = Vec::new();
    for name in schedule::to_remove(ours, keep, chrono::Local::now().naive_local()) {
        let p = sftp::join(&dest.fs, &dest.dir, &name);
        match dest.fs.remove(&p).await {
            Ok(()) => log.push(format!("Removed the old backup {name}")),
            Err(e) => log.push(format!("Couldn't remove {name}: {}", e.message)),
        }
    }
    log
}

/// Copies a dump file from a host into a private folder here, for reading and restoring it (removed with the dump).
pub async fn fetch(
    fs: &Fs,
    path: &str,
) -> AppResult<(crate::dbs::dumpfile::TempDir, std::path::PathBuf)> {
    match fs.stat(path).await {
        Ok(st) if st.kind == EntryKind::File => {}
        Ok(_) => {
            return Err(AppError::validation(
                "Pick a dump file (folders on a host aren't read yet).",
            ))
        }
        Err(e) => return Err(e),
    }
    let tmp = crate::dbs::dumpfile::TempDir::new("fetched")?;
    let local = tmp.0.join(sftp::fs_basename(fs, path));
    let mut from = fs.open_read(path).await?;
    let mut to = tokio::fs::File::create(&local)
        .await
        .map_err(|e| AppError::internal(format!("Couldn't make a local copy: {e}")))?;
    tokio::io::copy(&mut from, &mut to)
        .await
        .map_err(|e| AppError::internal(format!("Couldn't fetch the dump: {e}")))?;
    to.flush()
        .await
        .map_err(|e| AppError::internal(format!("Couldn't fetch the dump: {e}")))?;
    Ok((tmp, local))
}

/// The message of an error, for a run's log, with what a scheduled run can't do spelled out.
pub fn failure_text(e: &AppError) -> String {
    match e.code {
        ErrorCode::HostKeyChanged => format!(
            "{} A scheduled run never trusts a host key: open the database once by hand first.",
            e.message
        ),
        _ => e.message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbs::jobs::NewJob;
    use crate::dbs::store::{DatabaseInput, DbOptions};
    use crate::redis::store::{RedisEnvironment, RedisRoute, RedisWriteMode};

    /// D15 Docker: a saved backup of the test MongoDB streams to a folder on test host `a` over SFTP (no `.part` left,
    /// a gzip archive), then retention removes this job's oldest file and nothing else. A host whose key isn't
    /// trusted yet fails without asking. Needs `scripts/test-servers.sh up` and `mongodump` here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs the Docker test servers"]
    async fn l3_backup_streams_to_a_host_folder_and_retention_keeps_only_its_own_newest() {
        let h = crate::ssh::tests::harness(None).await;
        let ssh = crate::dbs::spike::sign_in_to_a(&h).await;
        let dir = format!("/tmp/dalan-d15-{}", std::process::id());
        let other = format!("{dir}/shop-Other-2026-01-01-0000.archive.gz");
        let old = |d: u32| format!("{dir}/admin-Nightly-2026-01-{d:02}-0200.archive.gz");
        crate::sql::backup::exec_collect(
            &ssh,
            &format!(
                "rm -rf {dir}; mkdir -p {dir} && touch {} {} {other} {dir}/notes.txt",
                old(1),
                old(2)
            ),
        )
        .await
        .unwrap();
        let store = h.state.store();
        let host_id = h.host_id.clone();
        let d = dir.clone();
        let (db_id, job_id) = store
            .with_db_session(move |c, s| {
                let db = crate::dbs::store::save(
                    c,
                    s,
                    DatabaseInput {
                        id: None,
                        engine: crate::dbs::Engine::Mongodb,
                        name: "mongo".into(),
                        environment: RedisEnvironment::Staging,
                        route: RedisRoute::Direct,
                        host_id: None,
                        address: "127.0.0.1".into(),
                        port: Some(17017),
                        default_database: Some("admin".into()),
                        username: Some("app".into()),
                        password: Some("test-mongo".into()),
                        clear_password: false,
                        tls: "off".into(),
                        tls_server_name: None,
                        write_mode: RedisWriteMode::Open,
                        options: DbOptions {
                            auth_source: Some("admin".into()),
                            ..Default::default()
                        },
                        from_url: None,
                    },
                    false,
                )?;
                let job = jobs::save(
                    c,
                    s,
                    NewJob {
                        name: "Nightly".into(),
                        kind: jobs::JobKind::Backup,
                        source: Some(db.id.clone()),
                        target: None,
                        summary: "Back up mongo".into(),
                        plan: serde_json::json!({ "host_id": host_id, "dir": d }),
                        destination: Some(serde_json::json!({ "host_id": host_id })),
                    },
                )?;
                jobs::set_schedule(
                    c,
                    s,
                    &job.id,
                    Some(schedule::Schedule {
                        every: schedule::Every::Day { hour: 2, minute: 0 },
                        notify_ok: false,
                    }),
                    Some(Retention {
                        keep_last: Some(2),
                        keep_days: None,
                    }),
                )?;
                // The host can't be deleted while the job backs up to it.
                let err = crate::hosts::delete(c, s, &host_id).unwrap_err();
                assert!(
                    err.message.contains("“Nightly” backs up to this host"),
                    "{}",
                    err.message
                );
                Ok((db.id, job.id))
            })
            .await
            .unwrap();
        let conn = h
            .state
            .sql
            .open_headless(store.clone(), db_id)
            .await
            .expect("opening the database with no window");
        let done = backup_into(&h.state, &conn, &job_id, "t1", &|_| {})
            .await
            .expect("the backup");
        h.state.sql.close(&conn);
        assert!(done.bytes > 100.0, "{}", done.bytes);
        assert!(
            done.path.starts_with(&dir) && done.path.ends_with(".archive.gz"),
            "{}",
            done.path
        );
        let (_, ls) = crate::sql::backup::exec_collect(
            &ssh,
            &format!("ls {dir}; head -c 2 {} | od -An -tx1", done.path),
        )
        .await
        .unwrap();
        assert!(ls.contains("1f 8b"), "not a gzip archive: {ls}");
        assert!(!ls.contains(".part"), "{ls}");
        assert!(
            !ls.contains("admin-Nightly-2026-01-01"),
            "the oldest should be gone: {ls}"
        );
        assert!(ls.contains("admin-Nightly-2026-01-02"), "{ls}");
        assert!(
            ls.contains("notes.txt") && ls.contains("shop-Other"),
            "{ls}"
        );
        assert!(
            done.log
                .iter()
                .any(|l| l
                    .contains("Removed the old backup admin-Nightly-2026-01-01-0200.archive.gz")),
            "{:?}",
            done.log
        );

        // Another saved host (the in-process test server), never connected: its key isn't trusted, so no questions,
        // no run.
        let port = h.port;
        let fresh = store
            .with_db_session(move |c, s| {
                let mut input = crate::ssh::tests::host_input("a2", port, None);
                input.username = Some("test".into());
                input.password = Some("test".into());
                crate::hosts::save(c, s, &input)
            })
            .await
            .unwrap();
        let err = open_host(&store, &fresh.id)
            .await
            .err()
            .expect("an untrusted key must fail");
        assert!(
            err.message.contains("can't answer questions"),
            "{}",
            err.message
        );
        let _ = crate::sql::backup::exec_collect(&ssh, &format!("rm -rf {dir}")).await;
    }
}
