//! The connection task: TCP -> SSH handshake (with host-key check) -> password auth -> PTY shell,
//! then a loop that moves bytes both ways.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout, MissedTickBehavior};
use zeroize::Zeroizing;

use super::{Cmd, PromptKind, Prompts, SessionEvent, SessionIo, SessionState};
use crate::error::ErrorCode;
use crate::hosts;
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

struct Client {
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    label: String,
    pattern: String,
    rejected: Arc<Mutex<Option<Rejected>>>,
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

pub async fn run(
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    p: Params,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    emit_state(&io, SessionState::Connecting);

    let host_id = p.host_id.clone();
    let info = match store
        .with_db_session(move |c, s| hosts::connect_info(c, s, &host_id))
        .await
    {
        Ok(i) => i,
        Err(e) => return emit_error(&io, e.code, e.message),
    };
    let Some(user) = info.username.clone() else {
        return emit_error(
            &io,
            ErrorCode::AuthFailed,
            format!("{} has no username. Edit the host to add one.", info.label),
        );
    };
    let pattern = known_hosts::pattern(&info.address, info.port);

    // 1. TCP.
    let tcp = match timeout(
        TCP_TIMEOUT,
        tokio::net::TcpStream::connect((info.address.as_str(), info.port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return emit_error(
                &io,
                ErrorCode::HostUnreachable,
                format!("Can't reach {pattern}: {e}"),
            )
        }
        Err(_) => {
            return emit_error(
                &io,
                ErrorCode::HostUnreachable,
                format!("Can't reach {pattern}: timed out"),
            )
        }
    };
    let _ = tcp.set_nodelay(true);

    // 2. SSH handshake. The host-key prompt happens inside this call.
    let rejected = Arc::new(Mutex::new(None));
    let handler = Client {
        store: store.clone(),
        io: Arc::clone(&io),
        prompts: Arc::clone(&prompts),
        label: info.label.clone(),
        pattern: pattern.clone(),
        rejected: Arc::clone(&rejected),
    };
    let config = Arc::new(client::Config {
        // Detect dead links (F11): 3 missed keepalives, 30 s apart.
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let mut handle = match client::connect_stream(config, tcp, handler).await {
        Ok(h) => h,
        Err(e) => {
            let why = rejected.lock().ok().and_then(|r| r.clone());
            return match why {
                Some(Rejected::ByUser) => emit_state(&io, SessionState::Closed),
                Some(Rejected::Unsupported) => emit_error(
                    &io,
                    ErrorCode::KexFailed,
                    "This server uses a host certificate, which Dalan doesn't support yet.",
                ),
                Some(Rejected::Failed(m)) => emit_error(&io, ErrorCode::Internal, m),
                None => emit_error(
                    &io,
                    ErrorCode::KexFailed,
                    format!("Couldn't agree on encryption with {pattern}: {e}"),
                ),
            };
        }
    };

    // 3. Password. Saved one first; otherwise ask (and remember if the user ticks the box).
    emit_state(&io, SessionState::Authenticating);
    let password: Zeroizing<String> = match info.password {
        Some(p) => p,
        None => {
            let kind = PromptKind::Password {
                host: info.label.clone(),
                user: user.clone(),
            };
            let Some(a) = prompts.ask(&io, kind).await else {
                let _ = handle
                    .disconnect(russh::Disconnect::ByApplication, "", "en")
                    .await;
                return emit_state(&io, SessionState::Closed);
            };
            let pw = Zeroizing::new(a.text.unwrap_or_default());
            if a.remember && !pw.is_empty() {
                let (hid, pw2) = (p.host_id.clone(), pw.clone());
                let _ = store
                    .with_db_session(move |c, s| hosts::save_password(c, s, &hid, &pw2))
                    .await;
            }
            pw
        }
    };
    match handle.authenticate_password(user, password.as_str()).await {
        Ok(r) if r.success() => {}
        Ok(_) => {
            return emit_error(
                &io,
                ErrorCode::AuthFailed,
                format!("{} rejected the password.", info.label),
            )
        }
        Err(e) => {
            return emit_error(
                &io,
                ErrorCode::HostUnreachable,
                format!("Connection lost while signing in: {e}"),
            )
        }
    }
    drop(password); // wiped now, not at the end of the session

    // 4. PTY + shell.
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
