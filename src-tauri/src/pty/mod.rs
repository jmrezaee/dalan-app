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

/// The shell for "This computer": the one chosen in Settings if it is a real file, else the user's own.
pub fn local_shell(conn: &rusqlite::Connection) -> String {
    crate::settings::hosts_field(conn, "local_shell")
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty() && std::path::Path::new(s).is_file())
        .unwrap_or_else(default_shell)
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

/// A program to run in the terminal.
pub struct Program {
    pub path: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// True for a shell: when it exits the tab closes, like any terminal window. False for something
    /// like `mosh-client`, where the exit is the end of a connection: the tab says so and can reconnect.
    pub close_on_exit: bool,
}

/// Runs until the shell exits or the tab closes. When the shell exits the tab closes too
/// (`closed`), like any terminal window.
pub async fn run(
    io: Arc<SessionIo>,
    shell: String,
    cols: u32,
    rows: u32,
    cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let args = if cfg!(unix) {
        vec!["-l".to_string()]
    } else {
        vec![]
    }; // a login shell, so profile files run
    let program = Program {
        path: shell,
        args,
        env: vec![],
        close_on_exit: true,
    };
    run_program(io, program, cols, rows, cmds).await;
}

pub async fn run_program(
    io: Arc<SessionIo>,
    program: Program,
    cols: u32,
    rows: u32,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let shell = program.path.clone();
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
    let mut cmd = CommandBuilder::new(&program.path);
    for a in &program.args {
        cmd.arg(a);
    }
    for (k, v) in &program.env {
        cmd.env(k, v);
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

    let mut closed_by_user = false;
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Close) => { closed_by_user = true; break; }
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
    if program.close_on_exit || closed_by_user {
        state(&io, SessionState::Closed);
    } else {
        (io.event)(SessionEvent::Exit { status: None });
        state(&io, SessionState::Disconnected);
    }
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

    /// A shell that prints without end must pause once the UI is MAX_UNACKED bytes behind, and go on after acks.
    #[tokio::test]
    async fn output_pauses_when_the_ui_is_behind_and_resumes_after_acks() {
        let total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let t = Arc::clone(&total);
        let io = Arc::new(SessionIo {
            event: Box::new(|_| {}),
            data: Box::new(move |b| {
                t.fetch_add(b.len(), Ordering::Relaxed);
            }),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(io, "/bin/sh".into(), 80, 24, rx));
        tx.send(Cmd::Write(b"yes dalan\n".to_vec())).unwrap();
        // Let it run, with no acks: the count must stall near the limit (the last chunk can overshoot a little).
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let first = total.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(700)).await;
        let second = total.load(Ordering::Relaxed);
        assert!(
            first >= MAX_UNACKED,
            "it should fill up to the limit first, got {first}"
        );
        assert!(
            second - first < 256 * 1024,
            "output must stall while nothing is acknowledged ({first} -> {second})"
        );
        assert!(
            second < MAX_UNACKED + 1024 * 1024,
            "far past the limit: {second}"
        );
        // The UI catches up: more output flows.
        tx.send(Cmd::Ack(second)).unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(
            total.load(Ordering::Relaxed) > second + 64 * 1024,
            "output must resume after an ack"
        );
        // Stop the flood and end the shell, so the reader thread finishes and the test runtime can shut down.
        tx.send(Cmd::Write(b"\x03".to_vec())).unwrap();
        tx.send(Cmd::Ack(usize::MAX / 2)).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.send(Cmd::Write(b"exit\n".to_vec())).unwrap();
        tx.send(Cmd::Ack(usize::MAX / 2)).unwrap();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the shell did not stop")
            .unwrap();
    }
}
