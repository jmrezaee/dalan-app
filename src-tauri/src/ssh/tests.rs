//! End-to-end session tests against an in-process SSH server (no Docker needed):
//! first connect, known key, changed key (blocked, then explicitly replaced), wrong password.

use std::sync::Arc;
use std::time::Duration;

use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use super::{
    client, Cmd, PromptAnswer, PromptKind, Prompts, SessionEvent, SessionIo, SessionState,
};
use crate::db::open_in_memory;
use crate::error::ErrorCode;
use crate::hosts::{self, HostInput};
use crate::known_hosts;
use crate::state::AppState;
use crate::vault::{self, kdf::KdfParams};

#[derive(Clone)]
struct TestServer;

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "test" && password == "right-pw" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _c: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _s: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        c: ChannelId,
        _t: &str,
        _w: u32,
        _h: u32,
        _pw: u32,
        _ph: u32,
        _m: &[(russh::Pty, u32)],
        s: &mut Session,
    ) -> Result<(), Self::Error> {
        s.channel_success(c)?;
        Ok(())
    }
    async fn shell_request(&mut self, c: ChannelId, s: &mut Session) -> Result<(), Self::Error> {
        s.channel_success(c)?;
        s.data(c, b"welcome\r\n".to_vec())?;
        Ok(())
    }
    async fn data(
        &mut self,
        c: ChannelId,
        data: &[u8],
        s: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut out = b"echo: ".to_vec();
        out.extend_from_slice(data);
        s.data(c, out)?;
        Ok(())
    }
}

async fn start_server() -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![russh::keys::PrivateKey::random(
            &mut rand::rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap()],
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    });
    let socket = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = socket.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut s = TestServer;
        let _ = s.run_on_socket(config, &socket).await;
    });
    port
}

struct Harness {
    state: AppState,
    host_id: String,
    port: u16,
}

/// A fresh unlocked vault with one host pointing at a new test server.
async fn harness(saved_password: Option<&str>) -> Harness {
    let conn = open_in_memory().unwrap();
    let session = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
    let state = AppState::new(conn);
    state.set_session(Some(session));
    let port = start_server().await;
    let pw = saved_password.map(String::from);
    let host = state
        .store()
        .with_db_session(move |c, s| {
            hosts::save(
                c,
                s,
                &HostInput {
                    id: None,
                    label: "test-box".into(),
                    address: "127.0.0.1".into(),
                    port: Some(port),
                    username: Some("test".into()),
                    password: pw,
                    clear_password: false,
                    group_id: None,
                    tags: vec![],
                },
            )
        })
        .await
        .unwrap();
    Harness {
        state,
        host_id: host.id,
        port,
    }
}

struct Running {
    events: mpsc::UnboundedReceiver<SessionEvent>,
    data: mpsc::UnboundedReceiver<Vec<u8>>,
    cmds: mpsc::UnboundedSender<Cmd>,
    prompts: Arc<Prompts>,
    task: tokio::task::JoinHandle<()>,
}

fn start(h: &Harness) -> Running {
    let (etx, events) = mpsc::unbounded_channel();
    let (dtx, data) = mpsc::unbounded_channel();
    let (cmds, crx) = mpsc::unbounded_channel();
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(move |b| {
            let _ = dtx.send(b);
        }),
    });
    let prompts = Arc::new(Prompts::default());
    let params = client::Params {
        host_id: h.host_id.clone(),
        cols: 80,
        rows: 24,
    };
    let task = tokio::spawn(client::run(
        h.state.store(),
        io,
        Arc::clone(&prompts),
        params,
        crx,
    ));
    Running {
        events,
        data,
        cmds,
        prompts,
        task,
    }
}

impl Running {
    async fn next(&mut self) -> SessionEvent {
        tokio::time::timeout(Duration::from_secs(10), self.events.recv())
            .await
            .expect("timed out waiting for an event")
            .expect("session ended")
    }
    /// Skips events until `pred` matches.
    async fn until(&mut self, pred: impl Fn(&SessionEvent) -> bool) -> SessionEvent {
        loop {
            let e = self.next().await;
            if pred(&e) {
                return e;
            }
        }
    }
    fn answer(&self, prompt_id: &str, accept: bool, text: Option<&str>, remember: bool) {
        self.prompts
            .answer(
                prompt_id,
                PromptAnswer {
                    accept,
                    text: text.map(String::from),
                    remember,
                },
            )
            .unwrap();
    }
    async fn expect_prompt(&mut self) -> (String, PromptKind) {
        match self
            .until(|e| matches!(e, SessionEvent::Prompt { .. }))
            .await
        {
            SessionEvent::Prompt { prompt_id, kind } => (prompt_id, kind),
            _ => unreachable!(),
        }
    }
    async fn read_until(&mut self, needle: &str) -> String {
        let mut all = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(chunk) = self.data.recv().await {
                all.extend(chunk);
                if String::from_utf8_lossy(&all).contains(needle) {
                    break;
                }
            }
        })
        .await
        .expect("timed out waiting for terminal output");
        String::from_utf8_lossy(&all).into_owned()
    }
}

fn is_state(s: SessionState) -> impl Fn(&SessionEvent) -> bool {
    move |e| matches!(e, SessionEvent::State { state } if std::mem::discriminant(state) == std::mem::discriminant(&s))
}

async fn saved_keys(h: &Harness) -> Vec<known_hosts::KnownHost> {
    h.state
        .store()
        .with_db_session(|c, s| known_hosts::list(c, &s.vault_id))
        .await
        .unwrap()
}

#[tokio::test]
async fn first_connect_trusts_the_key_then_runs_a_shell_and_remembers_it() {
    let h = harness(Some("right-pw")).await;

    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    let PromptKind::HostKeyNew { fingerprint, .. } = kind else {
        panic!("expected a first-connect prompt")
    };
    assert!(fingerprint.starts_with("SHA256:"));
    assert!(
        saved_keys(&h).await.is_empty(),
        "nothing is saved before the user says yes"
    );
    s.answer(&id, true, None, false);
    s.until(is_state(SessionState::Ready)).await;
    assert_eq!(saved_keys(&h).await.len(), 1);

    assert!(s.read_until("welcome").await.contains("welcome"));
    s.cmds.send(Cmd::Write(b"hi\n".to_vec())).unwrap();
    assert!(s.read_until("echo: hi").await.contains("echo: hi"));
    s.cmds.send(Cmd::Close).unwrap();
    s.until(is_state(SessionState::Closed)).await;
    s.task.await.unwrap();

    // Second connect: the key is known, so there is no prompt at all.
    let mut s = start(&h);
    let first = s
        .until(|e| {
            matches!(
                e,
                SessionEvent::Prompt { .. }
                    | SessionEvent::State {
                        state: SessionState::Ready
                    }
            )
        })
        .await;
    assert!(
        matches!(first, SessionEvent::State { .. }),
        "known key must not prompt"
    );
    s.cmds.send(Cmd::Close).unwrap();
    s.task.await.unwrap();
}

#[tokio::test]
async fn cancelling_the_first_connect_prompt_saves_nothing_and_closes() {
    let h = harness(Some("right-pw")).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, false, None, false);
    s.until(is_state(SessionState::Closed)).await;
    assert!(saved_keys(&h).await.is_empty());
}

#[tokio::test]
async fn changed_key_blocks_and_only_an_explicit_replace_continues() {
    let h = harness(Some("right-pw")).await;
    let pattern = known_hosts::pattern("127.0.0.1", h.port);
    h.state
        .store()
        .with_db_session(move |c, s| {
            known_hosts::trust(
                c,
                &s.vault_id,
                &pattern,
                "ssh-ed25519",
                "AAAAC3NzaC1lZDI1NTE5AAAAIOLDKEYOLDKEYOLDKEYOLDKEYOLDKEYOLDKEYOLDKEY",
                "SHA256:old",
            )
        })
        .await
        .unwrap();

    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    let PromptKind::HostKeyChanged {
        old_fingerprint,
        new_fingerprint,
        ..
    } = kind
    else {
        panic!("expected the identity-changed block")
    };
    assert_eq!(old_fingerprint, "SHA256:old");
    assert_ne!(new_fingerprint, old_fingerprint);
    s.answer(&id, false, None, false); // Close
    s.until(is_state(SessionState::Closed)).await;
    assert_eq!(
        saved_keys(&h).await[0].fingerprint,
        "SHA256:old",
        "cancel must leave the saved key alone"
    );

    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false); // Replace saved key
    s.until(is_state(SessionState::Ready)).await;
    assert_eq!(saved_keys(&h).await[0].fingerprint, new_fingerprint);
}

#[tokio::test]
async fn wrong_saved_password_reports_auth_failed() {
    let h = harness(Some("wrong-pw")).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false);
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, message } => {
            assert_eq!(code, ErrorCode::AuthFailed);
            assert!(message.contains("test-box"));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn no_saved_password_prompts_and_remember_saves_it() {
    let h = harness(None).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::Password { .. }));
    s.answer(&id, true, Some("right-pw"), true);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    s.task.await.unwrap();
    let hid = h.host_id.clone();
    let saved = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::get(c, &s.vault_id, &hid))
        .await
        .unwrap();
    assert!(saved.has_password, "Remember stores the password");
}

#[tokio::test]
async fn unreachable_host_reports_host_unreachable() {
    let h = harness(Some("right-pw")).await;
    // Port 1 on localhost: nothing listens there.
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| {
            hosts::save(
                c,
                s,
                &HostInput {
                    id: Some(hid),
                    label: "test-box".into(),
                    address: "127.0.0.1".into(),
                    port: Some(1),
                    username: Some("test".into()),
                    password: None,
                    clear_password: false,
                    group_id: None,
                    tags: vec![],
                },
            )
        })
        .await
        .unwrap();
    let mut s = start(&h);
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, .. } => assert_eq!(code, ErrorCode::HostUnreachable),
        _ => unreachable!(),
    }
}

/// Against the Docker server from CLAUDE.md ("Testing against real servers"). Run with:
/// `cargo test docker -- --ignored` after starting the container on port 2222.
#[tokio::test]
#[ignore = "needs the Docker OpenSSH server on 127.0.0.1:2222"]
async fn docker_openssh_server_runs_a_real_shell() {
    let h = harness(None).await;
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| {
            hosts::save(
                c,
                s,
                &HostInput {
                    id: Some(hid),
                    label: "docker".into(),
                    address: "127.0.0.1".into(),
                    port: Some(2222),
                    username: Some("test".into()),
                    password: Some("test".into()),
                    clear_password: false,
                    group_id: None,
                    tags: vec![],
                },
            )
        })
        .await
        .unwrap();
    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { .. }));
    s.answer(&id, true, None, false);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds
        .send(Cmd::Write(b"echo dalan-$((40+2))\n".to_vec()))
        .unwrap();
    assert!(s.read_until("dalan-42").await.contains("dalan-42"));
    // Backpressure path: a burst of output must arrive intact.
    s.cmds
        .send(Cmd::Write(b"seq 1 20000; echo END-MARK\n".to_vec()))
        .unwrap();
    let out = s.read_until("END-MARK").await;
    assert!(out.contains("20000"));
    s.cmds.send(Cmd::Close).unwrap();
    s.until(is_state(SessionState::Closed)).await;
}
