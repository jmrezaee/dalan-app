//! The transfer queue (S22): copy files and folders between any two places (disk or server),
//! with progress, conflicts the user decides, and cancel (which removes the half-written file).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, watch};

use super::fs::{basename, join, Fs};
use super::EntryKind;
use crate::error::{AppError, AppResult};

const CHUNK: usize = 128 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize, Type)]
pub struct TransferItem {
    pub src_endpoint: String,
    pub src_path: String,
    pub dst_endpoint: String,
    pub dst_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Replace,
    KeepBoth,
    Skip,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransferEvent {
    /// An item (a file, or a whole folder) is about to be copied. `total` is its size in bytes.
    Started {
        item: u32,
        name: String,
        total: f64,
    },
    Progress {
        item: u32,
        bytes: f64,
        total: f64,
    },
    /// The destination already has a file with this name. Answer with `transfer_resolve`.
    Conflict {
        item: u32,
        name: String,
        src_size: f64,
        src_mtime: Option<f64>,
        dest_size: f64,
        dest_mtime: Option<f64>,
    },
    ItemDone {
        item: u32,
    },
    ItemFailed {
        item: u32,
        message: String,
    },
    Done {
        cancelled: bool,
    },
}

pub type Emit = Arc<dyn Fn(TransferEvent) + Send + Sync>;

/// One copy job, with both places already resolved.
pub struct Job {
    pub src: Fs,
    pub src_path: String,
    pub dst: Fs,
    pub dst_dir: String,
}

struct Shared {
    cancel: watch::Sender<bool>,
    resolver: Mutex<Option<oneshot::Sender<(Choice, bool)>>>,
}

#[derive(Default)]
pub struct TransferManager {
    running: Mutex<HashMap<String, Arc<Shared>>>,
}

enum Stop {
    Cancelled,
    Failed(String),
}

impl From<AppError> for Stop {
    fn from(e: AppError) -> Self {
        Stop::Failed(e.message)
    }
}

impl From<std::io::Error> for Stop {
    fn from(e: std::io::Error) -> Self {
        Stop::Failed(e.to_string())
    }
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl TransferManager {
    pub fn start(&self, jobs: Vec<Job>, emit: Emit) -> AppResult<String> {
        let id = uuid::Uuid::now_v7().to_string();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            cancel: cancel_tx,
            resolver: Mutex::new(None),
        });
        self.running
            .lock()
            .map_err(|_| AppError::internal("transfer lock poisoned"))?
            .insert(id.clone(), Arc::clone(&shared));
        tauri::async_runtime::spawn(run_all(jobs, emit, cancel_rx, shared));
        Ok(id)
    }

    pub fn resolve(&self, id: &str, choice: Choice, apply_to_all: bool) -> AppResult<()> {
        let shared = self.get(id)?;
        let tx = shared
            .resolver
            .lock()
            .map_err(|_| AppError::internal("transfer lock poisoned"))?
            .take();
        match tx {
            Some(tx) => {
                let _ = tx.send((choice, apply_to_all));
                Ok(())
            }
            None => Err(AppError::not_found("Nothing is waiting for an answer.")),
        }
    }

    pub fn cancel(&self, id: &str) -> AppResult<()> {
        let _ = self.get(id)?.cancel.send(true);
        Ok(())
    }

    pub fn cancel_all(&self) {
        if let Ok(m) = self.running.lock() {
            for s in m.values() {
                let _ = s.cancel.send(true);
            }
        }
    }

    fn get(&self, id: &str) -> AppResult<Arc<Shared>> {
        self.running
            .lock()
            .map_err(|_| AppError::internal("transfer lock poisoned"))?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("That transfer has finished."))
    }
}

/// State for the item being copied, plus the transfer-wide "apply to all" answer.
struct Run {
    emit: Emit,
    cancel: watch::Receiver<bool>,
    shared: Arc<Shared>,
    apply_all: Option<Choice>,
    item: u32,
    total: u64,
    done: u64,
    last: Instant,
}

impl Run {
    fn cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    fn progress(&mut self, force: bool) {
        if force || self.last.elapsed() >= PROGRESS_EVERY {
            self.last = Instant::now();
            (self.emit)(TransferEvent::Progress {
                item: self.item,
                bytes: self.done as f64,
                total: self.total as f64,
            });
        }
    }
}

async fn run_all(jobs: Vec<Job>, emit: Emit, cancel: watch::Receiver<bool>, shared: Arc<Shared>) {
    let mut run = Run {
        emit: Arc::clone(&emit),
        cancel,
        shared,
        apply_all: None,
        item: 0,
        total: 0,
        done: 0,
        last: Instant::now(),
    };
    for (i, job) in jobs.iter().enumerate() {
        if run.cancelled() {
            break;
        }
        run.item = i as u32;
        run.done = 0;
        // The first chunk of every item reports at once, so a bar appears straight away.
        run.last = Instant::now()
            .checked_sub(PROGRESS_EVERY)
            .unwrap_or_else(Instant::now);
        let name = basename(&job.src, &job.src_path);
        run.total = match total_size(&job.src, &job.src_path).await {
            Ok(t) => t,
            Err(e) => {
                emit(TransferEvent::ItemFailed {
                    item: i as u32,
                    message: e.message,
                });
                continue;
            }
        };
        emit(TransferEvent::Started {
            item: i as u32,
            name: name.clone(),
            total: run.total as f64,
        });
        match copy_entry(
            job,
            &mut run,
            job.src_path.clone(),
            job.dst_dir.clone(),
            name,
        )
        .await
        {
            Ok(()) => {
                run.progress(true);
                emit(TransferEvent::ItemDone { item: i as u32 });
            }
            Err(Stop::Cancelled) => break,
            Err(Stop::Failed(message)) => emit(TransferEvent::ItemFailed {
                item: i as u32,
                message,
            }),
        }
    }
    emit(TransferEvent::Done {
        cancelled: run.cancelled(),
    });
}

/// Bytes in a file, or in every file under a folder (links to folders are not followed).
fn total_size<'a>(fs: &'a Fs, path: &'a str) -> BoxFut<'a, AppResult<u64>> {
    Box::pin(async move {
        let st = fs.stat(path).await?;
        match st.kind {
            EntryKind::File => Ok(st.size),
            EntryKind::Dir => {
                let mut sum = 0;
                for e in fs.list(path, true).await? {
                    if e.symlink && e.kind == EntryKind::Dir {
                        continue;
                    }
                    sum += total_size(fs, &join(fs, path, &e.name)).await?;
                }
                Ok(sum)
            }
            EntryKind::Other => Ok(0),
        }
    })
}

fn copy_entry<'a>(
    job: &'a Job,
    run: &'a mut Run,
    src_path: String,
    dst_dir: String,
    name: String,
) -> BoxFut<'a, Result<(), Stop>> {
    Box::pin(async move {
        if run.cancelled() {
            return Err(Stop::Cancelled);
        }
        let st = job.src.stat(&src_path).await?;
        match st.kind {
            EntryKind::Dir => {
                let dst_path = join(&job.dst, &dst_dir, &name);
                job.dst.ensure_dir(&dst_path).await?; // merging into a folder that exists is fine
                for e in job.src.list(&src_path, true).await? {
                    if e.symlink && e.kind == EntryKind::Dir {
                        continue;
                    }
                    copy_entry(
                        job,
                        run,
                        join(&job.src, &src_path, &e.name),
                        dst_path.clone(),
                        e.name,
                    )
                    .await?;
                }
                Ok(())
            }
            EntryKind::File => {
                copy_file(job, run, &src_path, &dst_dir, &name, st.size, st.mtime).await
            }
            EntryKind::Other => Ok(()), // sockets, devices: nothing to copy
        }
    })
}

/// `a.txt` -> `a (copy).txt`, then `a (copy 2).txt`, ... the first name that is free.
async fn free_name(fs: &Fs, dir: &str, name: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = if n == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {n}){ext}")
        };
        if !fs.exists(&join(fs, dir, &candidate)).await {
            return candidate;
        }
    }
    unreachable!()
}

async fn copy_file(
    job: &Job,
    run: &mut Run,
    src_path: &str,
    dst_dir: &str,
    name: &str,
    size: u64,
    mtime: Option<i64>,
) -> Result<(), Stop> {
    let mut dst_path = join(&job.dst, dst_dir, name);
    if job.dst.exists(&dst_path).await {
        let choice = match run.apply_all {
            Some(c) => c,
            None => {
                let existing = job.dst.stat(&dst_path).await.ok();
                let (tx, rx) = oneshot::channel();
                if let Ok(mut slot) = run.shared.resolver.lock() {
                    *slot = Some(tx);
                }
                (run.emit)(TransferEvent::Conflict {
                    item: run.item,
                    name: name.to_string(),
                    src_size: size as f64,
                    src_mtime: mtime.map(|m| m as f64),
                    dest_size: existing.as_ref().map_or(0.0, |s| s.size as f64),
                    dest_mtime: existing.and_then(|s| s.mtime).map(|m| m as f64),
                });
                let mut cancel = run.cancel.clone();
                let answer = tokio::select! {
                    a = rx => a.ok(),
                    _ = cancel.changed() => None,
                };
                let Some((choice, all)) = answer else {
                    return Err(Stop::Cancelled);
                };
                if all {
                    run.apply_all = Some(choice);
                }
                choice
            }
        };
        match choice {
            Choice::Skip => {
                run.done += size; // counted as handled, so the bar still reaches the end
                run.progress(false);
                return Ok(());
            }
            Choice::KeepBoth => {
                dst_path = join(&job.dst, dst_dir, &free_name(&job.dst, dst_dir, name).await)
            }
            Choice::Replace => {}
        }
    }

    let mut reader = job.src.open_read(src_path).await?;
    let mut writer = job.dst.open_write(&dst_path).await?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if run.cancelled() {
            drop(writer);
            let _ = job.dst.remove(&dst_path).await; // don't leave half a file behind
            return Err(Stop::Cancelled);
        }
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
        run.done += n as u64;
        run.progress(false);
    }
    writer.shutdown().await?;
    Ok(())
}
