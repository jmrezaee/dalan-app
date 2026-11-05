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

/// Shared knobs for the test server: which public key it accepts, and the commands it was asked to run.
#[derive(Clone, Default)]
struct TestServer {
    accepted_key: Arc<std::sync::Mutex<Option<String>>>,
    execs: Arc<std::sync::Mutex<Vec<String>>>,
}

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
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
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: russh::server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let fp = key
            .public_key
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        let ok =
            user == "test" && self.accepted_key.lock().unwrap().as_deref() == Some(fp.as_str());
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }
    async fn exec_request(
        &mut self,
        c: ChannelId,
        data: &[u8],
        s: &mut Session,
    ) -> Result<(), Self::Error> {
        self.execs
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(data).into_owned());
        s.channel_success(c)?;
        s.exit_status_request(c, 0)?;
        s.close(c)?;
        Ok(())
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

async fn start_server(srv: TestServer) -> u16 {
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
        let mut s = srv;
        let _ = s.run_on_socket(config, &socket).await;
    });
    port
}

struct Harness {
    state: AppState,
    host_id: String,
    port: u16,
    server: TestServer,
}

/// A fresh unlocked vault with one host pointing at a new test server.
async fn harness(saved_password: Option<&str>) -> Harness {
    let conn = open_in_memory().unwrap();
    let session = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
    let state = AppState::new(conn);
    state.set_session(Some(session));
    let server = TestServer::default();
    let port = start_server(server.clone()).await;
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
                    key_id: None,
                    identity_id: None,
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
        server,
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
                    key_id: None,
                    identity_id: None,
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
                    key_id: None,
                    identity_id: None,
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

/// Generates a key, stores it, makes the server accept it, and links it to the host.
async fn give_host_a_key(
    h: &Harness,
    passphrase: Option<&str>,
    save: bool,
) -> crate::keys::KeySummary {
    let (pass, hid) = (passphrase.map(String::from), h.host_id.clone());
    let key = h
        .state
        .store()
        .with_db_session(move |c, s| {
            let new = crate::keys::generate(&crate::keys::GenerateInput {
                label: "deploy-key".into(),
                kind: crate::keys::KeyKind::Ed25519,
                passphrase: pass,
                save_passphrase: save,
            })?;
            let k = crate::keys::store(c, s, &new)?;
            hosts::link_key(c, &s.vault_id, &hid, &k.id)?;
            Ok(k)
        })
        .await
        .unwrap();
    *h.server.accepted_key.lock().unwrap() = Some(key.fingerprint.clone());
    key
}

async fn trusted(h: &Harness) -> Running {
    let mut s = start(h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false);
    s
}

#[tokio::test]
async fn key_auth_signs_in_without_a_password() {
    let h = harness(None).await; // no password saved: only the key can work
    give_host_a_key(&h, None, false).await;
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await; // never saw a password prompt
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn encrypted_key_asks_for_its_passphrase_unless_saved() {
    let h = harness(None).await;
    give_host_a_key(&h, Some("key-pass-123"), false).await;
    let mut s = trusted(&h).await;
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::Passphrase { ref key_label } if key_label == "deploy-key"));
    s.answer(&id, true, Some("key-pass-123"), true); // remember it
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    s.task.await.unwrap();
    // Remembered: the next connect doesn't ask again.
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
        "a saved passphrase must not prompt"
    );
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn wrong_passphrase_is_reported_not_retried_blindly() {
    let h = harness(None).await;
    give_host_a_key(&h, Some("key-pass-123"), false).await;
    let mut s = trusted(&h).await;
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, Some("nope"), false);
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, message } => {
            assert_eq!(code, ErrorCode::AuthFailed);
            assert!(message.contains("deploy-key"));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn refused_key_without_a_password_says_the_key_was_rejected() {
    let h = harness(None).await;
    give_host_a_key(&h, None, false).await;
    *h.server.accepted_key.lock().unwrap() = Some("SHA256:someone-else".into());
    let mut s = trusted(&h).await;
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, message } => {
            assert_eq!(code, ErrorCode::AuthFailed);
            assert!(
                message.contains("rejected the key \"deploy-key\""),
                "{message}"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn refused_key_falls_back_to_the_saved_password() {
    let h = harness(Some("right-pw")).await;
    give_host_a_key(&h, None, false).await;
    *h.server.accepted_key.lock().unwrap() = Some("SHA256:someone-else".into());
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn export_installs_the_public_key_and_links_it_to_the_host() {
    let h = harness(Some("right-pw")).await;
    let hid = h.host_id.clone();
    let key = h
        .state
        .store()
        .with_db_session(move |c, s| {
            let new = crate::keys::generate(&crate::keys::GenerateInput {
                label: "it's-a-key".into(),
                kind: crate::keys::KeyKind::Ed25519,
                passphrase: None,
                save_passphrase: false,
            })?;
            crate::keys::store(c, s, &new)
        })
        .await
        .unwrap();
    let (etx, mut events) = mpsc::unbounded_channel();
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(|_| {}),
    });
    let prompts = Arc::new(Prompts::default());
    let task = tokio::spawn(client::run_export(
        h.state.store(),
        io,
        Arc::clone(&prompts),
        hid.clone(),
        key.id.clone(),
        "~/.ssh".into(),
        "authorized_keys".into(),
    ));
    // Trust the host key, then wait for the export to finish.
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            SessionEvent::Prompt { prompt_id, .. } => prompts
                .answer(
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: false,
                    },
                )
                .unwrap(),
            SessionEvent::Exit { status } => {
                assert_eq!(status, Some(0));
                break;
            }
            SessionEvent::Error { message, .. } => panic!("export failed: {message}"),
            _ => {}
        }
    }
    task.await.unwrap();
    let cmd = h
        .server
        .execs
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the server saw a command");
    assert!(
        cmd.contains("\"$HOME\"/.ssh") && cmd.contains("chmod 700") && cmd.contains("chmod 600"),
        "{cmd}"
    );
    assert!(
        cmd.contains("grep -qxF"),
        "must not add the key twice: {cmd}"
    );
    assert!(
        cmd.contains(&key.public_key.replace('\'', "'\\''")),
        "the quoted key is in the command: {cmd}"
    );
    let linked = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::get(c, &s.vault_id, &hid))
        .await
        .unwrap();
    assert_eq!(linked.key_id.as_deref(), Some(key.id.as_str()));
}

#[tokio::test]
async fn export_refuses_paths_that_could_escape_the_shell_command() {
    let h = harness(Some("right-pw")).await;
    let (etx, mut events) = mpsc::unbounded_channel();
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(|_| {}),
    });
    client::run_export(
        h.state.store(),
        io,
        Arc::new(Prompts::default()),
        h.host_id.clone(),
        "k".into(),
        "~/.ssh; rm -rf /".into(),
        "authorized_keys".into(),
    )
    .await;
    assert!(matches!(
        events.recv().await.unwrap(),
        SessionEvent::Error {
            code: ErrorCode::Validation,
            ..
        }
    ));
    assert!(h.server.execs.lock().unwrap().is_empty());
}

/// F02 against real OpenSSH: password sign-in, install a fresh key, then sign in with the key alone.
#[tokio::test]
#[ignore = "needs the Docker OpenSSH server on 127.0.0.1:2222"]
async fn docker_f02_install_key_then_connect_key_only() {
    let h = harness(None).await;
    let hid = h.host_id.clone();
    let key = h
        .state
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
                    key_id: None,
                    identity_id: None,
                    tags: vec![],
                },
            )?;
            let new = crate::keys::generate(&crate::keys::GenerateInput {
                label: "f02".into(),
                kind: crate::keys::KeyKind::Ed25519,
                passphrase: None,
                save_passphrase: false,
            })?;
            crate::keys::store(c, s, &new)
        })
        .await
        .unwrap();
    let (etx, mut events) = mpsc::unbounded_channel();
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(|_| {}),
    });
    let prompts = Arc::new(Prompts::default());
    let task = tokio::spawn(client::run_export(
        h.state.store(),
        io,
        Arc::clone(&prompts),
        h.host_id.clone(),
        key.id.clone(),
        "~/.ssh".into(),
        "authorized_keys".into(),
    ));
    loop {
        match tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            SessionEvent::Prompt { prompt_id, .. } => prompts
                .answer(
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: false,
                    },
                )
                .unwrap(),
            SessionEvent::Exit { .. } => break,
            SessionEvent::Error { message, .. } => panic!("export failed: {message}"),
            _ => {}
        }
    }
    task.await.unwrap();
    // Now drop the password: only the key (linked by the export) can get us in.
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
                    password: None,
                    clear_password: true,
                    group_id: None,
                    key_id: Some(key.id),
                    identity_id: None,
                    tags: vec![],
                },
            )
        })
        .await
        .unwrap();
    let mut s = start(&h);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds
        .send(Cmd::Write(b"echo key-only-$((6*7))\n".to_vec()))
        .unwrap();
    assert!(s.read_until("key-only-42").await.contains("key-only-42"));
    s.cmds.send(Cmd::Close).unwrap();
}
