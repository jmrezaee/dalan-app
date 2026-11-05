//! File operations on either place: the local disk, or a remote host over SFTP.

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{EntryKind, FsEntry};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::Connected;

/// An open SFTP session. Dropping it closes the SSH connection (and any jump hosts) under it.
pub struct RemoteFs {
    pub sftp: SftpSession,
    pub label: String,
    _conn: Connected,
}

impl RemoteFs {
    pub fn new(sftp: SftpSession, label: String, conn: Connected) -> Self {
        Self {
            sftp,
            label,
            _conn: conn,
        }
    }
}

#[derive(Clone)]
pub enum Fs {
    Local,
    Remote(Arc<RemoteFs>),
}

pub type Reader = Pin<Box<dyn AsyncRead + Send>>;
pub type Writer = Pin<Box<dyn AsyncWrite + Send>>;

/// What a transfer needs to know about one path.
pub struct Stat {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: Option<i64>,
}

fn io_err(e: &io::Error, path: &str) -> AppError {
    match e.kind() {
        io::ErrorKind::NotFound => AppError::not_found(format!("{path} doesn't exist.")),
        io::ErrorKind::PermissionDenied => AppError::new(
            ErrorCode::PermissionDenied,
            format!("Permission denied: {path}."),
        ),
        io::ErrorKind::AlreadyExists => AppError::validation(format!("{path} already exists.")),
        _ => AppError::internal(format!("{path}: {e}")),
    }
}

fn sftp_err(e: &russh_sftp::client::error::Error, path: &str) -> AppError {
    let text = e.to_string().to_lowercase();
    if text.contains("permission") {
        AppError::new(
            ErrorCode::PermissionDenied,
            format!("Permission denied: {path}."),
        )
    } else if text.contains("no such") || text.contains("not found") {
        AppError::not_found(format!("{path} doesn't exist."))
    } else if text.contains("exist") {
        AppError::validation(format!("{path} already exists."))
    } else {
        AppError::internal(format!("{path}: {e}"))
    }
}

/// Joins a directory and a name the way the place expects (`/` on a server, the OS separator locally).
pub fn join(fs: &Fs, dir: &str, name: &str) -> String {
    match fs {
        Fs::Local => Path::new(dir).join(name).to_string_lossy().into_owned(),
        Fs::Remote(_) => format!("{}/{}", dir.trim_end_matches('/'), name),
    }
}

pub fn basename(fs: &Fs, path: &str) -> String {
    match fs {
        Fs::Local => Path::new(path)
            .file_name()
            .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned()),
        Fs::Remote(_) => path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string(),
    }
}

fn ms(t: std::time::SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

#[cfg(unix)]
fn local_mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
fn local_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

fn kind_of(m: &std::fs::Metadata) -> EntryKind {
    if m.is_dir() {
        EntryKind::Dir
    } else if m.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Boxed so the recursive delete can call itself.
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl Fs {
    /// Where a new pane starts: your home folder.
    pub async fn home(&self) -> AppResult<String> {
        match self {
            Fs::Local => Ok(std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| "/".into())),
            Fs::Remote(r) => r
                .sftp
                .canonicalize(".")
                .await
                .map_err(|e| sftp_err(&e, ".")),
        }
    }

    pub async fn list(&self, path: &str, show_hidden: bool) -> AppResult<Vec<FsEntry>> {
        let mut out = match self {
            Fs::Local => {
                let p = path.to_string();
                tokio::task::spawn_blocking(move || -> AppResult<Vec<FsEntry>> {
                    let mut out = Vec::new();
                    for e in std::fs::read_dir(&p).map_err(|e| io_err(&e, &p))? {
                        let Ok(e) = e else { continue };
                        let Ok(link) = e.path().symlink_metadata() else {
                            continue;
                        };
                        // Follow links to learn what they point at; a broken link is shown as "other".
                        let target = e.metadata().ok();
                        let m = target.as_ref().unwrap_or(&link);
                        out.push(FsEntry {
                            name: e.file_name().to_string_lossy().into_owned(),
                            kind: if target.is_some() {
                                kind_of(m)
                            } else {
                                EntryKind::Other
                            },
                            symlink: link.file_type().is_symlink(),
                            size: if m.is_file() { m.len() } else { 0 },
                            mtime: m.modified().ok().and_then(ms),
                            mode: local_mode(m),
                        });
                    }
                    Ok(out)
                })
                .await??
            }
            Fs::Remote(r) => {
                let mut out = Vec::new();
                for e in r
                    .sftp
                    .read_dir(path)
                    .await
                    .map_err(|e| sftp_err(&e, path))?
                {
                    let attrs = e.metadata();
                    let mut kind = match e.file_type() {
                        russh_sftp::protocol::FileType::Dir => EntryKind::Dir,
                        russh_sftp::protocol::FileType::File => EntryKind::File,
                        _ => EntryKind::Other,
                    };
                    let symlink = e.file_type().is_symlink();
                    let mut attrs = attrs;
                    if symlink {
                        // Ask what the link points at, so folders behind links open like folders.
                        match r.sftp.metadata(e.path()).await {
                            Ok(t) => {
                                kind = match t.file_type() {
                                    russh_sftp::protocol::FileType::Dir => EntryKind::Dir,
                                    russh_sftp::protocol::FileType::File => EntryKind::File,
                                    _ => EntryKind::Other,
                                };
                                attrs = t;
                            }
                            Err(_) => kind = EntryKind::Other,
                        }
                    }
                    out.push(FsEntry {
                        name: e.file_name(),
                        kind,
                        symlink,
                        size: if kind == EntryKind::File {
                            attrs.size.unwrap_or(0)
                        } else {
                            0
                        },
                        mtime: attrs.mtime.map(|t| i64::from(t) * 1000),
                        mode: attrs.permissions.map(|p| p & 0o7777),
                    });
                }
                out
            }
        };
        if !show_hidden {
            out.retain(|e| !e.name.starts_with('.'));
        }
        Ok(out)
    }

    pub async fn stat(&self, path: &str) -> AppResult<Stat> {
        match self {
            Fs::Local => {
                let m = tokio::fs::metadata(path)
                    .await
                    .map_err(|e| io_err(&e, path))?;
                Ok(Stat {
                    kind: kind_of(&m),
                    size: m.len(),
                    mtime: m.modified().ok().and_then(ms),
                })
            }
            Fs::Remote(r) => {
                let a = r
                    .sftp
                    .metadata(path)
                    .await
                    .map_err(|e| sftp_err(&e, path))?;
                let kind = match a.file_type() {
                    russh_sftp::protocol::FileType::Dir => EntryKind::Dir,
                    russh_sftp::protocol::FileType::File => EntryKind::File,
                    _ => EntryKind::Other,
                };
                Ok(Stat {
                    kind,
                    size: a.size.unwrap_or(0),
                    mtime: a.mtime.map(|t| i64::from(t) * 1000),
                })
            }
        }
    }

    pub async fn exists(&self, path: &str) -> bool {
        match self {
            Fs::Local => tokio::fs::symlink_metadata(path).await.is_ok(),
            Fs::Remote(r) => r.sftp.try_exists(path).await.unwrap_or(false),
        }
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        match self {
            Fs::Local => tokio::fs::create_dir(path)
                .await
                .map_err(|e| io_err(&e, path)),
            Fs::Remote(r) => r
                .sftp
                .create_dir(path)
                .await
                .map_err(|e| sftp_err(&e, path)),
        }
    }

    /// `mkdir` that is fine with the folder already being there (merging into an existing folder).
    pub async fn ensure_dir(&self, path: &str) -> AppResult<()> {
        if self.exists(path).await {
            return Ok(());
        }
        self.mkdir(path).await
    }

    pub async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        if self.exists(to).await {
            return Err(AppError::validation(format!(
                "{} already exists.",
                basename(self, to)
            )));
        }
        match self {
            Fs::Local => tokio::fs::rename(from, to)
                .await
                .map_err(|e| io_err(&e, from)),
            Fs::Remote(r) => r
                .sftp
                .rename(from, to)
                .await
                .map_err(|e| sftp_err(&e, from)),
        }
    }

    /// Deletes a file, or a folder and everything in it.
    pub fn remove<'a>(&'a self, path: &'a str) -> BoxFut<'a, AppResult<()>> {
        Box::pin(async move {
            match self {
                Fs::Local => {
                    let m = tokio::fs::symlink_metadata(path)
                        .await
                        .map_err(|e| io_err(&e, path))?;
                    if m.is_dir() {
                        tokio::fs::remove_dir_all(path)
                            .await
                            .map_err(|e| io_err(&e, path))
                    } else {
                        tokio::fs::remove_file(path)
                            .await
                            .map_err(|e| io_err(&e, path))
                    }
                }
                Fs::Remote(r) => {
                    let a = r
                        .sftp
                        .symlink_metadata(path)
                        .await
                        .map_err(|e| sftp_err(&e, path))?;
                    if a.file_type().is_dir() {
                        for e in self.list(path, true).await? {
                            self.remove(&join(self, path, &e.name)).await?;
                        }
                        r.sftp
                            .remove_dir(path)
                            .await
                            .map_err(|e| sftp_err(&e, path))
                    } else {
                        r.sftp
                            .remove_file(path)
                            .await
                            .map_err(|e| sftp_err(&e, path))
                    }
                }
            }
        })
    }

    /// Sets the permission bits (like `chmod 644`).
    pub async fn chmod(&self, path: &str, mode: u32) -> AppResult<()> {
        let mode = mode & 0o7777;
        match self {
            #[cfg(unix)]
            Fs::Local => {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                    .await
                    .map_err(|e| io_err(&e, path))
            }
            #[cfg(not(unix))]
            Fs::Local => Err(AppError::validation(
                "This computer doesn't use Unix permissions.",
            )),
            Fs::Remote(r) => {
                let attrs = FileAttributes {
                    permissions: Some(mode),
                    ..FileAttributes::empty()
                };
                r.sftp
                    .set_metadata(path, attrs)
                    .await
                    .map_err(|e| sftp_err(&e, path))
            }
        }
    }

    pub async fn open_read(&self, path: &str) -> AppResult<Reader> {
        match self {
            Fs::Local => Ok(Box::pin(
                tokio::fs::File::open(path)
                    .await
                    .map_err(|e| io_err(&e, path))?,
            )),
            Fs::Remote(r) => Ok(Box::pin(
                r.sftp.open(path).await.map_err(|e| sftp_err(&e, path))?,
            )),
        }
    }

    /// Creates (or truncates) a file for writing.
    pub async fn open_write(&self, path: &str) -> AppResult<Writer> {
        match self {
            Fs::Local => Ok(Box::pin(
                tokio::fs::File::create(path)
                    .await
                    .map_err(|e| io_err(&e, path))?,
            )),
            Fs::Remote(r) => Ok(Box::pin(
                r.sftp.create(path).await.map_err(|e| sftp_err(&e, path))?,
            )),
        }
    }
}
