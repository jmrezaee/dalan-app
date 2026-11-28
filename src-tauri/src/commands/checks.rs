//! Backup check (D17, E7): the read check runs inside every backup; this is the optional restore check. The backup
//! is read back, restored into a scratch database `dalan_check_<id>` on a tab of a server that isn't production,
//! its tables (or collections) are counted there and in the source, and the scratch database is dropped again, also
//! when anything fails. The file is the backup just made, one chosen here, or one in a folder on a host.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use super::restore::Owned;
use super::sql::CsvFile;
use crate::dbs::backupcheck::{self, ReadCheck};
use crate::error::{AppError, AppResult};
use crate::sql::verify::{self, CountRow};
use crate::sql::{self};
use crate::state::AppState;

/// Which backup to check.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckFile {
    /// A file id from the backup just made, or from `backup_check_pick` (both kept for the source's tab).
    Picked { file_id: String },
    /// A file in a folder on a host, through the host picker's SFTP session.
    Host { sftp_id: String, path: String },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct VerifyReport {
    pub read: ReadCheck,
    /// Empty when the read check failed (nothing was restored).
    pub rows: Vec<CountRow>,
    pub same: bool,
    pub restored_on: String,
    pub seconds: f64,
}

/// The system dialog for a backup file to check. The window gets an id; the path stays here.
#[tauri::command]
#[specta::specta]
pub async fn backup_check_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
) -> AppResult<Option<CsvFile>> {
    state.require_unlocked()?;
    state.sql.db_id(&conn_id)?;
    Ok(super::sql::pick_open(&app, "Check a backup")
        .await?
        .map(|p| CsvFile {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            id: state.sql.put_file(&conn_id, p),
        }))
}

#[tauri::command]
#[specta::specta]
pub async fn backup_verify(
    state: State<'_, AppState>,
    source_conn_id: String,
    check_conn_id: String,
    file: CheckFile,
) -> AppResult<VerifyReport> {
    state.require_unlocked()?;
    let started = std::time::Instant::now();
    // The file, here (a host's is fetched into a private folder that lives until the check ends).
    let mut _fetched = None;
    let path = match &file {
        CheckFile::Picked { file_id } => state.sql.file(&source_conn_id, file_id)?,
        CheckFile::Host { sftp_id, path } => {
            let (tmp, local) = crate::dbs::runner::fetch(&state.sftp.fs(sftp_id)?, path).await?;
            _fetched = Some(tmp);
            local
        }
    };
    let read = backupcheck::read_check(&path).await;
    let check_db = sql::gate_target(&state.store(), &state.sql.db_id(&check_conn_id)?).await?;
    if !read.ok {
        return Ok(VerifyReport {
            read,
            rows: vec![],
            same: false,
            restored_on: check_db.name,
            seconds: started.elapsed().as_secs_f64(),
        });
    }
    let src_cfg = sql::load(&state.store(), &state.sql.db_id(&source_conn_id)?).await?;
    let cfg = sql::load(&state.store(), &state.sql.db_id(&check_conn_id)?).await?;
    if crate::sql::copy::family(src_cfg.engine) != crate::sql::copy::family(cfg.engine) {
        return Err(AppError::validation(
            "Restore the check on a server of the same engine as the backup.",
        ));
    }
    let dump = Arc::new(crate::dbs::dumpfile::pick(&path, "check".into()).await?);
    let ssh = state.sql.ssh(&check_conn_id).await?;
    let owned = Owned::get(&state.sql, &check_conn_id, cfg.engine).await?;
    let source_db = src_cfg.database.clone().unwrap_or_default();
    let rows = crate::sql::restore::with_scratch(
        owned.conn(),
        cfg,
        ssh,
        dump,
        "check",
        crate::sql::restore::Scratch {
            name: &check_db.name,
            environment: &check_db.environment,
            mode: check_db.mode,
        },
        "dalan_check",
        crate::dbs::dumpscan::Content::All,
        async |scratch| {
            let restored = verify::counts(
                &state
                    .sql
                    .connect_to(&check_conn_id, Some(scratch.clone()))
                    .await?,
                &scratch,
            )
            .await?;
            let source = verify::counts(
                &state.sql.connect_to(&source_conn_id, None).await?,
                &source_db,
            )
            .await?;
            Ok(verify::compare(restored, &source))
        },
    )
    .await?;
    Ok(VerifyReport {
        same: !rows.is_empty() && rows.iter().all(|r| r.same),
        rows,
        read,
        restored_on: check_db.name,
        seconds: started.elapsed().as_secs_f64(),
    })
}
