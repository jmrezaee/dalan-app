//! The task behind one running rule: connect, serve the forward, watch the connection, and
//! reconnect with backoff (1 s doubling to 30 s) when it drops.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::{
    permanent, pipe, Emit, ForwardKind, ForwardRule, RemoteHook, TunnelState, TunnelStatus,
    UserEvents,
};
use crate::error::ErrorCode;
use crate::ssh::{connect_authenticated, Connected, Prompts, SessionEvent, SessionIo};
use crate::state::Store;

const MAX_BACKOFF: u64 = 30;
const POLL: Duration = Duration::from_millis(400);

fn publish(
    status: &Mutex<TunnelStatus>,
    emit: &Emit,
    state: TunnelState,
    error: Option<String>,
    connections: u32,
) {
    let snapshot = {
        let Ok(mut s) = status.lock() else { return };
        s.state = state;
        s.error = error;
        s.connections = connections;
        s.clone()
    };
    emit(snapshot);
}

/// Sleeps, or returns `true` early if the rule was stopped.
async fn sleep_or_cancel(secs: u64, cancel: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
        _ = cancel.changed() => true,
    }
}

pub async fn run_tunnel(
    store: Store,
    rule: ForwardRule,
    user_events: Option<UserEvents>,
    prompts: Arc<Prompts>,
    status: Arc<Mutex<TunnelStatus>>,
    mut cancel: watch::Receiver<bool>,
    emit: Emit,
) {
    // The connection code reports problems as session events. Keep the last error for the status line,
    // and pass everything on to the UI when someone is there to see it.
    let last_error: Arc<Mutex<Option<(ErrorCode, String)>>> = Arc::default();
    let io = {
        let last = Arc::clone(&last_error);
        let user = user_events;
        Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::Error { code, message } = &e {
                    if let Ok(mut l) = last.lock() {
                        *l = Some((*code, message.clone()));
                    }
                }
                if let Some(u) = &user {
                    u(e);
                }
            }),
            data: Box::new(|_| {}),
        })
    };
    let open = Arc::new(AtomicU32::new(0));
    let hook = (rule.kind == ForwardKind::Remote).then(|| {
        Arc::new(RemoteHook {
            dest_address: rule.dest_address.clone().unwrap_or_default(),
            dest_port: rule.dest_port.unwrap_or(0),
            open: Arc::clone(&open),
        })
    });

    let mut backoff = 1u64;
    let mut reconnecting = false;
    loop {
        publish(
            &status,
            &emit,
            if reconnecting {
                TunnelState::Reconnecting
            } else {
                TunnelState::Starting
            },
            None,
            0,
        );
        if let Ok(mut l) = last_error.lock() {
            *l = None;
        }
        let conn = tokio::select! {
            c = connect_authenticated(&store, &io, &prompts, &rule.host_id, hook.clone(), false) => c,
            _ = cancel.changed() => return,
        };
        let Some(conn) = conn else {
            let err = last_error.lock().ok().and_then(|mut l| l.take());
            match err {
                // Network trouble: keep trying, slowly.
                Some((code, msg)) if !permanent(code) => {
                    publish(&status, &emit, TunnelState::Reconnecting, Some(msg), 0);
                    if sleep_or_cancel(backoff, &mut cancel).await {
                        return;
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    reconnecting = true;
                    continue;
                }
                Some((_, msg)) => return publish(&status, &emit, TunnelState::Error, Some(msg), 0),
                None => {
                    let msg = format!("{} needs an answer from you (a host key or a password). Start this rule yourself.", rule.label);
                    return publish(&status, &emit, TunnelState::Error, Some(msg), 0);
                }
            }
        };

        let conn = Arc::new(conn);
        let accept = match serve(&rule, &conn, &open).await {
            Ok(a) => a,
            Err(msg) => {
                let _ = conn
                    .handle
                    .disconnect(russh::Disconnect::ByApplication, "", "en")
                    .await;
                return publish(&status, &emit, TunnelState::Error, Some(msg), 0);
            }
        };
        publish(
            &status,
            &emit,
            TunnelState::Active,
            None,
            open.load(Ordering::Relaxed),
        );
        backoff = 1;
        reconnecting = true;

        // Watch the connection and keep the connection count fresh.
        let mut shown = open.load(Ordering::Relaxed);
        let stopped = loop {
            tokio::select! {
                _ = cancel.changed() => break true,
                _ = tokio::time::sleep(POLL) => {
                    if conn.handle.is_closed() {
                        break false;
                    }
                    let n = open.load(Ordering::Relaxed);
                    if n != shown {
                        shown = n;
                        publish(&status, &emit, TunnelState::Active, None, n);
                    }
                }
            }
        };
        if let Some(a) = accept {
            a.abort();
        }
        if rule.kind == ForwardKind::Remote {
            let _ = conn
                .handle
                .cancel_tcpip_forward(rule.bind_address.clone(), u32::from(rule.bind_port))
                .await;
        }
        let _ = conn
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
        open.store(0, Ordering::Relaxed);
        if stopped {
            return;
        }
        let msg = format!("The connection to the server dropped. Reconnecting in {backoff}s.");
        publish(&status, &emit, TunnelState::Reconnecting, Some(msg), 0);
        if sleep_or_cancel(backoff, &mut cancel).await {
            return;
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Starts serving the rule on a live connection. For `-L` and `-D` that is a local listener (its
/// accept task is returned so it can be stopped); for `-R` it asks the server to listen.
async fn serve(
    rule: &ForwardRule,
    conn: &Arc<Connected>,
    open: &Arc<AtomicU32>,
) -> Result<Option<JoinHandle<()>>, String> {
    match rule.kind {
        ForwardKind::Remote => {
            conn.handle
                .tcpip_forward(rule.bind_address.clone(), u32::from(rule.bind_port))
                .await
                .map_err(|_| {
                    format!(
                        "{} refused the remote port {}. Check that the port is free there, and that the server allows forwarding (GatewayPorts for anything but localhost).",
                        conn.label, rule.bind_port
                    )
                })?;
            Ok(None)
        }
        ForwardKind::Local | ForwardKind::Dynamic => {
            let listener = TcpListener::bind((rule.bind_address.as_str(), rule.bind_port)).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::AddrInUse => format!(
                    "Port {} is already in use on this computer. Pick another, or stop whatever is using it.",
                    rule.bind_port
                ),
                std::io::ErrorKind::PermissionDenied => format!(
                    "Dalan isn't allowed to listen on port {}. Ports below 1024 need special permission.",
                    rule.bind_port
                ),
                _ => format!("Couldn't listen on {}:{}: {e}", rule.bind_address, rule.bind_port),
            })?;
            let (conn, open, kind) = (Arc::clone(conn), Arc::clone(open), rule.kind);
            let dest = (
                rule.dest_address.clone().unwrap_or_default(),
                u32::from(rule.dest_port.unwrap_or(0)),
            );
            Ok(Some(tokio::spawn(async move {
                while let Ok((tcp, peer)) = listener.accept().await {
                    let (conn, open, dest) = (Arc::clone(&conn), Arc::clone(&open), dest.clone());
                    tokio::spawn(async move {
                        match kind {
                            ForwardKind::Dynamic => socks(tcp, peer, &conn, open).await,
                            _ => {
                                let ch = conn
                                    .handle
                                    .channel_open_direct_tcpip(
                                        dest.0,
                                        dest.1,
                                        peer.ip().to_string(),
                                        u32::from(peer.port()),
                                    )
                                    .await;
                                if let Ok(ch) = ch {
                                    pipe(tcp, ch.into_stream(), open).await;
                                }
                            }
                        }
                    });
                }
            })))
        }
    }
}

// ------------------------------------------------------------------ SOCKS5 (CONNECT, no authentication)

/// Reads the SOCKS5 greeting and request, returning where the client wants to go.
async fn socks_request(tcp: &mut TcpStream) -> std::io::Result<(String, u16)> {
    let bad = |m: &'static str| std::io::Error::new(std::io::ErrorKind::InvalidData, m);
    let mut head = [0u8; 2];
    tcp.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(bad("not SOCKS5"));
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    tcp.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        tcp.write_all(&[5, 0xFF]).await?; // no acceptable method
        return Err(bad("no anonymous method"));
    }
    tcp.write_all(&[5, 0]).await?;
    let mut req = [0u8; 4];
    tcp.read_exact(&mut req).await?;
    if req[1] != 1 {
        tcp.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?; // command not supported
        return Err(bad("only CONNECT"));
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            tcp.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            tcp.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            tcp.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| bad("bad host name"))?
        }
        4 => {
            let mut a = [0u8; 16];
            tcp.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return Err(bad("unknown address type")),
    };
    let mut port = [0u8; 2];
    tcp.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

async fn socks(mut tcp: TcpStream, peer: SocketAddr, conn: &Connected, open: Arc<AtomicU32>) {
    let Ok((host, port)) = socks_request(&mut tcp).await else {
        return;
    };
    match conn
        .handle
        .channel_open_direct_tcpip(
            host,
            u32::from(port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await
    {
        Ok(ch) => {
            if tcp.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.is_ok() {
                pipe(tcp, ch.into_stream(), open).await;
            }
        }
        Err(_) => {
            let _ = tcp.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await; // connection refused
        }
    }
}
