//! Local terminal (S23 "Local terminal"): a shell on this computer in a PTY, shown in a normal
//! terminal tab. It speaks the same `Cmd` / `SessionIo` language as an SSH session, so the UI
//! can't tell the difference. Output is raw bytes, never a `String` (hard rule 3).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use crate::error::ErrorCode;
use crate::ssh::{Cmd, SessionEvent, SessionIo, SessionState};

const FLUSH_EVERY: Duration = Duration::from_millis(8);
const FLUSH_AT: usize = 64 * 1024;
/// Stop reading from the shell while the UI is this far behind.
const MAX_UNACKED: usize = 1024 * 1024;

/// The user's shell: `$SHELL`, else `/bin/sh` (or `cmd.exe` on Windows).
pub fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into())
    }
}

fn state(io: &SessionIo, s: SessionState) {
    (io.event)(SessionEvent::State { state: s });
}

fn fail(io: &SessionIo, message: String) {
    (io.event)(SessionEvent::Error {
        code: ErrorCode::Internal,
        message,
    });
}

/// Runs until the shell exits or the tab closes. When the shell exits the tab closes too
/// (`closed`), like any terminal window.
pub async fn run(
    io: Arc<SessionIo>,
    shell: String,
    cols: u32,
    rows: u32,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let size = |c: u32, r: u32| PtySize {
        rows: r.clamp(1, 500) as u16,
        cols: c.clamp(1, 1000) as u16,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = match native_pty_system().openpty(size(cols, rows)) {
        Ok(p) => p,
        Err(e) => return fail(&io, format!("Couldn't open a terminal: {e}")),
    };
    let mut cmd = CommandBuilder::new(&shell);
    if cfg!(unix) {
        cmd.arg("-l"); // a login shell, like a new terminal window, so profile files run
    }
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        cmd.cwd(home);
    }
    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => return fail(&io, format!("Couldn't start {shell}: {e}")),
    };
    drop(pair.slave); // only the child should hold the slave end, or EOF never arrives
    let master = pair.master;
    let (mut reader, mut writer) = match (master.try_clone_reader(), master.take_writer()) {
        (Ok(r), Ok(w)) => (r, w),
        _ => {
            let _ = child.kill();
            return fail(&io, "Couldn't connect to the terminal.".into());
        }
    };

    // PTY reads block, so a thread does them and hands chunks over. The channel is small: when the
    // UI is behind and we stop receiving, the thread blocks and the shell slows down (backpressure).
    let (tx, mut out) = mpsc::channel::<Vec<u8>>(16);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 || tx.blocking_send(chunk[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    state(&io, SessionState::Ready);
    let unacked = AtomicUsize::new(0);
    let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_AT);
    let mut tick = interval(FLUSH_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let flush = |buf: &mut Vec<u8>| {
        if !buf.is_empty() {
            unacked.fetch_add(buf.len(), Ordering::Relaxed);
            (io.data)(std::mem::take(buf));
        }
    };

    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Close) => break,
                // Keystrokes are tiny; a blocking write here is fine.
                Some(Cmd::Write(b)) => { if writer.write_all(&b).and_then(|()| writer.flush()).is_err() { break; } }
                Some(Cmd::Resize(c, r)) => { let _ = master.resize(size(c, r)); }
                Some(Cmd::Ack(n)) => {
                    let _ = unacked.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |u| Some(u.saturating_sub(n)));
                }
            },
            chunk = out.recv(), if unacked.load(Ordering::Relaxed) < MAX_UNACKED => match chunk {
                None => break, // the shell closed its end
                Some(c) => {
                    buf.extend_from_slice(&c);
                    if buf.len() >= FLUSH_AT { flush(&mut buf); }
                }
            },
            _ = tick.tick() => flush(&mut buf),
        }
    }
    flush(&mut buf);
    let _ = child.kill(); // no-op when it already exited
    let _ = child.wait();
    state(&io, SessionState::Closed);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn shell_runs_commands_and_closes() {
        let got: Arc<Mutex<Vec<u8>>> = Arc::default();
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let (g, ev) = (Arc::clone(&got), Arc::clone(&events));
        let io = Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::State { state } = e {
                    ev.lock().unwrap().push(format!("{state:?}"));
                }
            }),
            data: Box::new(move |b| g.lock().unwrap().extend(b)),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(io, "/bin/sh".into(), 80, 24, rx));
        tx.send(Cmd::Write(b"echo dalan-$((40+2))\n".to_vec()))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !String::from_utf8_lossy(&got.lock().unwrap()).contains("dalan-42") {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the shell never printed the answer");
        // `exit` ends the shell, which closes the tab by itself.
        tx.send(Cmd::Write(b"exit\n".to_vec())).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("shell did not stop")
            .unwrap();
        assert_eq!(
            events.lock().unwrap().last().map(String::as_str),
            Some("Closed")
        );
    }
}
