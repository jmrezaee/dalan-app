//! Backup checks (D17, E7; replica/databases/restore-copy.md §9).
//!
//! The read check runs at the end of every backup: the file is read back with D12's reader (format, archive TOC,
//! collections), then once more to its end through its compression, which checks a gzip or zstd trailer, and a
//! plain SQL dump must end with its tool's "dump complete" line. A cut-off file fails it.

use std::path::Path;

use serde::Serialize;
use specta::Type;
use tokio::io::AsyncReadExt;

use crate::dbs::dumpfile::{self, DumpFormat};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Type)]
pub struct ReadCheck {
    pub ok: bool,
    /// What was read ("PostgreSQL custom archive, 42 objects").
    pub summary: String,
    /// Why it failed.
    pub problem: Option<String>,
}

fn failed(summary: &str, why: impl Into<String>) -> ReadCheck {
    ReadCheck {
        ok: false,
        summary: summary.into(),
        problem: Some(why.into()),
    }
}

/// The plain dump's last line its tool writes when it finished.
fn end_marker(tail: &str) -> bool {
    [
        "PostgreSQL database dump complete",
        "PostgreSQL database cluster dump complete",
        "-- Dump completed",
    ]
    .iter()
    .any(|m| tail.contains(m))
}

/// Reads a backup file back. Never errors: a file that can't be read is a failed check.
pub async fn read_check(path: &Path) -> ReadCheck {
    let dump = match dumpfile::pick(path, "check".into()).await {
        Ok(d) => d,
        Err(e) => return failed("Not readable as a dump", e.message),
    };
    let info = &dump.info;
    let summary = format!(
        "{}{}, {} objects",
        info.format_label,
        dump.outer
            .map(|o| format!(" ({})", o.label()))
            .unwrap_or_default(),
        info.objects.len() as f64 + info.objects_more
    );
    if let Some(r) = &info.refused {
        return failed(&summary, r.clone());
    }
    if matches!(info.format, DumpFormat::PgCustom | DumpFormat::PgTar) && dump.toc.is_empty() {
        return failed(&summary, "The archive's table of contents can't be read.");
    }
    if info.format.is_folder() {
        return ReadCheck {
            ok: true,
            summary,
            problem: None,
        };
    }
    // To the end, through the compression (its trailer is checked there), keeping the last 4 KB.
    let tail = match drain(path, dump.outer).await {
        Ok(t) => t,
        Err(e) => {
            return failed(
                &summary,
                format!("It ends early or is damaged: {}", e.message),
            )
        }
    };
    let plain = matches!(info.format, DumpFormat::PgPlain | DumpFormat::MySql);
    if plain && !end_marker(&String::from_utf8_lossy(&tail)) {
        return failed(&summary, "The dump has no \"dump complete\" line at its end: the tool stopped before it finished.");
    }
    ReadCheck {
        ok: true,
        summary,
        problem: None,
    }
}

async fn drain(path: &Path, outer: Option<dumpfile::Outer>) -> AppResult<Vec<u8>> {
    let mut r = dumpfile::open(path, outer).await?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut tail: Vec<u8> = Vec::with_capacity(8192);
    loop {
        let n = r
            .read(&mut buf)
            .await
            .map_err(|e| crate::error::AppError::validation(e.to_string()))?;
        if n == 0 {
            return Ok(tail);
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > 4096 {
            tail.drain(..tail.len() - 4096);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/dumps")
            .join(name)
    }

    #[tokio::test]
    async fn whole_backups_pass_and_cut_off_ones_fail() {
        for f in [
            "pg16.custom",
            "pg16.sql",
            "pg16.sql.gz",
            "mysql84.sql",
            "mysql84.sql.gz",
            "mongo7.archive.gz",
            "pg17.sql.zst",
        ] {
            let c = read_check(&fixture(f)).await;
            assert!(c.ok, "{f}: {c:?}");
        }
        let dir = std::env::temp_dir().join(format!("dalan-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cut = |name: &str, keep: f64| {
            let b = std::fs::read(fixture(name)).unwrap();
            let p = dir.join(format!("cut-{name}"));
            std::fs::write(&p, &b[..(b.len() as f64 * keep) as usize]).unwrap();
            p
        };
        for (f, keep) in [
            ("pg16.sql", 0.8),
            ("mysql84.sql", 0.9),
            ("pg16.sql.gz", 0.95),
            ("mongo7.archive.gz", 0.9),
            ("pg16.custom", 0.01),
        ] {
            let c = read_check(&cut(f, keep)).await;
            assert!(!c.ok, "{f} cut at {keep} passed: {c:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
