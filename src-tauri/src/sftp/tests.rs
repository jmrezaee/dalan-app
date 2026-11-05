//! File panes and transfers. Local-to-local covers the shared logic (folders, conflicts, cancel);
//! one ignored test runs the same against real OpenSSH over SFTP.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("dalan-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }
    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.0.join(rel)).unwrap()
    }
    fn exists(&self, rel: &str) -> bool {
        Path::new(&self.path(rel)).exists()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    mgr: Arc<TransferManager>,
    id: String,
    events: mpsc::UnboundedReceiver<TransferEvent>,
}

fn copy(items: Vec<(String, String)>) -> Run {
    let mgr = Arc::new(TransferManager::default());
    let (tx, events) = mpsc::unbounded_channel();
    let jobs = items
        .into_iter()
        .map(|(src, dst)| transfer_job(Fs::Local, src, Fs::Local, dst))
        .collect();
    let id = mgr
        .start(
            jobs,
            Arc::new(move |e| {
                let _ = tx.send(e);
            }),
        )
        .unwrap();
    Run { mgr, id, events }
}

impl Run {
    async fn next(&mut self) -> TransferEvent {
        tokio::time::timeout(Duration::from_secs(10), self.events.recv())
            .await
            .expect("timed out")
            .expect("closed")
    }
    /// Collects events until `Done`.
    async fn finish(&mut self) -> Vec<TransferEvent> {
        let mut all = Vec::new();
        loop {
            let e = self.next().await;
            let done = matches!(e, TransferEvent::Done { .. });
            all.push(e);
            if done {
                return all;
            }
        }
    }
}

#[tokio::test]
async fn copies_a_folder_tree_and_reports_progress_to_the_total() {
    let (src, dst) = (TempDir::new(), TempDir::new());
    src.write("site/index.html", b"<h1>hi</h1>");
    src.write("site/css/app.css", b"body{}");
    src.write("site/img/deep/logo.bin", &[7u8; 300_000]);
    std::fs::create_dir_all(src.path("site/empty")).unwrap();
    let mut run = copy(vec![(src.path("site"), dst.path(""))]);
    let events = run.finish().await;
    assert_eq!(dst.read("site/index.html"), b"<h1>hi</h1>");
    assert_eq!(dst.read("site/img/deep/logo.bin").len(), 300_000);
    assert!(dst.exists("site/empty"), "empty folders are copied too");
    let total = events
        .iter()
        .find_map(|e| {
            if let TransferEvent::Started { total, .. } = e {
                Some(*total)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(total, (11 + 6 + 300_000) as f64);
    let last_progress = events
        .iter()
        .rev()
        .find_map(|e| {
            if let TransferEvent::Progress { bytes, .. } = e {
                Some(*bytes)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(last_progress, total, "the bar ends at 100%");
    assert!(events
        .iter()
        .any(|e| matches!(e, TransferEvent::ItemDone { .. })));
    assert!(matches!(
        events.last(),
        Some(TransferEvent::Done { cancelled: false })
    ));
}

#[tokio::test]
async fn conflicts_wait_for_the_user_and_each_choice_does_what_it_says() {
    for (choice, expect_original, expect_copy) in [
        (Choice::Replace, "new", None),
        (Choice::Skip, "old", None),
        (Choice::KeepBoth, "old", Some("new")),
    ] {
        let (src, dst) = (TempDir::new(), TempDir::new());
        src.write("a.txt", b"new");
        dst.write("a.txt", b"old");
        let mut run = copy(vec![(src.path("a.txt"), dst.path(""))]);
        loop {
            match run.next().await {
                TransferEvent::Conflict {
                    name,
                    src_size,
                    dest_size,
                    ..
                } => {
                    assert_eq!((name.as_str(), src_size, dest_size), ("a.txt", 3.0, 3.0));
                    run.mgr.resolve(&run.id, choice, false).unwrap();
                }
                TransferEvent::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(dst.read("a.txt"), expect_original.as_bytes(), "{choice:?}");
        assert_eq!(
            dst.exists("a (copy).txt"),
            expect_copy.is_some(),
            "{choice:?}"
        );
        if let Some(c) = expect_copy {
            assert_eq!(dst.read("a (copy).txt"), c.as_bytes());
        }
    }
}

#[tokio::test]
async fn apply_to_all_answers_later_conflicts_without_asking_again() {
    let (src, dst) = (TempDir::new(), TempDir::new());
    for n in ["one.txt", "two.txt", "three.txt"] {
        src.write(&format!("pack/{n}"), b"new");
        dst.write(&format!("pack/{n}"), b"old");
    }
    let mut run = copy(vec![(src.path("pack"), dst.path(""))]);
    let mut asked = 0;
    loop {
        match run.next().await {
            TransferEvent::Conflict { .. } => {
                asked += 1;
                run.mgr.resolve(&run.id, Choice::Skip, true).unwrap();
            }
            TransferEvent::Done { .. } => break,
            _ => {}
        }
    }
    assert_eq!(asked, 1, "one answer covers the rest");
    assert_eq!(dst.read("pack/two.txt"), b"old");
}

#[tokio::test]
async fn cancelling_stops_the_copy_and_removes_the_half_written_file() {
    let (src, dst) = (TempDir::new(), TempDir::new());
    src.write("big.bin", &vec![1u8; 40 * 1024 * 1024]);
    let mut run = copy(vec![(src.path("big.bin"), dst.path(""))]);
    loop {
        match run.next().await {
            TransferEvent::Progress { .. } => {
                run.mgr.cancel(&run.id).unwrap();
            }
            TransferEvent::Done { cancelled } => {
                assert!(cancelled);
                break;
            }
            _ => {}
        }
    }
    assert!(!dst.exists("big.bin"), "no partial file is left");
}

#[tokio::test]
async fn a_missing_source_fails_that_item_but_the_queue_continues() {
    let (src, dst) = (TempDir::new(), TempDir::new());
    src.write("ok.txt", b"fine");
    let mut run = copy(vec![
        (src.path("gone.txt"), dst.path("")),
        (src.path("ok.txt"), dst.path("")),
    ]);
    let events = run.finish().await;
    assert!(events
        .iter()
        .any(|e| matches!(e, TransferEvent::ItemFailed { item: 0, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, TransferEvent::ItemDone { item: 1 })));
    assert_eq!(dst.read("ok.txt"), b"fine");
}

#[tokio::test]
async fn file_operations_list_hide_dotfiles_rename_chmod_and_delete_recursively() {
    let t = TempDir::new();
    t.write("visible.txt", b"x");
    t.write(".hidden", b"x");
    t.write("dir/inner/file.txt", b"x");
    let fs = Fs::Local;
    let shown = fs.list(&t.path(""), false).await.unwrap();
    assert_eq!(
        shown
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>()
            .len(),
        2
    );
    assert!(!shown.iter().any(|e| e.name == ".hidden"));
    assert!(fs
        .list(&t.path(""), true)
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == ".hidden"));
    let dir = shown.iter().find(|e| e.name == "dir").unwrap();
    assert_eq!(dir.kind, EntryKind::Dir);

    fs.mkdir(&t.path("made")).await.unwrap();
    assert!(
        fs.mkdir(&t.path("made")).await.is_err(),
        "can't make it twice"
    );
    fs.rename(&t.path("visible.txt"), &t.path("renamed.txt"))
        .await
        .unwrap();
    assert!(
        fs.rename(&t.path("renamed.txt"), &t.path("made"))
            .await
            .is_err(),
        "won't overwrite by renaming"
    );
    #[cfg(unix)]
    {
        fs.chmod(&t.path("renamed.txt"), 0o600).await.unwrap();
        let e = fs
            .list(&t.path(""), false)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.name == "renamed.txt")
            .unwrap();
        assert_eq!(e.mode, Some(0o600));
    }
    fs.remove(&t.path("dir")).await.unwrap();
    assert!(!t.exists("dir"));
    assert!(fs.remove(&t.path("dir")).await.is_err());
}

fn copy_fs(jobs: Vec<(Fs, String, Fs, String)>) -> Run {
    let mgr = Arc::new(TransferManager::default());
    let (tx, events) = mpsc::unbounded_channel();
    let jobs = jobs
        .into_iter()
        .map(|(s, sp, d, dd)| transfer_job(s, sp, d, dd))
        .collect();
    let id = mgr
        .start(
            jobs,
            Arc::new(move |e| {
                let _ = tx.send(e);
            }),
        )
        .unwrap();
    Run { mgr, id, events }
}

/// Real OpenSSH over SFTP: list, upload, download (byte-for-byte), conflict, chmod, rename, delete.
#[tokio::test]
#[ignore = "needs the Docker OpenSSH server on 127.0.0.1:2222"]
async fn docker_sftp_lists_uploads_downloads_and_deletes() {
    use crate::ssh::tests::{harness, host_input};
    use crate::ssh::{PromptAnswer, Prompts, SessionEvent, SessionIo};

    let h = harness(None).await;
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| {
            let mut i = host_input("docker", 2222, None);
            i.id = Some(hid);
            i.password = Some("test".into());
            crate::hosts::save(c, s, &i)
        })
        .await
        .unwrap();
    // Answer the host-key question as the UI would.
    let prompts = Arc::new(Prompts::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            let _ = tx.send(e);
        }),
        data: Box::new(|_| {}),
    });
    let answerer = Arc::clone(&prompts);
    tokio::spawn(async move {
        while let Some(e) = rx.recv().await {
            if let SessionEvent::Prompt { prompt_id, .. } = e {
                let _ = answerer.answer(
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: false,
                    },
                );
            }
        }
    });
    let remote = Arc::new(
        open_remote(&h.state.store(), &io, &prompts, &h.host_id)
            .await
            .unwrap(),
    );
    let fs = Fs::Remote(Arc::clone(&remote));
    let home = fs.home().await.unwrap();
    let dir = format!("{home}/dalan-sftp-{}", uuid::Uuid::now_v7());
    fs.mkdir(&dir).await.unwrap();

    // Upload a 3 MB file and a folder, then read the listing back.
    let local = TempDir::new();
    let payload: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    local.write("payload.bin", &payload);
    local.write("folder/note.txt", b"hello from dalan");
    let mut up = copy_fs(vec![
        (
            Fs::Local,
            local.path("payload.bin"),
            fs.clone(),
            dir.clone(),
        ),
        (Fs::Local, local.path("folder"), fs.clone(), dir.clone()),
    ]);
    let events = up.finish().await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, TransferEvent::ItemFailed { .. })),
        "{events:?}"
    );
    let listing = fs.list(&dir, false).await.unwrap();
    let file = listing.iter().find(|e| e.name == "payload.bin").unwrap();
    assert_eq!((file.kind, file.size), (EntryKind::File, 3_000_000));
    assert_eq!(
        listing.iter().find(|e| e.name == "folder").unwrap().kind,
        EntryKind::Dir
    );

    // Download it again and compare every byte.
    let back = TempDir::new();
    let mut down = copy_fs(vec![(
        fs.clone(),
        format!("{dir}/payload.bin"),
        Fs::Local,
        back.path(""),
    )]);
    down.finish().await;
    assert_eq!(back.read("payload.bin"), payload);

    // Uploading the same file again is a conflict the user decides.
    let mut again = copy_fs(vec![(
        Fs::Local,
        local.path("payload.bin"),
        fs.clone(),
        dir.clone(),
    )]);
    loop {
        match again.next().await {
            TransferEvent::Conflict {
                name, dest_size, ..
            } => {
                assert_eq!((name.as_str(), dest_size), ("payload.bin", 3_000_000.0));
                again
                    .mgr
                    .resolve(&again.id, Choice::KeepBoth, false)
                    .unwrap();
            }
            TransferEvent::Done { .. } => break,
            _ => {}
        }
    }
    assert!(fs.exists(&format!("{dir}/payload (copy).bin")).await);

    // Permissions, rename, delete.
    fs.chmod(&format!("{dir}/payload.bin"), 0o600)
        .await
        .unwrap();
    let mode = fs
        .list(&dir, false)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.name == "payload.bin")
        .unwrap()
        .mode;
    assert_eq!(mode, Some(0o600));
    fs.rename(&format!("{dir}/payload.bin"), &format!("{dir}/renamed.bin"))
        .await
        .unwrap();
    assert!(fs.exists(&format!("{dir}/renamed.bin")).await);
    fs.remove(&dir).await.unwrap();
    assert!(
        !fs.exists(&dir).await,
        "the folder and everything in it is gone"
    );
}
