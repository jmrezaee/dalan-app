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
pub(crate) struct TestServer {
    pub(crate) accepted_key: Arc<std::sync::Mutex<Option<String>>>,
    /// Sign-ins that came in as an OpenSSH certificate (for the accepted key).
    pub(crate) cert_logins: Arc<std::sync::Mutex<u32>>,
    pub(crate) execs: Arc<std::sync::Mutex<Vec<String>>>,
    /// What the server prints for a `uname` command (the OS-detection probe).
    pub(crate) uname_reply: Arc<std::sync::Mutex<Option<String>>>,
    /// What the server prints for `mosh-server new ...`.
    pub(crate) mosh_reply: Arc<std::sync::Mutex<Option<String>>>,
    /// Targets this server was asked to forward to (direct-tcpip), i.e. hops made through it.
    pub(crate) forwards: Arc<std::sync::Mutex<Vec<String>>>,
    /// Channels that carry forwarded traffic: their data belongs to the tunnel, not to our echo shell.
    pub(crate) tunnel_channels: Arc<std::sync::Mutex<Vec<ChannelId>>>,
    /// When set, the server only offers keyboard-interactive and wants this code (a 2FA-style server).
    pub(crate) kbd_code: Option<String>,
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
    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        certificate: &russh::keys::Certificate,
    ) -> Result<Auth, Self::Error> {
        // Stands in for "I trust the CA": accept a user certificate for the key we accept.
        let fp = russh::keys::PublicKey::from(certificate.public_key().clone())
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        let ok =
            user == "test" && self.accepted_key.lock().unwrap().as_deref() == Some(fp.as_str());
        if ok {
            *self.cert_logins.lock().unwrap() += 1;
        }
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(code) = self.kbd_code.clone() else {
            return Ok(Auth::reject());
        };
        Ok(match response {
            None => Auth::Partial {
                name: "Two-step sign-in".into(),
                instructions: "Open your authenticator app.".into(),
                prompts: vec![("Verification code: ".into(), true)].into(),
            },
            Some(mut r) => {
                let given = r.next().map(|b| String::from_utf8_lossy(&b).into_owned());
                if user == "test" && given.as_deref() == Some(code.as_str()) {
                    Auth::Accept
                } else {
                    Auth::reject()
                }
            }
        })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _oa: &str,
        _op: u32,
        reply: server::ChannelOpenHandle,
        _s: &mut Session,
    ) -> Result<(), Self::Error> {
        self.forwards.lock().unwrap().push(format!("{host}:{port}"));
        match tokio::net::TcpStream::connect((host, port as u16)).await {
            Ok(mut tcp) => {
                self.tunnel_channels.lock().unwrap().push(channel.id());
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                });
            }
            Err(_) => drop(reply), // dropping the handle rejects the channel
        }
        Ok(())
    }
    /// Remote forwards (`-R`): listen locally and hand each connection back to the client.
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Ok(listener) = TcpListener::bind((address, *port as u16)).await else {
            return Ok(false);
        };
        *port = u32::from(listener.local_addr().map(|a| a.port()).unwrap_or(0));
        let (handle, addr, p, tunnels) = (
            session.handle(),
            address.to_string(),
            *port,
            self.tunnel_channels.clone(),
        );
        tokio::spawn(async move {
            while let Ok((mut tcp, peer)) = listener.accept().await {
                let (handle, addr, tunnels) = (handle.clone(), addr.clone(), tunnels.clone());
                tokio::spawn(async move {
                    if let Ok(ch) = handle
                        .channel_open_forwarded_tcpip(
                            addr,
                            p,
                            peer.ip().to_string(),
                            u32::from(peer.port()),
                        )
                        .await
                    {
                        tunnels.lock().unwrap().push(ch.id());
                        let mut stream = ch.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                    }
                });
            }
        });
        Ok(true)
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
        if data.starts_with(b"mosh-server") {
            if let Some(reply) = self.mosh_reply.lock().unwrap().clone() {
                s.data(c, reply.into_bytes())?;
            }
        }
        if data.starts_with(b"uname") {
            if let Some(reply) = self.uname_reply.lock().unwrap().clone() {
                s.data(c, reply.into_bytes())?;
            }
        }
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
        if self.tunnel_channels.lock().unwrap().contains(&c) {
            return Ok(()); // the channel's own reader forwards it
        }
        let mut out = b"echo: ".to_vec();
        out.extend_from_slice(data);
        s.data(c, out)?;
        Ok(())
    }
}

pub(crate) async fn start_server(srv: TestServer) -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![russh::keys::PrivateKey::random(
            &mut rand::rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap()],
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        methods: if srv.kbd_code.is_some() {
            russh::MethodSet::from(&[russh::MethodKind::KeyboardInteractive][..])
        } else {
            server::Config::default().methods
        },
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

pub(crate) struct Harness {
    pub(crate) state: AppState,
    pub(crate) host_id: String,
    pub(crate) port: u16,
    pub(crate) server: TestServer,
}

/// A fresh unlocked vault with one host pointing at a new test server.
pub(crate) async fn harness(saved_password: Option<&str>) -> Harness {
    harness_with(saved_password, |_| {}).await
}

/// Same, with a chance to change the test server before it starts.
pub(crate) async fn harness_with(
    saved_password: Option<&str>,
    tweak: impl FnOnce(&mut TestServer),
) -> Harness {
    let conn = open_in_memory().unwrap();
    let session = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
    let state = AppState::new(conn);
    state.set_session(Some(session));
    let mut server = TestServer::default();
    tweak(&mut server);
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
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
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
    start_with(h, |store, io, prompts, params, crx| {
        tokio::spawn(client::run(store, io, prompts, params, crx))
    })
}

/// Like `start`, but for the Mosh job.
fn start_mosh(h: &Harness) -> Running {
    start_with(h, |store, io, prompts, params, crx| {
        tokio::spawn(crate::mosh::run(store, io, prompts, params, crx))
    })
}

fn start_with(
    h: &Harness,
    spawn: impl FnOnce(
        crate::state::Store,
        Arc<SessionIo>,
        Arc<Prompts>,
        client::Params,
        mpsc::UnboundedReceiver<Cmd>,
    ) -> tokio::task::JoinHandle<()>,
) -> Running {
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
    let task = spawn(h.state.store(), io, Arc::clone(&prompts), params, crx);
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
        let e = self.next_inner().await;
        if std::env::var("DALAN_TEST_EVENTS").is_ok() {
            eprintln!("event: {e:?}");
        }
        e
    }
    async fn next_inner(&mut self) -> SessionEvent {
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
    s.answer(&id, true, None, true);
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

    // A bare "yes" without the typed host name is not enough, even if the UI were bypassed.
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, true);
    s.until(is_state(SessionState::Closed)).await;
    assert_eq!(saved_keys(&h).await[0].fingerprint, "SHA256:old");

    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, Some("test-box"), true); // Replace saved key
    s.until(is_state(SessionState::Ready)).await;
    assert_eq!(saved_keys(&h).await[0].fingerprint, new_fingerprint);
}

#[tokio::test]
async fn wrong_saved_password_reports_auth_failed() {
    let h = harness(Some("wrong-pw")).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, true);
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
    s.answer(&id, true, None, true);
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
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
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
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
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
                    port: Some(crate::testservers::port("a")),
                    username: Some("test".into()),
                    password: Some("test".into()),
                    clear_password: false,
                    key_id: None,
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
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
    s.answer(&id, true, None, true);
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
    s.answer(&id, true, None, true);
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

/// A user certificate for `key`, signed by a throwaway CA.
pub(crate) fn sign_user_cert(
    key_public: &str,
    valid_secs: (u64, u64),
    cert_type: russh::keys::ssh_key::certificate::CertType,
) -> String {
    use russh::keys::ssh_key::{certificate::Builder, Algorithm, PrivateKey, PublicKey};
    let ca = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let subject = PublicKey::from_openssh(key_public).unwrap();
    let mut b =
        Builder::new_with_random_nonce(&mut rand::rng(), &subject, valid_secs.0, valid_secs.1)
            .unwrap();
    b.serial(1).unwrap();
    b.key_id("test-cert").unwrap();
    b.cert_type(cert_type).unwrap();
    b.valid_principal("test").unwrap();
    b.sign(&ca).unwrap().to_openssh().unwrap()
}

async fn os_hint(h: &Harness) -> Option<String> {
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| Ok(hosts::get(c, &s.vault_id, &hid)?.os_hint))
        .await
        .unwrap()
}

#[tokio::test]
async fn telnet_hosts_use_plain_tcp_and_refuse_ssh_only_features() {
    use crate::ssh::{Job, SessionManager};
    use tokio::io::AsyncWriteExt;
    // A "telnet server": greets with a banner and hangs up.
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut c, _) = l.accept().await.unwrap();
        c.write_all(b"hello from telnet\r\n").await.unwrap();
    });
    let h = harness(Some("right-pw")).await;
    let hid = h.host_id.clone();
    let saved = h
        .state
        .store()
        .with_db_session(move |c, s| {
            hosts::save(
                c,
                s,
                &HostInput {
                    id: Some(hid.clone()),
                    label: "t".into(),
                    address: "127.0.0.1".into(),
                    port: Some(port),
                    username: Some("test".into()),
                    password: Some("right-pw".into()),
                    clear_password: false,
                    key_id: None,
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
                    group_id: None,
                    tags: vec![],
                },
            )?;
            hosts::set_protocol(c, s, &hid, "telnet")?;
            let g = hosts::get(c, &s.vault_id, &hid)?;
            Ok((g.protocol, g.has_password))
        })
        .await
        .unwrap();
    assert_eq!(saved, ("telnet".to_string(), false)); // the saved password is dropped

    // A shell tab on a Telnet host runs the Telnet job and shows the banner.
    let shown: Arc<std::sync::Mutex<Vec<u8>>> = Arc::default();
    let d = Arc::clone(&shown);
    let (etx, mut events) = mpsc::unbounded_channel();
    let io = SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(move |b| d.lock().unwrap().extend(b)),
    };
    let mgr = SessionManager::default();
    mgr.open(h.state.store(), h.host_id.clone(), Job::Telnet, 80, 24, io)
        .unwrap();
    loop {
        if let Some(SessionEvent::State {
            state: SessionState::Disconnected,
        }) = events.recv().await
        {
            break;
        }
    }
    assert_eq!(shown.lock().unwrap().as_slice(), b"hello from telnet\r\n");

    // SSH-only entry points (SFTP, tunnels, key install) say so instead of speaking SSH to port 23.
    let mut s = start(&h);
    let e = s.until(|e| matches!(e, SessionEvent::Error { .. })).await;
    assert!(
        matches!(e, SessionEvent::Error { code: ErrorCode::Validation, ref message } if message.contains("Telnet"))
    );
    // The default port follows the protocol.
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| {
            hosts::save(
                c,
                s,
                &HostInput {
                    id: Some(hid.clone()),
                    label: "t".into(),
                    address: "127.0.0.1".into(),
                    port: None,
                    username: Some("test".into()),
                    password: None,
                    clear_password: false,
                    key_id: None,
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
                    group_id: None,
                    tags: vec![],
                },
            )?;
            assert_eq!(hosts::connect_info(c, s, &hid)?.port, 23);
            hosts::set_protocol(c, s, &hid, "ssh")?;
            assert_eq!(hosts::connect_info(c, s, &hid)?.port, 22);
            Ok(())
        })
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn mosh_starts_the_server_over_ssh_then_runs_the_local_client() {
    use std::os::unix::fs::PermissionsExt;
    // A stand-in for `mosh-client` that reports how it was started.
    let dir = std::env::temp_dir().join(format!("dalan-mosh-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("mosh-client");
    std::fs::write(&fake, "#!/bin/sh\necho \"ARGS:$1 $2 KEY:$MOSH_KEY\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("DALAN_MOSH_CLIENT", &fake); // only this test reads it

    let h = harness(Some("right-pw")).await;
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| hosts::set_protocol(c, s, &hid, "mosh"))
        .await
        .unwrap();
    *h.server.mosh_reply.lock().unwrap() =
        Some("\r\nMOSH CONNECT 60001 4NeCCgvZFe2RnPgrcU1PQw\r\n".into());

    let mut s = start_mosh(&h);
    let (id, _) = s.expect_prompt().await; // the SSH bootstrap asks about the host key as usual
    s.answer(&id, true, None, true);
    s.until(is_state(SessionState::Ready)).await;
    s.until(is_state(SessionState::Disconnected)).await; // the fake client exits at once
    let mut shown = Vec::new();
    while let Ok(b) = s.data.try_recv() {
        shown.extend(b);
    }
    let shown = String::from_utf8_lossy(&shown).into_owned();
    assert!(
        shown.contains("ARGS:127.0.0.1 60001 KEY:4NeCCgvZFe2RnPgrcU1PQw"),
        "{shown:?}"
    );
    assert!(h
        .server
        .execs
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.starts_with("mosh-server new")));

    // The server never started mosh: a clear message, no client.
    *h.server.mosh_reply.lock().unwrap() = Some("bash: mosh-server: command not found\r\n".into());
    let mut s = start_mosh(&h);
    let e = s.until(|e| matches!(e, SessionEvent::Error { .. })).await;
    assert!(
        matches!(e, SessionEvent::Error { ref message, .. } if message.contains("isn't installed"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn os_is_detected_on_the_first_connect_only_and_can_be_switched_off() {
    let h = harness(Some("right-pw")).await;
    *h.server.uname_reply.lock().unwrap() = Some("Linux\nubuntu\n".into());
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    assert_eq!(os_hint(&h).await.as_deref(), Some("ubuntu"));
    let probes = || {
        h.server
            .execs
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.starts_with("uname"))
            .count()
    };
    assert_eq!(probes(), 1);
    // Second connect: already known, nothing more is run on the server.
    let mut s = start(&h);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    assert_eq!(probes(), 1);

    // Switched off in Settings: a new host is never probed.
    let h2 = harness(Some("right-pw")).await;
    h2.state
        .with_db(|c| crate::settings::set(c, "hosts", r#"{"detect_os":false}"#))
        .await
        .unwrap();
    let mut s = trusted(&h2).await;
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    assert!(h2.server.execs.lock().unwrap().is_empty());
    assert_eq!(os_hint(&h2).await, None);
}

#[tokio::test]
async fn key_with_a_certificate_signs_in_as_a_certificate() {
    use russh::keys::ssh_key::certificate::CertType;
    let h = harness(None).await;
    let key = give_host_a_key(&h, None, false).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let cert = sign_user_cert(&key.public_key, (now - 60, now + 86_400), CertType::User);
    let kid = key.id.clone();
    let summary = h
        .state
        .store()
        .with_db_session(move |c, s| crate::keys::set_certificate(c, s, &kid, Some(&cert)))
        .await
        .unwrap();
    assert!(summary.certificate);
    assert!(summary
        .certificate_note
        .as_deref()
        .unwrap()
        .contains("test"));
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await;
    assert_eq!(*h.server.cert_logins.lock().unwrap(), 1); // it really went in as a certificate
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
                        remember: true,
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
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
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
                    port: Some(crate::testservers::port("a")),
                    username: Some("test".into()),
                    password: Some("test".into()),
                    clear_password: false,
                    group_id: None,
                    key_id: None,
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
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
                        remember: true,
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
                    port: Some(crate::testservers::port("a")),
                    username: Some("test".into()),
                    password: None,
                    clear_password: true,
                    group_id: None,
                    key_id: Some(key.id),
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "inherit".into(),
                    jump_host_ids: vec![],
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

// ------------------------------------------------------------------ jump hosts (F05) and startup snippets

pub(crate) fn host_input(label: &str, port: u16, group: Option<&str>) -> HostInput {
    HostInput {
        id: None,
        label: label.into(),
        address: "127.0.0.1".into(),
        port: Some(port),
        username: Some("test".into()),
        password: Some("right-pw".into()),
        clear_password: false,
        group_id: group.map(String::from),
        key_id: None,
        identity_id: None,
        startup_snippet_id: None,
        jump_mode: "inherit".into(),
        jump_host_ids: vec![],
        tags: vec![],
    }
}

/// A second server to act as the bastion, and a host for it.
async fn add_bastion(h: &Harness) -> (String, TestServer) {
    let server = TestServer::default();
    let port = start_server(server.clone()).await;
    let input = host_input("bastion", port, None);
    let host = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
    (host.id, server)
}

async fn set_target(h: &Harness, mode: &str, hops: Vec<String>) {
    let mut input = host_input("test-box", h.port, None);
    input.id = Some(h.host_id.clone());
    input.password = None;
    input.jump_mode = mode.into();
    input.jump_host_ids = hops;
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
}

#[tokio::test]
async fn connects_through_a_jump_host_with_a_prompt_per_hop() {
    let h = harness(Some("right-pw")).await;
    let (bastion, bastion_server) = add_bastion(&h).await;
    set_target(&h, "custom", vec![bastion]).await;

    let mut s = start(&h);
    // The UI is told the route up front, then each hop asks about its own host key.
    match s.until(|e| matches!(e, SessionEvent::Hop { .. })).await {
        SessionEvent::Hop { via } => assert_eq!(via, vec!["bastion"]),
        _ => unreachable!(),
    }
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { ref host, .. } if host == "bastion"));
    s.answer(&id, true, None, true);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { ref host, .. } if host == "test-box"));
    s.answer(&id, true, None, true);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Write(b"hi\n".to_vec())).unwrap();
    assert!(s.read_until("echo: hi").await.contains("echo: hi"));
    assert_eq!(
        bastion_server.forwards.lock().unwrap().len(),
        1,
        "the shell must run through the bastion"
    );
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn a_group_chain_is_inherited_and_a_host_can_opt_out() {
    let h = harness(Some("right-pw")).await;
    let (bastion, bastion_server) = add_bastion(&h).await;
    let group = h
        .state
        .store()
        .with_db_session(move |c, s| {
            crate::groups::save(
                c,
                s,
                &crate::groups::GroupInput {
                    id: None,
                    parent_id: None,
                    label: "Prod".into(),
                    port: None,
                    username: None,
                    password: None,
                    clear_password: false,
                    key_id: None,
                    identity_id: None,
                    startup_snippet_id: None,
                    jump_mode: "custom".into(),
                    jump_host_ids: vec![bastion],
                },
            )
        })
        .await
        .unwrap();
    // Move the target into the group; it sets nothing itself, so it inherits the chain.
    let mut input = host_input("test-box", h.port, Some(&group.id));
    input.id = Some(h.host_id.clone());
    input.password = None;
    let hid = h.host_id.clone();
    let saved = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
    assert_eq!(saved.effective_jump_host_ids.len(), 1);
    assert_eq!(saved.inherited.jump.as_ref().unwrap().group, "Prod");

    let mut s = trusted_chain(&h, 2).await;
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    s.task.await.unwrap();
    assert_eq!(bastion_server.forwards.lock().unwrap().len(), 1);

    // "none" on the host overrides the group: direct again, no new forward.
    let mut input = host_input("test-box", h.port, Some(&group.id));
    input.id = Some(hid);
    input.password = None;
    input.jump_mode = "none".into();
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
    let mut s = start(&h);
    s.until(is_state(SessionState::Ready)).await; // host keys are known now: no prompts
    s.cmds.send(Cmd::Close).unwrap();
    assert_eq!(
        bastion_server.forwards.lock().unwrap().len(),
        1,
        "no hop when the host opts out"
    );
}

/// Answers `n` first-connect prompts with yes.
async fn trusted_chain(h: &Harness, n: usize) -> Running {
    let mut s = start(h);
    for _ in 0..n {
        let (id, _) = s.expect_prompt().await;
        s.answer(&id, true, None, true);
    }
    s
}

#[tokio::test]
async fn startup_snippet_runs_when_the_shell_is_ready() {
    let h = harness(Some("right-pw")).await;
    let snippet_input = crate::snippets::SnippetInput {
        id: None,
        label: "Uptime".into(),
        script: "uptime".into(),
        package_id: None,
        target_host_ids: vec![],
    };
    let sid = h
        .state
        .store()
        .with_db_session(move |c, s| {
            crate::snippets::save(c, &s.vault_id, &snippet_input)?;
            Ok(crate::snippets::list(c, &s.vault_id)?.snippets[0]
                .id
                .clone())
        })
        .await
        .unwrap();
    let mut input = host_input("test-box", h.port, None);
    input.id = Some(h.host_id.clone());
    input.password = None;
    input.startup_snippet_id = Some(sid);
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await;
    assert!(s.read_until("echo: uptime").await.contains("echo: uptime"));
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn jump_chain_rules_and_a_hop_cannot_be_deleted() {
    let h = harness(Some("right-pw")).await;
    let (bastion, _) = add_bastion(&h).await;
    let store = h.state.store();
    // A host can't be its own jump host, and unknown hops are refused.
    let mut input = host_input("test-box", h.port, None);
    input.id = Some(h.host_id.clone());
    input.jump_mode = "custom".into();
    input.jump_host_ids = vec![h.host_id.clone()];
    assert!(store
        .with_db_session({
            let i = input.clone_for_test();
            move |c, s| hosts::save(c, s, &i)
        })
        .await
        .is_err());
    input.jump_host_ids = vec!["nope".into()];
    assert!(store
        .with_db_session({
            let i = input.clone_for_test();
            move |c, s| hosts::save(c, s, &i)
        })
        .await
        .is_err());
    // Using the bastion as a hop locks it against deletion until it is removed from the chain.
    set_target(&h, "custom", vec![bastion.clone()]).await;
    let b = bastion.clone();
    let err = store
        .with_db_session(move |c, s| hosts::delete(c, s, &b))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, ErrorCode::InUse);
    assert!(err.message.contains("test-box"), "{}", err.message);
    set_target(&h, "none", vec![]).await;
    store
        .with_db_session(move |c, s| hosts::delete(c, s, &bastion))
        .await
        .unwrap();
}

/// F05 against real OpenSSH: the container as its own bastion (it forwards to its own port 2222).
#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn docker_jump_through_the_container_to_itself() {
    let h = harness(None).await;
    let mk = |label: &str, jump: Vec<String>| {
        let mut i = host_input(label, crate::testservers::port("a"), None);
        i.password = Some("test".into());
        if !jump.is_empty() {
            i.jump_mode = "custom".into();
            i.jump_host_ids = jump;
        }
        i
    };
    let hop = mk("gateway", vec![]);
    let gw = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &hop))
        .await
        .unwrap();
    let mut inner = mk("inner", vec![gw.id]);
    inner.id = Some(h.host_id.clone());
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &inner))
        .await
        .unwrap();
    let mut s = trusted_chain(&h, 1).await;
    s.until(is_state(SessionState::Ready)).await;
    s.cmds
        .send(Cmd::Write(b"echo via-$((20+22))\n".to_vec()))
        .unwrap();
    assert!(s.read_until("via-42").await.contains("via-42"));
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn keyboard_interactive_asks_the_servers_question_and_signs_in() {
    // A 2FA-style server: no password method at all, just a code. No password may be asked for.
    let h = harness_with(None, |s| s.kbd_code = Some("123456".into())).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, true); // trust the host key
    let (id, kind) = s.expect_prompt().await;
    let PromptKind::KeyboardInteractive {
        prompt, name, echo, ..
    } = kind
    else {
        panic!("expected the server's question, not a password prompt")
    };
    assert_eq!(prompt.trim(), "Verification code:");
    assert_eq!(name, "Two-step sign-in");
    assert!(echo);
    s.answer(&id, true, Some("123456"), false);
    s.until(is_state(SessionState::Ready)).await;

    // A wrong code is a normal sign-in failure.
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, Some("000000"), false);
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, .. } => assert_eq!(code, ErrorCode::AuthFailed),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn connects_with_post_quantum_key_exchange_switched_off_and_the_local_shell_setting_is_checked(
) {
    let h = harness(Some("right-pw")).await;
    h.state
        .with_db(|c| {
            crate::settings::set(
                c,
                "hosts",
                r#"{"post_quantum":false,"local_shell":"/definitely/not/a/shell"}"#,
            )
        })
        .await
        .unwrap();
    let mut s = trusted(&h).await;
    s.until(is_state(SessionState::Ready)).await; // the handshake still works without ML-KEM
    s.cmds.send(Cmd::Close).unwrap();
    // A path that isn't a file is ignored: the user's own shell is used instead.
    let shell = h
        .state
        .with_db(|c| Ok(crate::pty::local_shell(c)))
        .await
        .unwrap();
    assert_eq!(shell, crate::pty::default_shell());
    h.state
        .with_db(|c| crate::settings::set(c, "hosts", r#"{"local_shell":"/bin/sh"}"#))
        .await
        .unwrap();
    let shell = h
        .state
        .with_db(|c| Ok(crate::pty::local_shell(c)))
        .await
        .unwrap();
    assert_eq!(shell, "/bin/sh");
}

#[tokio::test]
async fn a_jump_hop_whose_key_changed_blocks_before_reaching_the_target() {
    let h = harness(Some("right-pw")).await;
    let (bastion, bastion_server) = add_bastion(&h).await;
    set_target(&h, "custom", vec![bastion.clone()]).await;
    // The bastion's saved key is not what it will present.
    let bid = bastion.clone();
    let port = h
        .state
        .store()
        .with_db_session(move |c, s| hosts::get(c, &s.vault_id, &bid))
        .await
        .unwrap()
        .effective_port;
    let pattern = known_hosts::pattern("127.0.0.1", port);
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
    assert!(
        matches!(kind, PromptKind::HostKeyChanged { ref host, .. } if host == "bastion"),
        "the block must name the hop whose key changed"
    );
    s.answer(&id, false, None, false); // Close
    s.until(is_state(SessionState::Closed)).await;
    assert!(
        bastion_server.forwards.lock().unwrap().is_empty(),
        "nothing may be forwarded through a hop that failed the key check"
    );
}

#[tokio::test]
async fn locking_the_vault_ends_live_sessions_and_every_guarded_call_refuses() {
    use crate::ssh::Job;
    let h = harness(Some("right-pw")).await;
    let (etx, mut events) = mpsc::unbounded_channel();
    let io = SessionIo {
        event: Box::new(move |e| {
            let _ = etx.send(e);
        }),
        data: Box::new(|_| {}),
    };
    let sid = h
        .state
        .sessions
        .open(h.state.store(), h.host_id.clone(), Job::Shell, 80, 24, io)
        .unwrap();
    // Trust the host key when asked, then wait for the shell.
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("timed out")
            .expect("session ended early")
        {
            SessionEvent::Prompt { prompt_id, .. } => h
                .state
                .sessions
                .respond(
                    &sid,
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: true,
                    },
                )
                .unwrap(),
            SessionEvent::State {
                state: SessionState::Ready,
            } => break,
            _ => {}
        }
    }
    assert!(h.state.is_unlocked());

    h.state.set_session(None); // lock

    // The live shell is closed, not left running with the key gone.
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(e) = events.recv().await {
            if matches!(
                e,
                SessionEvent::State {
                    state: SessionState::Closed
                }
            ) {
                return true;
            }
        }
        true // the channel closing also means it ended
    })
    .await
    .expect("the session survived the lock");
    assert!(ended);
    // Every guarded entry point now says "locked".
    assert_eq!(
        h.state.require_unlocked().unwrap_err().code,
        ErrorCode::Locked
    );
    assert_eq!(
        h.state.with_session(|_| Ok(())).unwrap_err().code,
        ErrorCode::Locked
    );
    let err = h
        .state
        .with_db_session(|c, s| hosts::tree(c, &s.vault_id))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert!(h.state.sftp.fs("no-such-endpoint").is_err());
}

#[tokio::test]
async fn the_connection_reports_plain_steps_and_never_a_secret() {
    let h = harness(Some("right-pw")).await;
    let mut s = start(&h);
    let mut steps: Vec<String> = Vec::new();
    loop {
        match s.next().await {
            SessionEvent::Step { text } => steps.push(text),
            SessionEvent::Prompt { prompt_id, .. } => s.answer(&prompt_id, true, None, true),
            SessionEvent::State {
                state: SessionState::Ready,
            } => break,
            _ => {}
        }
    }
    let all = steps.join("\n");
    for expected in [
        "Connecting to",
        "First time seeing this server",
        "Signing in as test",
        "Trying the password",
        "Signed in",
        "Shell started",
    ] {
        assert!(all.contains(expected), "missing \"{expected}\" in:\n{all}");
    }
    assert!(
        !all.contains("right-pw"),
        "a password leaked into the steps"
    );
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn connect_once_signs_in_and_saves_nothing_so_the_next_connect_asks_again() {
    let h = harness(Some("right-pw")).await;
    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { .. }));
    s.answer(&id, true, None, false); // "Connect once"
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
    assert!(
        saved_keys(&h).await.is_empty(),
        "trust-once must not save the key"
    );

    let mut s = start(&h);
    let (_, kind) = s.expect_prompt().await;
    assert!(
        matches!(kind, PromptKind::HostKeyNew { .. }),
        "the next connect asks again"
    );
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
async fn a_failed_sign_in_lists_what_the_server_accepts_and_a_chosen_method_is_the_only_one_tried()
{
    // The server only speaks keyboard-interactive: that is the one way to offer.
    let h = harness_with(None, |s| s.kbd_code = Some("123456".into())).await;
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, true); // trust the host key
    match s
        .until(|e| matches!(e, SessionEvent::AuthMethods { .. }))
        .await
    {
        SessionEvent::AuthMethods { methods } => {
            assert_eq!(methods, vec![crate::ssh::AuthMethod::KeyboardInteractive])
        }
        _ => unreachable!(),
    }
    s.cmds.send(Cmd::Close).unwrap();

    // Choosing "key" for a host with no saved key says so, and asks for nothing else.
    // (The task has not run yet on this single-threaded test runtime, so the choice lands first.)
    let mut s = start(&h);
    s.prompts.force_method(Some(crate::ssh::AuthMethod::Key));
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, message } => {
            assert_eq!(code, ErrorCode::AuthFailed);
            assert!(message.contains("no saved key"), "{message}");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn try_another_way_applies_to_the_target_not_the_bastion() {
    let h = harness(Some("right-pw")).await;
    let (bastion, _server) = add_bastion(&h).await;
    set_target(&h, "custom", vec![bastion]).await;
    let mut s = start(&h);
    s.prompts.force_method(Some(crate::ssh::AuthMethod::Key));
    for _ in 0..2 {
        let (id, _) = s.expect_prompt().await;
        s.answer(&id, true, None, true);
    }
    // The bastion signs in by itself; the chosen method (a key) is tried on test-box, which has none.
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { message, .. } => {
            assert!(
                message.contains("test-box") && message.contains("no saved key"),
                "{message}"
            )
        }
        _ => unreachable!(),
    }
}

// ------------------------------------------------------------------ L3: the Docker test servers (T2)
// Run with `scripts/test-servers.sh up`, then `cargo test -- --ignored`. The servers are described in
// `.test-servers/servers.json` (see `crate::testservers`).

/// Points the harness's host at a Docker test server. `password: None` leaves no saved password.
async fn point_host_at(h: &Harness, s: &crate::testservers::Server, password: Option<&str>) {
    let mut i = host_input(
        &s.name,
        s.port.expect("this server has no port on this computer"),
        None,
    );
    i.id = Some(h.host_id.clone());
    i.username = s.user.clone();
    i.password = password.map(String::from);
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &i))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_a_two_step_server_asks_for_the_password_then_the_code() {
    let h = harness(None).await;
    point_host_at(&h, &crate::testservers::server("kbd"), None).await;
    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { .. }));
    s.answer(&id, true, None, true);
    // Two questions from the server's own PAM stack: the password, then the one-time code.
    for _ in 0..2 {
        let (id, kind) = s.expect_prompt().await;
        let PromptKind::KeyboardInteractive { prompt, .. } = kind else {
            panic!("expected one of the server's questions")
        };
        if prompt.to_lowercase().contains("password") {
            s.answer(&id, true, Some("test"), false);
        } else {
            assert!(prompt.contains("Verification code"), "{prompt}");
            s.answer(&id, true, Some(&crate::testservers::otp()), false);
        }
    }
    s.until(is_state(SessionState::Ready)).await;
    s.cmds
        .send(Cmd::Write(b"echo two-step-ok\n".to_vec()))
        .unwrap();
    assert!(s.read_until("two-step-ok").await.contains("two-step-ok"));
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_an_old_key_exchange_is_reported_as_kex_failed() {
    let h = harness(None).await;
    point_host_at(&h, &crate::testservers::server("oldkex"), Some("test")).await;
    let mut s = start(&h);
    match s.until(|e| matches!(e, SessionEvent::Error { .. })).await {
        SessionEvent::Error { code, message } => {
            assert_eq!(code, ErrorCode::KexFailed, "{message}");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_a_rotated_host_key_blocks_as_changed() {
    let h = harness(None).await;
    // `rot` exists for this test: replacing its host key must not disturb the tests that use `a`.
    point_host_at(&h, &crate::testservers::server("rot"), Some("test")).await;
    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    assert!(matches!(kind, PromptKind::HostKeyNew { .. }));
    s.answer(&id, true, None, true); // trust and remember
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();

    crate::testservers::script(&["rotate", "rot"]);

    let mut s = start(&h);
    let (id, kind) = s.expect_prompt().await;
    assert!(
        matches!(kind, PromptKind::HostKeyChanged { .. }),
        "{kind:?}"
    );
    s.answer(&id, false, None, false); // cancel is the safe default
    s.until(|e| {
        matches!(
            e,
            SessionEvent::Error { .. }
                | SessionEvent::State {
                    state: SessionState::Closed
                }
        )
    })
    .await;
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_a_slow_server_takes_its_time_and_still_signs_in() {
    let h = harness(None).await;
    point_host_at(&h, &crate::testservers::server("slow"), Some("test")).await;
    let started = std::time::Instant::now();
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false);
    s.until(is_state(SessionState::Ready)).await;
    assert!(
        started.elapsed() > Duration::from_secs(3),
        "the 800 ms delay was not felt: {:?}",
        started.elapsed()
    );
    s.cmds.send(Cmd::Close).unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_the_http_and_socks_proxies_carry_a_connection_to_the_bastion() {
    use tokio::io::AsyncReadExt;
    let a = crate::testservers::server("a");
    for (socks, name) in [(false, "squid"), (true, "socks")] {
        let proxy = crate::proxy::Proxy {
            socks,
            host: "127.0.0.1".into(),
            port: crate::testservers::port(name),
        };
        // The proxy resolves the Docker name, so this only works through it.
        let mut stream = crate::proxy::connect(&proxy, &a.net_host, a.net_port)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut banner = [0u8; 8];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"SSH-2.0-", "{name}");
    }
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_the_telnet_parser_talks_to_a_real_telnet_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream =
        tokio::net::TcpStream::connect(("127.0.0.1", crate::testservers::port("telnet")))
            .await
            .unwrap();
    let mut parser = crate::telnet::Parser::new(80, 24);
    stream
        .write_all(&crate::telnet::encode_input(b"echo telnet-ok-$((6*7))\n"))
        .await
        .unwrap();
    let mut shown = Vec::new();
    let mut buf = [0u8; 1024];
    tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&shown).contains("telnet-ok-42") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "the server closed the connection");
            let (display, reply) = parser.feed(&buf[..n]);
            shown.extend(display);
            if !reply.is_empty() {
                stream.write_all(&reply).await.unwrap();
            }
        }
    })
    .await
    .expect("timed out waiting for the shell's answer");
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_key_only_server_accepts_the_test_key() {
    let b = crate::testservers::server("b");
    let h = harness(None).await;
    let pem = std::fs::read_to_string(crate::testservers::root().join(b.key.as_deref().unwrap()))
        .unwrap();
    let key = h
        .state
        .store()
        .with_db_session(move |c, s| {
            let new = crate::keys::parse_import(&crate::keys::ImportInput {
                label: "test key".into(),
                pem: Some(pem),
                path: None,
                passphrase: None,
                save_passphrase: false,
            })?;
            crate::keys::store(c, s, &new)
        })
        .await
        .unwrap();
    let mut i = host_input("keyuser", b.port.unwrap(), None);
    i.id = Some(h.host_id.clone());
    i.username = b.user.clone();
    i.password = None;
    i.key_id = Some(key.id);
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &i))
        .await
        .unwrap();
    let mut s = start(&h);
    let (id, _) = s.expect_prompt().await;
    s.answer(&id, true, None, false);
    s.until(is_state(SessionState::Ready)).await;
    s.cmds.send(Cmd::Close).unwrap();
}
