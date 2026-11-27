//! Dump files made anywhere (D12, S48, F31; replica/databases/restore-copy.md §2): what a file is, read from its
//! bytes (never its name), and what is inside it, read by our own readers, streaming, without loading the file.
//!
//! - Outer compression (gzip, zstd, bzip2, xz, a one-file zip) is unwrapped while reading.
//! - PostgreSQL archives (custom, tar, directory): the header and the table of contents (`toc.dat`).
//! - MongoDB: the `mongodump --archive` prelude and body (document counts), or a `mongodump --out` folder.
//! - Plain SQL (`pg_dump -Fp`, `pg_dumpall`, `mysqldump`, `mariadb-dump`, other tools): `dumpscan`.
//!
//! Folders packed in a zip or a tar are unpacked into a private temporary folder on this computer (removed when the
//! dump is let go), so every directory format is read and uploaded the same way.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bson::Document;
use serde::Serialize;
use specta::Type;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

use super::dumpscan::{self, Dialect, PlainInfo};
use super::Engine;
use crate::error::{AppError, AppResult, ErrorCode};

pub type Reader = Pin<Box<dyn AsyncRead + Send>>;

/// Compression around the dump itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outer {
    Gzip,
    Zstd,
    Bzip2,
    Xz,
    /// A zip with one file in it.
    Zip,
}

impl Outer {
    pub fn label(self) -> &'static str {
        match self {
            Outer::Gzip => "gzip",
            Outer::Zstd => "zstd",
            Outer::Bzip2 => "bzip2",
            Outer::Xz => "xz",
            Outer::Zip => "zip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DumpFormat {
    /// `pg_dump -Fc`
    PgCustom,
    /// `pg_dump -Ft`
    PgTar,
    /// `pg_dump -Fd`
    PgDirectory,
    /// `pg_dump -Fp`, `pg_dumpall`, or SQL that reads as PostgreSQL's
    PgPlain,
    /// `mysqldump`, `mariadb-dump`, or SQL that reads as MySQL's
    MySql,
    /// `mongodump --archive`
    MongoArchive,
    /// `mongodump --out`
    MongoFolder,
}

impl DumpFormat {
    pub fn engine(self) -> Engine {
        match self {
            DumpFormat::PgCustom
            | DumpFormat::PgTar
            | DumpFormat::PgDirectory
            | DumpFormat::PgPlain => Engine::Postgres,
            DumpFormat::MySql => Engine::Mysql,
            DumpFormat::MongoArchive | DumpFormat::MongoFolder => Engine::Mongodb,
        }
    }

    /// The tool has to read files on the host (it can't take the dump on its input).
    pub fn is_folder(self) -> bool {
        matches!(self, DumpFormat::PgDirectory | DumpFormat::MongoFolder)
    }

    pub fn label(self) -> &'static str {
        match self {
            DumpFormat::PgCustom => "PostgreSQL custom archive",
            DumpFormat::PgTar => "PostgreSQL tar archive",
            DumpFormat::PgDirectory => "PostgreSQL directory dump",
            DumpFormat::PgPlain => "PostgreSQL SQL script",
            DumpFormat::MySql => "MySQL SQL script",
            DumpFormat::MongoArchive => "MongoDB archive",
            DumpFormat::MongoFolder => "MongoDB dump folder",
        }
    }
}

/// One thing the dump makes, for the preview.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DumpObject {
    pub kind: String,
    pub database: Option<String>,
    pub name: String,
    pub rows: Option<f64>,
    /// `rows` is an exact count (COPY lines, archive documents); otherwise it is counted from INSERT tuples.
    pub rows_exact: bool,
}

/// "Things that will need attention", each saying what Dalan will do about it.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DumpNote {
    pub code: String,
    pub text: String,
}

/// What the window shows about a dump (the path stays in Rust; the window holds the id).
#[derive(Debug, Clone, Serialize, Type)]
pub struct DumpInfo {
    pub id: String,
    pub file_name: String,
    pub bytes: f64,
    pub format: DumpFormat,
    pub format_label: String,
    pub engine: Engine,
    /// A MariaDB dump (restorable into MySQL or MariaDB, with a note).
    pub mariadb: bool,
    /// The format was guessed from the statements (no tool header).
    pub guessed: bool,
    pub compression: Option<String>,
    pub made_by: Option<String>,
    pub server_version: Option<String>,
    pub source_db: Option<String>,
    pub created: Option<String>,
    pub encoding: Option<String>,
    /// Databases in the file (multi-database dumps; Mongo archives).
    pub databases: Vec<String>,
    pub objects: Vec<DumpObject>,
    pub objects_more: f64,
    pub notes: Vec<DumpNote>,
    /// Why the file will not be restored (the safety scan), in words. Nothing can be planned while this is set.
    pub refused: Option<String>,
    /// Directory formats go to the host over SFTP first.
    pub needs_upload: bool,
}

/// What Rust keeps about a picked dump, for planning and running the restore.
pub struct Dump {
    pub info: DumpInfo,
    pub path: PathBuf,
    pub outer: Option<Outer>,
    /// Directory formats: the folder (the picked one, or one unpacked from a zip or tar).
    pub folder: Option<PathBuf>,
    /// Removes the unpacked folder when the dump is dropped.
    pub _unpacked: Option<TempDir>,
    /// PostgreSQL archives: the format version (major, minor) and pg_dump's major version.
    pub archive_version: Option<(u8, u8)>,
    pub dump_major: Option<u32>,
    /// Tables the dump creates or fills: (schema or "", table).
    pub tables: Vec<(String, String)>,
    /// MongoDB folders with `.bson.gz` files need `--gzip`.
    pub mongo_gzip: bool,
}

/// A private folder on this computer, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(what: &str) -> AppResult<TempDir> {
        let p = std::env::temp_dir().join(format!("dalan-{what}-{}", uuid::Uuid::now_v7()));
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&p)
            .map_err(|e| AppError::internal(format!("Couldn't make a temporary folder: {e}")))?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn unknown(first: &[u8]) -> AppError {
    let shown: String = first
        .iter()
        .take(16)
        .map(|b| {
            if b.is_ascii_graphic() || *b == b' ' {
                (*b as char).to_string()
            } else {
                format!("\\x{b:02x}")
            }
        })
        .collect();
    AppError::new(
        ErrorCode::DbDumpUnknown,
        format!("Dalan doesn't recognise this file. It starts with “{shown}”."),
    )
}

fn io(e: std::io::Error) -> AppError {
    AppError::validation(format!("Couldn't read the file: {e}"))
}

/// Which compression the first bytes show.
pub fn outer_of(head: &[u8]) -> Option<Outer> {
    if head.starts_with(&[0x1f, 0x8b]) {
        Some(Outer::Gzip)
    } else if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some(Outer::Zstd)
    } else if head.starts_with(b"BZh") {
        Some(Outer::Bzip2)
    } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Some(Outer::Xz)
    } else if head.starts_with(b"PK\x03\x04") {
        Some(Outer::Zip)
    } else {
        None
    }
}

/// Opens a file, unwrapping one layer of compression. A zip must hold exactly one file here (folders are unpacked
/// by [`pick`]).
pub async fn open(path: &Path, outer: Option<Outer>) -> AppResult<Reader> {
    open_counted(path, outer, None).await
}

/// Counts the bytes read from the file itself (before decompression), for a restore's progress bar.
struct Counted<R> {
    inner: R,
    n: Arc<AtomicU64>,
}

impl<R: AsyncRead + Unpin> AsyncRead for Counted<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &r {
            self.n
                .fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        }
        r
    }
}

pub async fn open_counted(
    path: &Path,
    outer: Option<Outer>,
    counter: Option<Arc<AtomicU64>>,
) -> AppResult<Reader> {
    use async_compression::tokio::bufread as dec;
    let f = tokio::fs::File::open(path).await.map_err(io)?;
    let n = counter.clone().unwrap_or_default();
    let b = BufReader::with_capacity(64 * 1024, Counted { inner: f, n });
    Ok(match outer {
        None => Box::pin(b),
        Some(Outer::Gzip) => {
            let mut d = dec::GzipDecoder::new(b);
            d.multiple_members(true);
            Box::pin(d)
        }
        Some(Outer::Zstd) => Box::pin(dec::ZstdDecoder::new(b)),
        Some(Outer::Bzip2) => Box::pin(dec::BzDecoder::new(b)),
        Some(Outer::Xz) => Box::pin(dec::XzDecoder::new(b)),
        Some(Outer::Zip) => {
            // The zip reader is blocking: it runs on its own thread and writes into a pipe this side reads.
            let (mut w, r) = tokio::io::duplex(256 * 1024);
            let path = path.to_path_buf();
            let rt = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                use std::io::Read;
                use tokio::io::AsyncWriteExt;
                let Ok(file) = std::fs::File::open(&path) else {
                    return;
                };
                let Ok(mut z) = zip::ZipArchive::new(file) else {
                    return;
                };
                let Ok(mut entry) = z.by_index(0) else { return };
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match entry.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Some(c) = &counter {
                                c.fetch_add(n as u64, Ordering::Relaxed);
                            }
                            if rt.block_on(w.write_all(&buf[..n])).is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = rt.block_on(w.shutdown());
            });
            Box::pin(r)
        }
    })
}

/// Reads up to `n` bytes (fewer at the end of the stream).
async fn read_up_to(r: &mut Reader, n: usize) -> AppResult<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let k = r.read(&mut buf[got..]).await.map_err(io)?;
        if k == 0 {
            break;
        }
        got += k;
    }
    buf.truncate(got);
    Ok(buf)
}

fn is_tar(head: &[u8]) -> bool {
    head.len() >= 512 && &head[257..262] == b"ustar"
}

fn tar_name(h: &[u8]) -> String {
    let name = &h[..100];
    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(100)];
    let prefix = &h[345..500];
    let prefix = &prefix[..prefix.iter().position(|&b| b == 0).unwrap_or(155)];
    let mut s = String::from_utf8_lossy(prefix).into_owned();
    if !s.is_empty() {
        s.push('/');
    }
    s.push_str(&String::from_utf8_lossy(name));
    s
}

fn tar_size(h: &[u8]) -> u64 {
    let f = &h[124..136];
    if f[0] & 0x80 != 0 {
        // Base-256 (sizes over 8 GB).
        return f[4..].iter().fold(0u64, |a, b| (a << 8) | u64::from(*b));
    }
    let s = String::from_utf8_lossy(f);
    u64::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0)
}

/// A relative path from an archive, or `None` when it would leave the folder (`..`, absolute, drive letters).
fn safe_rel(name: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            x if x.contains(':') => return None,
            x => p.push(x),
        }
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// Unpacks a tar stream into `dir`. Only plain files and folders; links and devices are skipped.
async fn untar(r: &mut Reader, dir: &Path) -> AppResult<()> {
    use tokio::io::AsyncWriteExt;
    let mut long_name: Option<String> = None;
    loop {
        let h = read_up_to(r, 512).await?;
        if h.len() < 512 || h.iter().all(|&b| b == 0) {
            return Ok(());
        }
        let size = tar_size(&h);
        let kind = h[156];
        let name = long_name.take().unwrap_or_else(|| tar_name(&h));
        let padded = size.div_ceil(512) * 512;
        if kind == b'L' {
            // GNU long name: the next header's name is this entry's contents.
            let body = read_up_to(r, padded as usize).await?;
            long_name = Some(
                String::from_utf8_lossy(&body[..size as usize])
                    .trim_end_matches('\0')
                    .to_string(),
            );
            continue;
        }
        let target = safe_rel(&name).map(|p| dir.join(p));
        match (kind, target) {
            (b'0' | 0, Some(t)) => {
                if let Some(parent) = t.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(io)?;
                }
                let mut f = tokio::fs::File::create(&t).await.map_err(io)?;
                let mut left = size;
                let mut buf = vec![0u8; 64 * 1024];
                while left > 0 {
                    let want = left.min(buf.len() as u64) as usize;
                    r.read_exact(&mut buf[..want]).await.map_err(io)?;
                    f.write_all(&buf[..want]).await.map_err(io)?;
                    left -= want as u64;
                }
                let pad = (padded - size) as usize;
                if pad > 0 {
                    read_up_to(r, pad).await?;
                }
            }
            (b'5', Some(t)) => {
                tokio::fs::create_dir_all(&t).await.map_err(io)?;
                read_up_to(r, padded as usize).await?;
            }
            _ => {
                let mut left = padded;
                while left > 0 {
                    let n = read_up_to(r, left.min(64 * 1024) as usize).await?;
                    if n.is_empty() {
                        return Ok(());
                    }
                    left -= n.len() as u64;
                }
            }
        }
    }
}

/// Unpacks a zip into `dir` (blocking: run on a blocking thread).
fn unzip(path: &Path, dir: &Path) -> AppResult<()> {
    let file = std::fs::File::open(path).map_err(io)?;
    let mut z = zip::ZipArchive::new(file)
        .map_err(|e| AppError::validation(format!("Couldn't read the zip: {e}")))?;
    for i in 0..z.len() {
        let mut e = z
            .by_index(i)
            .map_err(|e| AppError::validation(format!("Couldn't read the zip: {e}")))?;
        let Some(rel) = e
            .enclosed_name()
            .and_then(|p| safe_rel(&p.to_string_lossy()))
        else {
            continue;
        };
        let t = dir.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&t).map_err(io)?;
            continue;
        }
        if !e.is_file() {
            continue;
        }
        if let Some(parent) = t.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut f = std::fs::File::create(&t).map_err(io)?;
        std::io::copy(&mut e, &mut f).map_err(io)?;
    }
    Ok(())
}

/// The folder a directory dump really starts in: the one with `toc.dat`, or the top of a Mongo dump.
fn dump_root(dir: &Path) -> Option<(PathBuf, DumpFormat)> {
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        if d.join("toc.dat").is_file() {
            return Some((d, DumpFormat::PgDirectory));
        }
        let entries: Vec<PathBuf> = std::fs::read_dir(&d)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        if entries.iter().any(|p| is_bson(p)) {
            // A database's folder: its parent is the dump's top (`<top>/<db>/<coll>.bson`).
            return Some((d, DumpFormat::MongoFolder));
        }
        if depth < 3 {
            stack.extend(
                entries
                    .into_iter()
                    .filter(|p| p.is_dir())
                    .map(|p| (p, depth + 1)),
            );
        }
    }
    None
}

fn is_bson(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    n.ends_with(".bson") || n.ends_with(".bson.gz")
}

/// Reads a picked file or folder: what it is and what is inside. The safety scan runs here too; a refused file
/// comes back with `refused` set, so the person sees why.
pub async fn pick(path: &Path, id: String) -> AppResult<Dump> {
    let meta = tokio::fs::metadata(path).await.map_err(io)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = if meta.is_dir() {
        folder_size(path)
    } else {
        meta.len()
    } as f64;
    let mut base = DumpInfo {
        id,
        file_name,
        bytes,
        format: DumpFormat::PgPlain,
        format_label: String::new(),
        engine: Engine::Postgres,
        mariadb: false,
        guessed: false,
        compression: None,
        made_by: None,
        server_version: None,
        source_db: None,
        created: None,
        encoding: None,
        databases: Vec::new(),
        objects: Vec::new(),
        objects_more: 0.0,
        notes: Vec::new(),
        refused: None,
        needs_upload: false,
    };
    if meta.is_dir() {
        return read_folder(path.to_path_buf(), None, path.to_path_buf(), base).await;
    }
    let mut raw = open(path, None).await?;
    let head = read_up_to(&mut raw, 8).await?;
    drop(raw);
    let mut outer = outer_of(&head);
    if outer == Some(Outer::Zip) {
        let p = path.to_path_buf();
        let count = tokio::task::spawn_blocking(move || -> AppResult<usize> {
            let f = std::fs::File::open(&p).map_err(io)?;
            Ok(zip::ZipArchive::new(f)
                .map_err(|e| AppError::validation(format!("Couldn't read the zip: {e}")))?
                .len())
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        if count != 1 {
            base.compression = Some("zip".into());
            let tmp = TempDir::new("dump")?;
            let (p, d) = (path.to_path_buf(), tmp.0.clone());
            tokio::task::spawn_blocking(move || unzip(&p, &d))
                .await
                .map_err(|e| AppError::internal(e.to_string()))??;
            let dir = tmp.0.clone();
            return read_folder(dir, Some(tmp), path.to_path_buf(), base).await;
        }
    }
    base.compression = outer.map(|o| o.label().to_string());
    let mut r = open(path, outer).await?;
    let head = read_up_to(&mut r, 64 * 1024).await?;
    // A compressed file inside a zip (`dump.sql.gz` zipped) is not unwrapped twice.
    if outer.is_some() && outer_of(&head).is_some() {
        return Err(unknown(&head));
    }
    if is_tar(&head) && !tar_name(&head).ends_with("toc.dat") {
        // A tar of a folder (a directory dump, or a Mongo dump folder): unpacked first.
        let tmp = TempDir::new("dump")?;
        let mut r: Reader = Box::pin(std::io::Cursor::new(head).chain(r));
        untar(&mut r, &tmp.0).await?;
        let dir = tmp.0.clone();
        return read_folder(dir, Some(tmp), path.to_path_buf(), base).await;
    }
    let format = if head.starts_with(b"PGDMP") {
        DumpFormat::PgCustom
    } else if is_tar(&head) {
        DumpFormat::PgTar
    } else if head.starts_with(&MONGO_MAGIC) {
        DumpFormat::MongoArchive
    } else {
        match sql_kind(&head) {
            Some((f, guessed)) => {
                base.guessed = guessed;
                f
            }
            None => return Err(unknown(&head)),
        }
    };
    base.format = format;
    base.engine = format.engine();
    base.format_label = format.label().to_string();
    let r: Reader = Box::pin(std::io::Cursor::new(head).chain(r));
    let mut dump = Dump {
        info: base,
        path: path.to_path_buf(),
        outer: outer.take(),
        folder: None,
        _unpacked: None,
        archive_version: None,
        dump_major: None,
        tables: Vec::new(),
        mongo_gzip: false,
    };
    match format {
        DumpFormat::PgCustom => read_pg_archive(r, &mut dump).await?,
        DumpFormat::PgTar => {
            let mut r = r;
            let h = read_up_to(&mut r, 512).await?;
            let size = tar_size(&h);
            let toc = read_up_to(&mut r, size as usize).await?;
            read_pg_archive(Box::pin(std::io::Cursor::new(toc)), &mut dump).await?;
        }
        DumpFormat::MongoArchive => read_mongo_archive(r, &mut dump).await?,
        DumpFormat::PgPlain | DumpFormat::MySql => read_plain(r, &mut dump).await?,
        DumpFormat::PgDirectory | DumpFormat::MongoFolder => {
            unreachable!("folders are read by read_folder")
        }
    }
    Ok(dump)
}

fn folder_size(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                match e.metadata() {
                    Ok(m) if m.is_dir() => stack.push(e.path()),
                    Ok(m) => total += m.len(),
                    Err(_) => {}
                }
            }
        }
    }
    total
}

async fn read_folder(
    dir: PathBuf,
    unpacked: Option<TempDir>,
    picked: PathBuf,
    mut base: DumpInfo,
) -> AppResult<Dump> {
    let Some((root, format)) = dump_root(&dir) else {
        return Err(AppError::new(ErrorCode::DbDumpUnknown, "Dalan doesn't recognise this folder: it has no toc.dat (a PostgreSQL directory dump) and no .bson files (a MongoDB dump)."));
    };
    base.format = format;
    base.engine = format.engine();
    base.format_label = format.label().to_string();
    base.needs_upload = true;
    let mut dump = Dump {
        info: base,
        path: picked,
        outer: None,
        folder: Some(root.clone()),
        _unpacked: unpacked,
        archive_version: None,
        dump_major: None,
        tables: Vec::new(),
        mongo_gzip: false,
    };
    match format {
        DumpFormat::PgDirectory => {
            let r: Reader = Box::pin(BufReader::new(
                tokio::fs::File::open(root.join("toc.dat"))
                    .await
                    .map_err(io)?,
            ));
            read_pg_archive(r, &mut dump).await?;
        }
        _ => read_mongo_folder(&root, &mut dump).await?,
    }
    Ok(dump)
}

/// Plain SQL: which client reads it, and whether that is a guess.
fn sql_kind(head: &[u8]) -> Option<(DumpFormat, bool)> {
    if head.contains(&0) {
        return None;
    }
    let t = String::from_utf8_lossy(head);
    if t.contains("-- PostgreSQL database dump")
        || t.contains("-- PostgreSQL database cluster dump")
    {
        return Some((DumpFormat::PgPlain, false));
    }
    if t.contains("-- MySQL dump") || t.contains("-- MariaDB dump") || t.starts_with("/*M!999999") {
        return Some((DumpFormat::MySql, false));
    }
    let up = t.to_ascii_uppercase();
    let sqlish = [
        "CREATE ", "INSERT ", "SET ", "DROP ", "ALTER ", "BEGIN", "COPY ",
    ]
    .iter()
    .any(|k| up.contains(k));
    if !sqlish {
        return None;
    }
    let my = t.matches('`').count() / 2
        + up.matches("ENGINE=").count() * 5
        + up.matches("AUTO_INCREMENT").count() * 3
        + t.matches("/*!").count() * 3;
    let pg = up.matches("FROM STDIN").count() * 5
        + t.matches("$$").count() * 2
        + t.matches("::").count()
        + up.matches("SEARCH_PATH").count() * 3
        + up.matches("CREATE EXTENSION").count() * 3
        + up.matches(" SERIAL").count() * 2;
    Some((
        if my > pg {
            DumpFormat::MySql
        } else {
            DumpFormat::PgPlain
        },
        true,
    ))
}

// ---------------------------------------------------------------- plain SQL

async fn read_plain(mut r: Reader, dump: &mut Dump) -> AppResult<()> {
    let d = if dump.info.format == DumpFormat::MySql {
        Dialect::My
    } else {
        Dialect::Pg
    };
    let mut s = dumpscan::Scanner::new(d, None, true);
    let mut buf = vec![0u8; 256 * 1024];
    let mut sink = Vec::new();
    let mut refused = None;
    loop {
        let n = r.read(&mut buf).await.map_err(io)?;
        if n == 0 {
            break;
        }
        if let Err(e) = s.feed(&buf[..n], &mut sink) {
            refused = Some(e);
            break;
        }
    }
    if refused.is_none() {
        if let Err(e) = s.finish(&mut sink) {
            refused = Some(e);
        }
    }
    let info = std::mem::take(&mut s.info);
    if let Some(e) = &refused {
        dump.info.refused = Some(e.message());
    }
    plain_to_info(info, dump);
    Ok(())
}

fn plain_to_info(p: PlainInfo, dump: &mut Dump) {
    let i = &mut dump.info;
    i.made_by = p.made_by.clone();
    i.server_version = p.server_version.clone();
    i.source_db = p
        .source_db
        .clone()
        .or_else(|| (p.databases.len() == 1).then(|| p.databases[0].clone()));
    i.encoding = p.encoding.clone();
    i.mariadb = p.mariadb_sandbox
        || p.made_by.as_deref().is_some_and(|m| m.contains("MariaDB"))
        || p.server_version
            .as_deref()
            .is_some_and(|v| v.contains("MariaDB"));
    i.databases = p.databases.clone();
    dump.dump_major = p
        .made_by
        .as_deref()
        .and_then(|m| m.strip_prefix("pg_dump "))
        .and_then(major);
    let mut seen = std::collections::BTreeSet::new();
    for o in &p.objects {
        let rows = (o.kind == "table").then(|| p.rows.get(&o.name)).flatten();
        if o.kind == "table" {
            seen.insert(o.name.clone());
            dump.tables.push(split_name(&o.name));
        }
        i.objects.push(DumpObject {
            kind: o.kind.clone(),
            database: o.database.clone(),
            name: o.name.clone(),
            rows: rows.map(|r| r.0 as f64),
            rows_exact: rows.is_some_and(|r| r.1),
        });
    }
    // Tables the file fills but doesn't create (data-only dumps).
    for (name, (n, exact)) in &p.rows {
        if !seen.contains(name) {
            dump.tables.push(split_name(name));
            i.objects.push(DumpObject {
                kind: "table data".into(),
                database: None,
                name: name.clone(),
                rows: Some(*n as f64),
                rows_exact: *exact,
            });
        }
    }
    i.objects_more = p.objects_more as f64;
    let pg = dump.info.engine == Engine::Postgres;
    let notes = &mut dump.info.notes;
    if dump.info.guessed {
        notes.push(note(
            "guessed",
            format!(
                "No tool header: this reads as {} SQL. That is a guess; check the target's engine.",
                if pg { "PostgreSQL" } else { "MySQL" }
            ),
        ));
    }
    if !p.owners.is_empty() {
        notes.push(note("owners", format!("Objects are owned by {}. Dalan skips ownership and privileges by default, so everything belongs to the user you sign in as.", list(&p.owners))));
    } else if p.grants > 0 {
        notes.push(note(
            "grants",
            format!("{} grants. Skipped by default.", p.grants),
        ));
    }
    if !p.extensions.is_empty() {
        notes.push(note(
            "extensions",
            format!(
                "Needs the extensions {}. Dalan checks the target can install them.",
                list(&p.extensions)
            ),
        ));
    }
    if p.databases.len() > 1 {
        notes.push(note(
            "databases",
            format!(
                "The file holds {} databases ({}). Pick one: only its part is restored.",
                p.databases.len(),
                list(&p.databases)
            ),
        ));
    } else if !p.databases.is_empty() {
        notes.push(note("connect", format!("The file creates or switches to the database {}. Those lines are left out, so everything lands in the target you choose.", p.databases[0])));
    }
    if p.definers > 0 {
        notes.push(note("definer", format!("{} views, routines or triggers name a DEFINER. Dalan removes it, so they belong to the user you sign in as.", p.definers)));
    }
    if p.roles > 0 {
        notes.push(note("roles", format!("{} role and tablespace statements (cluster-wide). Skipped unless you include roles.", p.roles)));
    }
    if p.superuser_roles > 0 {
        notes.push(note(
            "superuser",
            format!(
                "{} statements make a superuser. Dalan never restores those.",
                p.superuser_roles
            ),
        ));
    }
    if p.gtid > 0 {
        notes.push(note("gtid", "The file sets GTID_PURGED and SQL_LOG_BIN, which need admin rights. Those lines are left out.".to_string()));
    }
    if p.mariadb_sandbox {
        notes.push(note("sandbox", "Made by MariaDB's dump tool, which starts the file with a sandbox line. The MariaDB client reads it; for MySQL's client Dalan leaves it out.".to_string()));
    }
}

fn split_name(n: &str) -> (String, String) {
    match n.rsplit_once('.') {
        Some((s, t)) => (s.to_string(), t.to_string()),
        None => (String::new(), n.to_string()),
    }
}

fn list(v: &[String]) -> String {
    let shown: Vec<&str> = v.iter().take(5).map(String::as_str).collect();
    let more = v.len().saturating_sub(5);
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

fn note(code: &str, text: String) -> DumpNote {
    DumpNote {
        code: code.into(),
        text,
    }
}

/// "16.4" / "17.2 (Debian 17.2-1)" → 16 / 17.
pub fn major(v: &str) -> Option<u32> {
    v.trim()
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

// ---------------------------------------------------------------- PostgreSQL archives

/// The newest archive format this reader knows (pg_dump 17 writes 1.16).
const PG_MAX: (u8, u8) = (1, 16);

/// One entry of a pg archive's table of contents.
#[derive(Debug, Clone)]
pub struct TocEntry {
    pub id: i64,
    pub desc: String,
    pub tag: String,
    pub namespace: String,
    pub owner: String,
    pub defn: String,
    pub copy_stmt: String,
}

pub struct PgArchive {
    pub version: (u8, u8, u8),
    pub format: u8,
    pub compression: String,
    pub created: Option<String>,
    pub dbname: Option<String>,
    pub server_version: Option<String>,
    pub dump_version: Option<String>,
    /// `None` when the format is newer than this reader.
    pub entries: Option<Vec<TocEntry>>,
}

struct Ar<'a> {
    r: &'a mut Reader,
    ver: (u8, u8),
    int_size: usize,
    off_size: usize,
}

fn bad_archive() -> AppError {
    AppError::new(
        ErrorCode::DbDumpUnknown,
        "This PostgreSQL archive is damaged or cut short: its table of contents can't be read.",
    )
}

impl Ar<'_> {
    async fn byte(&mut self) -> AppResult<u8> {
        self.r.read_u8().await.map_err(|_| bad_archive())
    }
    async fn int(&mut self) -> AppResult<i64> {
        let sign = if self.ver > (1, 0) {
            self.byte().await?
        } else {
            0
        };
        let mut v: i64 = 0;
        for i in 0..self.int_size {
            let b = i64::from(self.byte().await?);
            if i < 8 {
                v |= b << (8 * i);
            }
        }
        Ok(if sign != 0 { -v } else { v })
    }
    async fn str(&mut self) -> AppResult<Option<String>> {
        let n = self.int().await?;
        if n < 0 {
            return Ok(None);
        }
        // A function body can be large; a table of contents string past 256 MB is damage.
        if n > 256 << 20 {
            return Err(bad_archive());
        }
        let mut buf = vec![0u8; n as usize];
        self.r
            .read_exact(&mut buf)
            .await
            .map_err(|_| bad_archive())?;
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }
    async fn s(&mut self) -> AppResult<String> {
        Ok(self.str().await?.unwrap_or_default())
    }
}

/// Reads a pg archive's header and table of contents (custom format, or a tar/directory dump's `toc.dat`).
pub async fn read_pg_toc(mut r: Reader) -> AppResult<PgArchive> {
    let mut magic = [0u8; 5];
    r.read_exact(&mut magic).await.map_err(|_| bad_archive())?;
    if &magic != b"PGDMP" {
        return Err(bad_archive());
    }
    let mut a = Ar {
        r: &mut r,
        ver: (1, 0),
        int_size: 4,
        off_size: 4,
    };
    let vmaj = a.byte().await?;
    let vmin = a.byte().await?;
    let vrev = if vmaj > 1 || (vmaj == 1 && vmin > 0) {
        a.byte().await?
    } else {
        0
    };
    a.ver = (vmaj, vmin);
    if a.ver < (1, 0) {
        return Err(bad_archive());
    }
    a.int_size = a.byte().await? as usize;
    if a.int_size == 0 || a.int_size > 32 {
        return Err(bad_archive());
    }
    a.off_size = if a.ver >= (1, 7) {
        a.byte().await? as usize
    } else {
        a.int_size
    };
    let format = a.byte().await?;
    let compression = if a.ver >= (1, 15) {
        match a.byte().await? {
            0 => "none",
            1 => "gzip",
            2 => "lz4",
            3 => "zstd",
            _ => "unknown",
        }
        .to_string()
    } else if a.ver >= (1, 2) {
        let level = if a.ver < (1, 4) {
            i64::from(a.byte().await?)
        } else {
            a.int().await?
        };
        if level == 0 { "none" } else { "gzip" }.to_string()
    } else {
        "none".to_string()
    };
    let mut created = None;
    let mut dbname = None;
    if a.ver >= (1, 4) {
        let mut t = [0i64; 7];
        for x in &mut t {
            *x = a.int().await?;
        }
        // sec, min, hour, mday, mon (0-based), year (since 1900), isdst
        created = Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            t[5] + 1900,
            t[4] + 1,
            t[3],
            t[2],
            t[1]
        ));
        dbname = a.str().await?;
    }
    let (mut server_version, mut dump_version) = (None, None);
    if a.ver >= (1, 10) {
        server_version = a.str().await?;
        dump_version = a.str().await?;
    }
    let mut out = PgArchive {
        version: (vmaj, vmin, vrev),
        format,
        compression,
        created,
        dbname,
        server_version,
        dump_version,
        entries: None,
    };
    if (vmaj, vmin) > PG_MAX {
        return Ok(out);
    }
    let count = a.int().await?;
    if !(0..=10_000_000).contains(&count) {
        return Err(bad_archive());
    }
    let mut entries = Vec::with_capacity(count.min(100_000) as usize);
    for _ in 0..count {
        let id = a.int().await?;
        let _had_dumper = a.int().await?;
        if a.ver >= (1, 8) {
            a.str().await?;
        }
        a.str().await?; // oid
        let tag = a.s().await?;
        let desc = a.s().await?;
        if a.ver >= (1, 11) {
            a.int().await?; // section
        }
        let defn = a.s().await?;
        a.str().await?; // drop statement
        let copy_stmt = if a.ver >= (1, 3) {
            a.s().await?
        } else {
            String::new()
        };
        let namespace = if a.ver >= (1, 6) {
            a.s().await?
        } else {
            String::new()
        };
        if a.ver >= (1, 10) {
            a.str().await?; // tablespace
        }
        if a.ver >= (1, 14) {
            a.str().await?; // table access method
        }
        if a.ver >= (1, 16) {
            a.int().await?; // relkind
        }
        let owner = a.s().await?;
        if a.ver >= (1, 9) {
            a.str().await?; // "with oids"
        }
        if a.ver >= (1, 5) {
            while a.str().await?.is_some() {}
        }
        match format {
            // Custom: where the data is (a flag byte, then the offset).
            1 => {
                if a.ver >= (1, 7) {
                    a.byte().await?;
                    for _ in 0..a.off_size {
                        a.byte().await?;
                    }
                } else {
                    a.int().await?;
                }
            }
            // Tar and directory: the data file's name.
            3 | 5 => {
                a.str().await?;
            }
            _ => return Err(bad_archive()),
        }
        entries.push(TocEntry {
            id,
            desc,
            tag,
            namespace,
            owner,
            defn,
            copy_stmt,
        });
    }
    out.entries = Some(entries);
    Ok(out)
}

/// The kinds of TOC entry shown as objects, and their preview name.
fn toc_kind(desc: &str) -> Option<&'static str> {
    Some(match desc {
        "TABLE" => "table",
        "VIEW" => "view",
        "MATERIALIZED VIEW" => "materialized view",
        "FUNCTION" => "function",
        "PROCEDURE" => "procedure",
        "AGGREGATE" => "aggregate",
        "TRIGGER" => "trigger",
        "SEQUENCE" => "sequence",
        "TYPE" => "type",
        "DOMAIN" => "domain",
        "SCHEMA" => "schema",
        "EXTENSION" => "extension",
        "INDEX" => "index",
        "FOREIGN TABLE" => "foreign table",
        _ => return None,
    })
}

async fn read_pg_archive(r: Reader, dump: &mut Dump) -> AppResult<()> {
    let ar = read_pg_toc(r).await?;
    let i = &mut dump.info;
    dump.archive_version = Some((ar.version.0, ar.version.1));
    dump.dump_major = ar.dump_version.as_deref().and_then(major);
    i.made_by = ar.dump_version.as_ref().map(|v| format!("pg_dump {v}"));
    i.server_version = ar
        .server_version
        .as_ref()
        .map(|v| format!("PostgreSQL {v}"));
    i.source_db = ar.dbname.clone();
    i.created = ar.created.clone();
    if ar.compression != "none" {
        i.notes.push(note(
            "inner",
            format!("Compressed inside the archive with {}.", ar.compression),
        ));
    }
    let Some(entries) = ar.entries else {
        i.notes.push(note("toc", format!("The archive's format ({}.{}) is newer than Dalan's reader, so its contents can't be listed. pg_restore on the target still checks it.", ar.version.0, ar.version.1)));
        return Ok(());
    };
    let mut owners: Vec<String> = Vec::new();
    let mut extensions: Vec<String> = Vec::new();
    let mut grants = 0u64;
    for e in &entries {
        if !e.owner.is_empty()
            && !owners.contains(&e.owner)
            && owners.len() < 50
            && e.desc != "DATABASE"
        {
            owners.push(e.owner.clone());
        }
        if e.desc == "ACL" || e.desc == "DEFAULT ACL" {
            grants += 1;
        }
        if e.desc == "ENCODING" {
            i.encoding = e.defn.split('\'').nth(1).map(str::to_string);
        }
        if let Some(kind) = toc_kind(&e.desc) {
            if kind == "extension" {
                extensions.push(e.tag.clone());
            }
            let name = if e.namespace.is_empty()
                || kind == "trigger"
                || kind == "schema"
                || kind == "extension"
            {
                e.tag.clone()
            } else {
                format!("{}.{}", e.namespace, e.tag)
            };
            if kind == "table" {
                dump.tables.push((e.namespace.clone(), e.tag.clone()));
            }
            if i.objects.len() < 5000 {
                i.objects.push(DumpObject {
                    kind: kind.into(),
                    database: None,
                    name,
                    rows: None,
                    rows_exact: false,
                });
            } else {
                i.objects_more += 1.0;
            }
        }
        // The SQL pg_restore will run: the same statement rules as a plain dump (no client here, so no commands).
        for sql in [&e.defn, &e.copy_stmt] {
            if sql.is_empty() {
                continue;
            }
            if let Err(r) = dumpscan::scan_all(Dialect::Pg, sql.as_bytes(), None, false) {
                if i.refused.is_none() {
                    i.refused = Some(format!(
                        "The archive's entry {} ({} {}) {}. Dalan won't restore this file.",
                        e.id,
                        e.desc.to_lowercase(),
                        e.tag,
                        r.what
                    ));
                }
            }
        }
    }
    let owners: Vec<String> = owners.into_iter().filter(|o| o != "postgres").collect();
    if !owners.is_empty() {
        i.notes.push(note("owners", format!("Objects are owned by {}. Dalan skips ownership and privileges by default (--no-owner --no-privileges), so everything belongs to the user you sign in as.", list(&owners))));
    } else if grants > 0 {
        i.notes.push(note(
            "grants",
            format!("{grants} grants. Skipped by default (--no-privileges)."),
        ));
    }
    if !extensions.is_empty() {
        i.notes.push(note(
            "extensions",
            format!(
                "Needs the extensions {}. Dalan checks the target can install them.",
                list(&extensions)
            ),
        ));
    }
    i.notes.push(note("rows", "Row counts aren't listed for archives: the data is compressed inside and is read only by pg_restore.".to_string()));
    Ok(())
}

// ---------------------------------------------------------------- MongoDB

pub const MONGO_MAGIC: [u8; 4] = [0x6d, 0xe2, 0x99, 0x81];
const TERMINATOR: i32 = -1;

fn bad_mongo() -> AppError {
    AppError::new(
        ErrorCode::DbDumpUnknown,
        "This MongoDB archive is damaged or cut short.",
    )
}

/// The next length-prefixed BSON document, or `None` at a terminator (`-1`).
async fn bson_doc(r: &mut Reader, keep: bool) -> AppResult<Option<Option<Document>>> {
    let len = match r.read_i32_le().await {
        Ok(l) => l,
        Err(_) => return Err(bad_mongo()),
    };
    if len == TERMINATOR {
        return Ok(None);
    }
    if !(5..=48 << 20).contains(&len) {
        return Err(bad_mongo());
    }
    let mut body = vec![0u8; len as usize];
    body[..4].copy_from_slice(&len.to_le_bytes());
    r.read_exact(&mut body[4..])
        .await
        .map_err(|_| bad_mongo())?;
    if !keep {
        return Ok(Some(None));
    }
    Document::from_reader(&mut body.as_slice())
        .map(|d| Some(Some(d)))
        .map_err(|_| bad_mongo())
}

#[derive(Default)]
struct MongoColl {
    db: String,
    name: String,
    kind: String,
    indexes: usize,
    docs: u64,
}

fn coll_from_metadata(db: &str, name: &str, kind: Option<&str>, metadata: &str) -> MongoColl {
    let v: serde_json::Value = serde_json::from_str(metadata).unwrap_or_default();
    let view = v.get("options").and_then(|o| o.get("viewOn")).is_some();
    let ts = v.get("options").and_then(|o| o.get("timeseries")).is_some();
    MongoColl {
        db: db.into(),
        name: name.into(),
        kind: match kind {
            Some("view") => "view".into(),
            Some("timeseries") => "time series".into(),
            _ if view => "view".into(),
            _ if ts => "time series".into(),
            _ => "collection".into(),
        },
        indexes: v
            .get("indexes")
            .and_then(|i| i.as_array())
            .map_or(0, Vec::len),
        docs: 0,
    }
}

fn mongo_to_info(colls: Vec<MongoColl>, dump: &mut Dump) {
    let i = &mut dump.info;
    for c in &colls {
        if !i.databases.contains(&c.db) {
            i.databases.push(c.db.clone());
        }
    }
    i.source_db = (i.databases.len() == 1).then(|| i.databases[0].clone());
    let indexes: usize = colls.iter().map(|c| c.indexes).sum();
    for c in colls {
        i.objects.push(DumpObject {
            kind: c.kind.clone(),
            database: Some(c.db.clone()),
            name: c.name.clone(),
            rows: (c.kind != "view").then_some(c.docs as f64),
            rows_exact: true,
        });
        dump.tables.push((c.db, c.name));
    }
    if indexes > 0 {
        i.notes.push(note(
            "indexes",
            format!("{indexes} index definitions; mongorestore rebuilds them after the documents."),
        ));
    }
    if i.databases.len() > 1 {
        i.notes.push(note(
            "databases",
            format!(
                "The dump holds {} databases ({}). Pick one: only its collections are restored.",
                i.databases.len(),
                list(&i.databases)
            ),
        ));
    }
}

/// `mongodump --archive`: the prelude (header, then one metadata document per collection, then a terminator), then
/// blocks of documents, each after a namespace header and ended by a terminator. Documents are counted, not read.
async fn read_mongo_archive(mut r: Reader, dump: &mut Dump) -> AppResult<()> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).await.map_err(|_| bad_mongo())?;
    let header = bson_doc(&mut r, true)
        .await?
        .flatten()
        .ok_or_else(bad_mongo)?;
    dump.info.server_version = header
        .get_str("server_version")
        .ok()
        .map(|v| format!("MongoDB {v}"));
    dump.info.made_by = header
        .get_str("tool_version")
        .ok()
        .map(|v| format!("mongodump {v}"));
    let mut colls: Vec<MongoColl> = Vec::new();
    while let Some(d) = bson_doc(&mut r, true).await? {
        let d = d.ok_or_else(bad_mongo)?;
        colls.push(coll_from_metadata(
            d.get_str("db").unwrap_or(""),
            d.get_str("collection").unwrap_or(""),
            d.get_str("type").ok(),
            d.get_str("metadata").unwrap_or("{}"),
        ));
        if colls.len() > 100_000 {
            return Err(bad_mongo());
        }
    }
    // The body: until the stream ends.
    loop {
        let ns = match r.read_i32_le().await {
            Err(_) => break,
            Ok(len) => {
                if !(5..=16 << 20).contains(&len) {
                    return Err(bad_mongo());
                }
                let mut body = vec![0u8; len as usize];
                body[..4].copy_from_slice(&len.to_le_bytes());
                r.read_exact(&mut body[4..])
                    .await
                    .map_err(|_| bad_mongo())?;
                Document::from_reader(&mut body.as_slice()).map_err(|_| bad_mongo())?
            }
        };
        let (db, coll) = (
            ns.get_str("db").unwrap_or("").to_string(),
            ns.get_str("collection").unwrap_or("").to_string(),
        );
        let eof = ns.get_bool("EOF").unwrap_or(false);
        let mut n = 0u64;
        if !eof {
            while bson_doc(&mut r, false).await?.is_some() {
                n += 1;
            }
        } else {
            // An EOF header is followed by its own terminator.
            let _ = bson_doc(&mut r, false).await?;
        }
        if let Some(c) = colls.iter_mut().find(|c| c.db == db && c.name == coll) {
            c.docs += n;
        }
    }
    mongo_to_info(colls, dump);
    Ok(())
}

/// `mongodump --out`: `<top>/<db>/<coll>.bson[.gz]` and `<coll>.metadata.json[.gz]`.
async fn read_mongo_folder(db_dir: &Path, dump: &mut Dump) -> AppResult<()> {
    let db = db_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The folder to hand mongorestore is the dump's top (the database folder's parent).
    if let Some(top) = db_dir.parent() {
        dump.folder = Some(top.to_path_buf());
    }
    for p in [
        db_dir.join("prelude.json"),
        db_dir
            .parent()
            .map(|t| t.join("prelude.json"))
            .unwrap_or_default(),
    ] {
        if let Ok(t) = tokio::fs::read_to_string(&p).await {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
            dump.info.server_version = v
                .get("ServerVersion")
                .and_then(|x| x.as_str())
                .map(|x| format!("MongoDB {x}"));
            dump.info.made_by = v
                .get("ToolVersion")
                .and_then(|x| x.as_str())
                .map(|x| format!("mongodump {x}"));
            break;
        }
    }
    let mut names: Vec<String> = std::fs::read_dir(db_dir)
        .map_err(io)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut colls = Vec::new();
    for n in &names {
        let Some(coll) = n
            .strip_suffix(".metadata.json")
            .or_else(|| n.strip_suffix(".metadata.json.gz"))
        else {
            continue;
        };
        let gz = n.ends_with(".gz");
        let meta = read_small(&db_dir.join(n), gz).await.unwrap_or_default();
        let mut c = coll_from_metadata(&db, coll, None, &meta);
        for (bson_name, gz) in [
            (format!("{coll}.bson"), false),
            (format!("{coll}.bson.gz"), true),
        ] {
            let p = db_dir.join(&bson_name);
            if p.is_file() {
                dump.mongo_gzip |= gz;
                let mut r = open(&p, gz.then_some(Outer::Gzip)).await?;
                let mut docs = 0;
                loop {
                    match r.read_i32_le().await {
                        Err(_) => break,
                        Ok(len) if (5..=48 << 20).contains(&len) => {
                            let mut skip = (&mut r).take(len as u64 - 4);
                            let got = tokio::io::copy(&mut skip, &mut tokio::io::sink())
                                .await
                                .map_err(io)?;
                            if got != len as u64 - 4 {
                                return Err(bad_mongo());
                            }
                            docs += 1;
                        }
                        Ok(_) => return Err(bad_mongo()),
                    }
                }
                c.docs = docs;
            }
        }
        colls.push(c);
    }
    // Collections with data but no metadata file.
    for n in &names {
        let Some(coll) = n
            .strip_suffix(".bson")
            .or_else(|| n.strip_suffix(".bson.gz"))
        else {
            continue;
        };
        if !colls.iter().any(|c| c.name == coll) {
            colls.push(MongoColl {
                db: db.clone(),
                name: coll.to_string(),
                kind: "collection".into(),
                ..MongoColl::default()
            });
        }
    }
    mongo_to_info(colls, dump);
    Ok(())
}

async fn read_small(p: &Path, gz: bool) -> AppResult<String> {
    let mut r = open(p, gz.then_some(Outer::Gzip)).await?;
    let mut s = String::new();
    (&mut r)
        .take(16 << 20)
        .read_to_string(&mut s)
        .await
        .map_err(io)?;
    Ok(s)
}

#[cfg(test)]
#[path = "dumpfile_tests.rs"]
mod tests;
