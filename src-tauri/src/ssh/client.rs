//! The connection task: TCP -> SSH handshake (with host-key check) -> password auth -> PTY shell,
//! then a loop that moves bytes both ways.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout, MissedTickBehavior};
use zeroize::Zeroizing;

use super::{Cmd, PromptKind, Prompts, SessionEvent, SessionIo, SessionState};
use crate::error::ErrorCode;
use crate::hosts;
use crate::keys;
use crate::known_hosts::{self, Check};
use crate::state::Store;

pub struct Params {
    pub host_id: String,
    pub cols: u32,
    pub rows: u32,
}

/// Flush output to the UI at least this often, or sooner once this much has piled up.
const FLUSH_EVERY: Duration = Duration::from_millis(8);
const FLUSH_AT: usize = 64 * 1024;
/// Stop reading from the server while the UI is this far behind (it acks what it has drawn).
const MAX_UNACKED: usize = 1024 * 1024;
const TCP_TIMEOUT: Duration = Duration::from_secs(10);

/// Why the handshake refused the server's key (so we can tell "user said no" from a failure).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Rejected {
    ByUser,
    Unsupported,
    Failed(String),
}

pub struct Client {
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    label: String,
    pattern: String,
    rejected: Arc<Mutex<Option<Rejected>>>,
    /// Set for a remote (`-R`) forward: where connections the server sends us should go.
    remote: Option<Arc<crate::tunnels::RemoteHook>>,
    /// The host asked for agent forwarding: accept the agent channels the server opens.
    agent: bool,
}

impl Client {
    fn reject(&self, why: Rejected) -> Result<bool, russh::Error> {
        if let Ok(mut r) = self.rejected.lock() {
            *r = Some(why);
        }
        Ok(false)
    }
}

impl Handler for Client {
    type Error = russh::Error;

    /// Remote forward: the server accepted a connection on its listening port and hands it to us.
    /// We connect to the local destination and pipe the two together. Without a hook, refuse.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<russh::client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        let Some(hook) = self.remote.clone() else {
            return Ok(()); // dropping `reply` rejects the channel
        };
        if let Ok(tcp) =
            tokio::net::TcpStream::connect((hook.dest_address.as_str(), hook.dest_port)).await
        {
            reply.accept().await;
            tokio::spawn(crate::tunnels::pipe(
                tcp,
                channel.into_stream(),
                Arc::clone(&hook.open),
            ));
        }
        Ok(())
    }

    /// The server wants to use our agent (we asked with `agent_forward`). Refused unless the host
    /// has forwarding on and a local agent is running.
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<russh::client::Msg>,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        if self.agent && crate::agent::serve(channel.into_stream()).await {
            reply.accept().await;
        }
        Ok(())
    }

    /// F01 / F12. Known key: continue. Unknown: ask, and trust only on a yes. Changed: block
    /// unless the user explicitly replaces the saved key.
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return self.reject(Rejected::Unsupported); // host certificates: not supported yet
        };
        let key_type = key.algorithm().to_string();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        // "ssh-ed25519 AAAA..." -> keep only the base64 blob.
        let blob = match key
            .to_openssh()
            .ok()
            .and_then(|l| l.split_whitespace().nth(1).map(String::from))
        {
            Some(b) => b,
            None => return self.reject(Rejected::Failed("unreadable server key".into())),
        };

        let (pattern, kt, b) = (self.pattern.clone(), key_type.clone(), blob.clone());
        let check = self
            .store
            .with_db_session(move |c, s| known_hosts::check(c, &s.vault_id, &pattern, &kt, &b))
            .await;
        let kind = match check {
            Ok(Check::Known) => return Ok(true),
            Ok(Check::Unknown) => PromptKind::HostKeyNew {
                host: self.label.clone(),
                key_type: key_type.clone(),
                fingerprint: fingerprint.clone(),
            },
            Ok(Check::Changed { old_fingerprint }) => PromptKind::HostKeyChanged {
                host: self.label.clone(),
                key_type: key_type.clone(),
                old_fingerprint,
                new_fingerprint: fingerprint.clone(),
            },
            Err(e) => return self.reject(Rejected::Failed(e.message)),
        };
        if self.prompts.ask(&self.io, kind).await.is_none() {
            return self.reject(Rejected::ByUser);
        }
        let (pattern, kt) = (self.pattern.clone(), key_type);
        let saved = self
            .store
            .with_db_session(move |c, s| {
                known_hosts::trust(c, &s.vault_id, &pattern, &kt, &blob, &fingerprint)
            })
            .await;
        match saved {
            Ok(()) => Ok(true),
            Err(e) => self.reject(Rejected::Failed(e.message)),
        }
    }
}

fn emit_state(io: &SessionIo, state: SessionState) {
    (io.event)(SessionEvent::State { state });
}

fn emit_error(io: &SessionIo, code: ErrorCode, message: impl Into<String>) {
    (io.event)(SessionEvent::Error {
        code,
        message: message.into(),
    });
}

/// A connected, signed-in SSH connection and the host's label.
pub struct Connected {
    pub handle: client::Handle<Client>,
    pub label: String,
    /// The host's startup snippet, to type once the shell is ready.
    pub startup_script: Option<String>,
    /// Ask for agent forwarding on the shell.
    pub agent_forwarding: bool,
    /// The jump hosts the connection runs through. They must stay alive as long as `handle` does.
    _hops: Vec<client::Handle<Client>>,
}

/// Reports a failure to the UI and gives back `None`, so callers can `return fail(...)`.
fn fail<T>(io: &SessionIo, code: ErrorCode, message: impl Into<String>) -> Option<T> {
    emit_error(io, code, message);
    None
}

fn closed<T>(io: &SessionIo) -> Option<T> {
    emit_state(io, SessionState::Closed);
    None
}

/// One SSH handshake over any byte stream (a TCP socket, or a channel through a jump host), then sign in.
/// The host-key prompt happens inside `connect_stream`; sign-in failures are reported through `io`.
#[allow(clippy::too_many_arguments)] // one call site per hop; a struct would just move the list
async fn handshake<R>(
    stream: R,
    store: &Store,
    io: &Arc<SessionIo>,
    prompts: &Arc<Prompts>,
    info: &hosts::ConnectInfo,
    user: &str,
    pattern: &str,
    remote: Option<Arc<crate::tunnels::RemoteHook>>,
    agent: bool,
) -> Option<client::Handle<Client>>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let rejected = Arc::new(Mutex::new(None));
    let handler = Client {
        store: store.clone(),
        io: Arc::clone(io),
        prompts: Arc::clone(prompts),
        label: info.label.clone(),
        pattern: pattern.to_string(),
        rejected: Arc::clone(&rejected),
        remote,
        agent,
    };
    let config = Arc::new(client::Config {
        // Detect dead links (F11): 3 missed keepalives, 30 s apart.
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let mut handle = match client::connect_stream(config, stream, handler).await {
        Ok(h) => h,
        Err(e) => {
            let why = rejected.lock().ok().and_then(|r| r.clone());
            return match why {
                Some(Rejected::ByUser) => closed(io),
                Some(Rejected::Unsupported) => fail(
                    io,
                    ErrorCode::KexFailed,
                    "This server uses a host certificate, which Dalan doesn't support yet.",
                ),
                Some(Rejected::Failed(m)) => fail(io, ErrorCode::Internal, m),
                None => fail(
                    io,
                    ErrorCode::KexFailed,
                    format!("Couldn't agree on encryption with {pattern}: {e}"),
                ),
            };
        }
    };
    emit_state(io, SessionState::Authenticating);
    if !authenticate(&mut handle, store, io, prompts, info, user, &info.id).await {
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
        return None;
    }
    Some(handle)
}

/// Steps shared by shells and key export: resolve the host and its jump chain, then for each hop
/// (TCP for the first, a forwarded channel for the rest) handshake, check the host key and sign in.
/// Every hop asks its own questions. Failures are reported through `io`.
pub async fn connect_authenticated(
    store: &Store,
    io: &Arc<SessionIo>,
    prompts: &Arc<Prompts>,
    host_id: &str,
    remote: Option<Arc<crate::tunnels::RemoteHook>>,
) -> Option<Connected> {
    emit_state(io, SessionState::Connecting);

    let hid = host_id.to_string();
    let plan = match store
        .with_db_session(move |c, s| hosts::connect_plan(c, s, &hid))
        .await
    {
        Ok(p) => p,
        Err(e) => return fail(io, e.code, e.message),
    };
    let last = plan.len() - 1;
    if matches!(plan[last].protocol.as_str(), "telnet" | "serial") {
        let what = if plan[last].protocol == "serial" {
            "a serial line"
        } else {
            "Telnet"
        };
        return fail(
            io,
            ErrorCode::Validation,
            format!(
                "{} uses {what}, which has no files, tunnels or keys.",
                plan[last].label
            ),
        );
    }
    if last > 0 {
        let via = plan[..last].iter().map(|i| i.label.clone()).collect();
        (io.event)(SessionEvent::Hop { via });
    }

    let mut hops: Vec<client::Handle<Client>> = Vec::new();
    for (i, info) in plan.iter().enumerate() {
        let Some(user) = info.username.clone() else {
            return fail(
                io,
                ErrorCode::AuthFailed,
                format!("{} has no username. Edit the host to add one.", info.label),
            );
        };
        let pattern = known_hosts::pattern(&info.address, info.port);

        let handle = match hops.last() {
            // Later hops ride a direct-tcpip channel opened on the previous hop.
            Some(prev) => {
                let ch = match prev
                    .channel_open_direct_tcpip(
                        info.address.clone(),
                        u32::from(info.port),
                        "127.0.0.1",
                        0,
                    )
                    .await
                {
                    Ok(ch) => ch,
                    Err(e) => {
                        return fail(
                            io,
                            ErrorCode::HostUnreachable,
                            format!("Can't reach {pattern} through {}: {e}", plan[i - 1].label),
                        )
                    }
                };
                handshake(
                    ch.into_stream(),
                    store,
                    io,
                    prompts,
                    info,
                    &user,
                    &pattern,
                    remote.clone().filter(|_| i == last),
                    info.agent_forwarding && i == last,
                )
                .await?
            }
            None => {
                // Through the host's proxy if it has one, else straight to the server.
                let dial = async {
                    match &info.proxy {
                        Some(p) => crate::proxy::connect(p, &info.address, info.port).await,
                        None => {
                            tokio::net::TcpStream::connect((info.address.as_str(), info.port)).await
                        }
                    }
                };
                let tcp = match timeout(TCP_TIMEOUT, dial).await {
                    Ok(Ok(s)) => s,
                    Ok(Err(e)) => {
                        return fail(
                            io,
                            ErrorCode::HostUnreachable,
                            format!("Can't reach {pattern}: {e}"),
                        )
                    }
                    Err(_) => {
                        return fail(
                            io,
                            ErrorCode::HostUnreachable,
                            format!("Can't reach {pattern}: timed out"),
                        )
                    }
                };
                let _ = tcp.set_nodelay(true);
                handshake(
                    tcp,
                    store,
                    io,
                    prompts,
                    info,
                    &user,
                    &pattern,
                    remote.clone().filter(|_| i == last),
                    info.agent_forwarding && i == last,
                )
                .await?
            }
        };
        if i == last {
            return Some(Connected {
                handle,
                label: info.label.clone(),
                startup_script: info.startup_script.clone(),
                agent_forwarding: info.agent_forwarding,
                _hops: hops,
            });
        }
        hops.push(handle);
    }
    None
}

enum KeyResult {
    Accepted,
    /// The server said no to this key (carries the key's label).
    Rejected(String),
    /// Cancelled or failed; the UI has already been told.
    Stop,
}

/// F02: sign in with the host's key. An encrypted key needs its passphrase (saved, or asked for).
async fn try_key(
    handle: &mut client::Handle<Client>,
    store: &Store,
    io: &Arc<SessionIo>,
    prompts: &Arc<Prompts>,
    key_id: &str,
    user: &str,
) -> KeyResult {
    let kid = key_id.to_string();
    let loaded = match store
        .with_db_session(move |c, s| keys::load_for_auth(c, s, &kid))
        .await
    {
        Ok(l) => l,
        Err(e) => {
            emit_error(io, e.code, e.message);
            return KeyResult::Stop;
        }
    };
    let passphrase: Option<Zeroizing<String>> = if !loaded.encrypted {
        None
    } else if let Some(p) = &loaded.saved_passphrase {
        Some(p.clone())
    } else {
        let kind = PromptKind::Passphrase {
            key_label: loaded.label.clone(),
        };
        let Some(a) = prompts.ask(io, kind).await else {
            emit_state(io, SessionState::Closed);
            return KeyResult::Stop;
        };
        let pw = Zeroizing::new(a.text.unwrap_or_default());
        if a.remember && !pw.is_empty() {
            let (id, label, pw2) = (key_id.to_string(), loaded.label.clone(), pw.to_string());
            let _ = store
                .with_db_session(move |c, s| {
                    keys::update(
                        c,
                        s,
                        &keys::KeyUpdate {
                            id,
                            label,
                            save_passphrase: Some(pw2),
                            forget_passphrase: false,
                        },
                    )
                })
                .await;
        }
        Some(pw)
    };
    let key = match keys::open_private(&loaded, passphrase.as_ref().map(|p| p.as_str())) {
        Ok(k) => k,
        Err(_) => {
            emit_error(
                io,
                ErrorCode::AuthFailed,
                format!(
                    "That passphrase didn't work for the key \"{}\".",
                    loaded.label
                ),
            );
            return KeyResult::Stop;
        }
    };
    // RSA keys sign with SHA-2 when the server supports it; other types have no choice to make.
    let hash = if matches!(key.algorithm(), russh::keys::Algorithm::Rsa { .. }) {
        handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten()
    } else {
        None
    };
    let result = match loaded
        .certificate
        .as_deref()
        .map(russh::keys::Certificate::from_openssh)
    {
        // Key + certificate: the server trusts the CA that signed the certificate, not the bare key.
        Some(Ok(cert)) => {
            if keys::certificate_expired(&cert) {
                emit_error(
                    io,
                    ErrorCode::AuthFailed,
                    format!("The certificate for the key \"{}\" has expired. Attach a new one in Keychain.", loaded.label),
                );
                return KeyResult::Stop;
            }
            handle
                .authenticate_openssh_cert(user, Arc::new(key), cert)
                .await
        }
        _ => {
            handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
        }
    };
    match result {
        Ok(r) if r.success() => KeyResult::Accepted,
        Ok(_) => KeyResult::Rejected(loaded.label.clone()),
        Err(e) => {
            emit_error(
                io,
                ErrorCode::HostUnreachable,
                format!("Connection lost while signing in: {e}"),
            );
            KeyResult::Stop
        }
    }
}

/// Key first (if the host has one), then the password: saved, else asked for.
/// Returns false after reporting why it failed.
async fn authenticate(
    handle: &mut client::Handle<Client>,
    store: &Store,
    io: &Arc<SessionIo>,
    prompts: &Arc<Prompts>,
    info: &hosts::ConnectInfo,
    user: &str,
    host_id: &str,
) -> bool {
    let mut rejected_key = None;
    if let Some(key_id) = &info.key_id {
        match try_key(handle, store, io, prompts, key_id, user).await {
            KeyResult::Accepted => return true,
            KeyResult::Rejected(label) => rejected_key = Some(label),
            KeyResult::Stop => return false,
        }
    }
    let password: Zeroizing<String> = match (&info.password, &rejected_key) {
        (Some(p), _) => p.clone(),
        // A key was refused and there is no password to fall back on: say so, don't ask for one.
        (None, Some(label)) => {
            emit_error(
                io,
                ErrorCode::AuthFailed,
                format!("{} rejected the key \"{label}\".", info.label),
            );
            return false;
        }
        (None, None) => {
            let kind = PromptKind::Password {
                host: info.label.clone(),
                user: user.to_string(),
            };
            let Some(a) = prompts.ask(io, kind).await else {
                emit_state(io, SessionState::Closed);
                return false;
            };
            let pw = Zeroizing::new(a.text.unwrap_or_default());
            if a.remember && !pw.is_empty() {
                let (hid, pw2) = (host_id.to_string(), pw.clone());
                let _ = store
                    .with_db_session(move |c, s| hosts::save_password(c, s, &hid, &pw2))
                    .await;
            }
            pw
        }
    };
    match handle.authenticate_password(user, password.as_str()).await {
        Ok(r) if r.success() => true,
        Ok(_) => {
            emit_error(
                io,
                ErrorCode::AuthFailed,
                format!("{} rejected the password.", info.label),
            );
            false
        }
        Err(e) => {
            emit_error(
                io,
                ErrorCode::HostUnreachable,
                format!("Connection lost while signing in: {e}"),
            );
            false
        }
    }
}

/// Runs one command on its own channel and returns what it printed (capped at `max` bytes), or
/// `None` when the channel couldn't be opened or `limit` passed.
pub async fn exec_output(
    handle: &client::Handle<Client>,
    command: &str,
    max: usize,
    limit: Duration,
) -> Option<Vec<u8>> {
    let ask = async {
        let mut ch = handle.channel_open_session().await.ok()?;
        ch.exec(true, command).await.ok()?;
        let mut out = Vec::new();
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    out.extend_from_slice(&data);
                    if out.len() >= max {
                        break;
                    }
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = ch.close().await;
        Some(out)
    };
    timeout(limit, ask).await.ok().flatten()
}

/// Runs the OS-detection command on its own channel. Slow or silent servers give `unknown`
/// after a short wait, so a connect is never held up for long.
async fn detect_os(handle: &client::Handle<Client>) -> String {
    let ask = async {
        let mut ch = handle.channel_open_session().await.ok()?;
        ch.exec(true, crate::osdetect::COMMAND).await.ok()?;
        let mut out = Vec::new();
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    out.extend_from_slice(&data);
                    if out.len() > 4096 {
                        break;
                    }
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = ch.close().await;
        crate::osdetect::parse_os(&String::from_utf8_lossy(&out))
    };
    timeout(Duration::from_secs(3), ask)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::osdetect::UNKNOWN.to_string())
}

pub async fn run(
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    p: Params,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let Some(conn) = connect_authenticated(&store, &io, &prompts, &p.host_id, None).await else {
        return;
    };

    // First connect to this host: ask which OS it is, once (Settings can switch this off).
    let hid = p.host_id.clone();
    if store
        .with_db_session(move |c, s| Ok(crate::osdetect::needed(c, &s.vault_id, &hid)))
        .await
        .unwrap_or(false)
    {
        let hint = detect_os(&conn.handle).await;
        let hid = p.host_id.clone();
        let _ = store
            .with_db_session(move |c, s| crate::osdetect::set(c, &s.vault_id, &hid, &hint))
            .await;
    }

    // 4. PTY + shell.
    let handle = &conn.handle;
    let mut channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            return emit_error(
                &io,
                ErrorCode::Internal,
                format!("Couldn't open a shell: {e}"),
            )
        }
    };
    if conn.agent_forwarding {
        let _ = channel.agent_forward(false).await;
    }
    // Variables for this host (after inheritance). Sent before the shell starts, without waiting for
    // an answer: a server that doesn't accept a name just ignores it.
    let hid = p.host_id.clone();
    let vars = store
        .with_db_session(move |c, s| crate::env::resolve_for_host(c, &s.vault_id, &hid))
        .await
        .unwrap_or_default();
    for v in vars {
        let _ = channel.set_env(false, v.name, v.value).await;
    }
    if channel
        .request_pty(false, "xterm-256color", p.cols, p.rows, 0, 0, &[])
        .await
        .is_err()
        || channel.request_shell(false).await.is_err()
    {
        return emit_error(
            &io,
            ErrorCode::Internal,
            "The server refused to start a shell.",
        );
    }
    emit_state(&io, SessionState::Ready);
    // The host's startup snippet runs as if typed. A failure to send is not worth ending the session over.
    if let Some(script) = &conn.startup_script {
        let mut bytes = script.trim_end().as_bytes().to_vec();
        bytes.push(b'\n');
        let _ = channel.data_bytes(bytes).await;
    }
    let hid = p.host_id.clone();
    let _ = store
        .with_db_session(move |c, s| hosts::mark_connected(c, &s.vault_id, &hid))
        .await;

    // 5. Move bytes.
    let unacked = Arc::new(AtomicUsize::new(0));
    let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_AT);
    let mut tick = interval(FLUSH_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut exit_status = None;
    let mut closed_by_user = false;

    let flush = |buf: &mut Vec<u8>| {
        if !buf.is_empty() {
            unacked.fetch_add(buf.len(), Ordering::Relaxed);
            (io.data)(std::mem::take(buf)); // hands the whole buffer over, leaves a fresh empty one
        }
    };

    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Close) => { closed_by_user = true; break; }
                Some(Cmd::Write(bytes)) => { if channel.data_bytes(bytes).await.is_err() { break; } }
                Some(Cmd::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                Some(Cmd::Ack(n)) => {
                    let _ = unacked.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |u| Some(u.saturating_sub(n)));
                }
            },
            // Not polled while the UI is behind: the SSH window fills and the server pauses.
            msg = channel.wait(), if unacked.load(Ordering::Relaxed) < MAX_UNACKED => match msg {
                None | Some(ChannelMsg::Eof | ChannelMsg::Close) => break,
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    buf.extend_from_slice(&data);
                    if buf.len() >= FLUSH_AT { flush(&mut buf); }
                }
                Some(ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
                Some(_) => {}
            },
            _ = tick.tick() => flush(&mut buf),
        }
    }
    flush(&mut buf);
    if closed_by_user {
        let _ = channel.close().await;
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
        emit_state(&io, SessionState::Closed);
    } else {
        (io.event)(SessionEvent::Exit {
            status: exit_status,
        });
        emit_state(&io, SessionState::Disconnected);
    }
}

/// What `~` expands to, and which characters may appear in an install path. The path goes into a
/// shell command, so it is checked strictly instead of quoted cleverly.
fn safe_path_part(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
}

/// S08: append the public key to `authorized_keys` on the server (folder 700, file 600, no
/// duplicates), then link the key to the host. Prompts (host key, password) arrive on `io` like a shell's.
pub async fn run_export(
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    host_id: String,
    key_id: String,
    dir: String,
    file: String,
) {
    let dir = dir.trim().to_string();
    let rel = dir.strip_prefix("~/").unwrap_or(&dir).to_string();
    if !safe_path_part(&rel) || !safe_path_part(&file) || file.contains('/') {
        return emit_error(
            &io,
            ErrorCode::Validation,
            "Use a plain folder and file name, like ~/.ssh and authorized_keys.",
        );
    }
    let kid = key_id.clone();
    let key = match store
        .with_db_session(move |c, s| keys::get(c, s, &kid))
        .await
    {
        Ok(k) => k,
        Err(e) => return emit_error(&io, e.code, e.message),
    };
    let Some(conn) = connect_authenticated(&store, &io, &prompts, &host_id, None).await else {
        return;
    };
    // Absolute paths stay as typed; `~/x` becomes "$HOME/x".
    let base = if dir.starts_with("~/") {
        format!("\"$HOME\"/{rel}")
    } else {
        rel.clone()
    };
    let line = key.public_key.replace('\'', "'\\''");
    let cmd = format!(
        "umask 077; mkdir -p {base} && chmod 700 {base} && touch {base}/{file} && chmod 600 {base}/{file} && (grep -qxF '{line}' {base}/{file} || printf '%s\\n' '{line}' >> {base}/{file})"
    );
    let result = async {
        let mut ch = conn.handle.channel_open_session().await.ok()?;
        ch.exec(true, cmd).await.ok()?;
        let mut status = None;
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        status
    }
    .await;
    let _ = conn
        .handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    match result {
        Some(0) => {
            let (hid, kid) = (host_id, key_id);
            match store
                .with_db_session(move |c, s| hosts::link_key(c, &s.vault_id, &hid, &kid))
                .await
            {
                Ok(()) => {
                    (io.event)(SessionEvent::Exit { status: Some(0) });
                    emit_state(&io, SessionState::Closed);
                }
                Err(e) => emit_error(&io, e.code, e.message),
            }
        }
        _ => emit_error(
            &io,
            ErrorCode::PermissionDenied,
            format!("{} didn't let Dalan write the key there.", conn.label),
        ),
    }
}
