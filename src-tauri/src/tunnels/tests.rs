//! Port forwarding against the in-process SSH server: local, SOCKS5, remote, a busy port, the
//! reconnect loop, and refusing questions when nobody can answer them (autostart).

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::*;
use crate::ssh::tests::{harness, Harness};

/// A TCP server that sends back whatever it receives.
async fn echo_server() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

async fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn rule(h: &Harness, kind: ForwardKind, bind_port: u16, dest_port: Option<u16>) -> ForwardRule {
    ForwardRule {
        id: format!("rule-{bind_port}"),
        host_id: h.host_id.clone(),
        label: "test-rule".into(),
        kind,
        bind_address: "127.0.0.1".into(),
        bind_port,
        dest_address: dest_port.map(|_| "127.0.0.1".into()),
        dest_port,
        autostart: false,
    }
}

struct Live {
    mgr: Arc<TunnelManager>,
    statuses: mpsc::UnboundedReceiver<TunnelStatus>,
    id: String,
}

/// Starts a rule like the UI would: host-key questions are answered "trust" as they arrive.
fn begin(h: &Harness, rule: ForwardRule, interactive: bool) -> Live {
    let mgr = Arc::new(TunnelManager::default());
    let (stx, statuses) = mpsc::unbounded_channel();
    let emit: Emit = Arc::new(move |s| {
        let _ = stx.send(s);
    });
    let id = rule.id.clone();
    let user: Option<UserEvents> = interactive.then(|| {
        let (m, id) = (Arc::clone(&mgr), id.clone());
        Box::new(move |e: SessionEvent| {
            if let SessionEvent::Prompt { prompt_id, .. } = e {
                let _ = m.respond(
                    &id,
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: false,
                    },
                );
            }
        }) as UserEvents
    });
    mgr.start(h.state.store(), rule, user, emit).unwrap();
    Live { mgr, statuses, id }
}

impl Live {
    async fn until(&mut self, state: TunnelState) -> TunnelStatus {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let s = self.statuses.recv().await.expect("status channel closed");
                if s.state == state {
                    return s;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {state:?}"))
    }
}

async fn echo_roundtrip(addr: (&str, u16)) {
    let mut s = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    s.write_all(b"ping-through-the-tunnel").await.unwrap();
    let mut buf = vec![0u8; 23];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"ping-through-the-tunnel");
}

#[tokio::test]
async fn local_forward_reaches_the_destination_through_the_server() {
    let h = harness(Some("right-pw")).await;
    let (echo, bind) = (echo_server().await, free_port().await);
    let mut live = begin(&h, rule(&h, ForwardKind::Local, bind, Some(echo)), true);
    live.until(TunnelState::Active).await;
    echo_roundtrip(("127.0.0.1", bind)).await;
    assert_eq!(
        h.server.forwards.lock().unwrap().len(),
        1,
        "the connection went through the SSH server"
    );
    live.mgr.stop(&live.id);
    live.until(TunnelState::Stopped).await;
    // Once stopped, nothing listens any more.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(TcpStream::connect(("127.0.0.1", bind)).await.is_err());
}

#[tokio::test]
async fn socks5_proxy_connects_wherever_the_client_asks() {
    let h = harness(Some("right-pw")).await;
    let (echo, bind) = (echo_server().await, free_port().await);
    let mut live = begin(&h, rule(&h, ForwardKind::Dynamic, bind, None), true);
    live.until(TunnelState::Active).await;
    let mut s = TcpStream::connect(("127.0.0.1", bind)).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap(); // greeting: one method, no authentication
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [5, 0]);
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend(echo.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut ok = [0u8; 10];
    s.read_exact(&mut ok).await.unwrap();
    assert_eq!(ok[..2], [5, 0], "CONNECT succeeded");
    s.write_all(b"via-socks").await.unwrap();
    let mut buf = [0u8; 9];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"via-socks");
    live.mgr.stop_all();
}

#[tokio::test]
async fn remote_forward_brings_a_server_port_back_to_this_computer() {
    let h = harness(Some("right-pw")).await;
    let (echo, remote) = (echo_server().await, free_port().await);
    let mut live = begin(&h, rule(&h, ForwardKind::Remote, remote, Some(echo)), true);
    live.until(TunnelState::Active).await;
    // The "server" listens on `remote`; connecting there ends up at our local echo server.
    echo_roundtrip(("127.0.0.1", remote)).await;
    live.mgr.stop_all();
}

#[tokio::test]
async fn a_busy_local_port_is_reported_clearly_and_not_retried() {
    let h = harness(Some("right-pw")).await;
    let taken = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let bind = taken.local_addr().unwrap().port();
    let mut live = begin(&h, rule(&h, ForwardKind::Local, bind, Some(1)), true);
    let s = live.until(TunnelState::Error).await;
    let msg = s.error.unwrap();
    assert!(
        msg.contains(&format!("Port {bind} is already in use on this computer")),
        "{msg}"
    );
    drop(taken);
}

#[tokio::test]
async fn an_unreachable_server_is_retried_with_backoff_until_stopped() {
    let h = harness(Some("right-pw")).await;
    // Point the host at a port nothing listens on.
    let hid = h.host_id.clone();
    let dead = free_port().await;
    h.state
        .store()
        .with_db_session(move |c, s| {
            let mut i = crate::ssh::tests::host_input("test-box", dead, None);
            i.id = Some(hid);
            i.password = None;
            crate::hosts::save(c, s, &i)
        })
        .await
        .unwrap();
    let mut live = begin(
        &h,
        rule(&h, ForwardKind::Local, free_port().await, Some(1)),
        true,
    );
    let s = live.until(TunnelState::Reconnecting).await;
    assert!(s.error.unwrap().contains("Can't reach"));
    assert!(live.mgr.is_running(&live.id), "still trying");
    live.mgr.stop(&live.id);
    live.until(TunnelState::Stopped).await;
    assert!(!live.mgr.is_running(&live.id));
}

#[tokio::test]
async fn autostart_never_answers_host_key_questions_for_you() {
    let h = harness(Some("right-pw")).await; // the server's key is not trusted yet
    let mut live = begin(
        &h,
        rule(&h, ForwardKind::Local, free_port().await, Some(1)),
        false,
    );
    let s = live.until(TunnelState::Error).await;
    assert!(s.error.unwrap().contains("needs an answer from you"));
}

#[tokio::test]
async fn rules_are_validated_and_local_ports_cannot_collide() {
    let h = harness(Some("right-pw")).await;
    let hid = h.host_id.clone();
    let result = h
        .state
        .store()
        .with_db_session(move |c, s| {
            let v = &s.vault_id;
            let mk = |label: &str, kind: ForwardKind, port: u16, dest: Option<(&str, u16)>| {
                ForwardInput {
                    id: None,
                    host_id: hid.clone(),
                    label: label.into(),
                    kind,
                    bind_address: "127.0.0.1".into(),
                    bind_port: port,
                    dest_address: dest.map(|d| d.0.into()),
                    dest_port: dest.map(|d| d.1),
                    autostart: false,
                }
            };
            save_rule(
                c,
                v,
                &mk("db", ForwardKind::Local, 5432, Some(("127.0.0.1", 5432))),
            )?;
            assert!(
                save_rule(c, v, &mk("dup", ForwardKind::Dynamic, 5432, None)).is_err(),
                "same local address and port"
            );
            save_rule(
                c,
                v,
                &mk(
                    "remote-ok",
                    ForwardKind::Remote,
                    5432,
                    Some(("127.0.0.1", 3000)),
                ),
            )?; // remote ports live on the server
            assert!(save_rule(c, v, &mk(" ", ForwardKind::Local, 1, Some(("a", 1)))).is_err());
            assert!(save_rule(c, v, &mk("x", ForwardKind::Local, 0, Some(("a", 1)))).is_err());
            assert!(
                save_rule(c, v, &mk("x", ForwardKind::Local, 2000, None)).is_err(),
                "local needs a destination"
            );
            let mut bad = mk("x", ForwardKind::Local, 2001, Some(("a", 1)));
            bad.bind_address = "not an ip".into();
            assert!(save_rule(c, v, &bad).is_err());
            // SOCKS drops any destination that was sent along.
            let socks = save_rule(
                c,
                v,
                &mk("socks", ForwardKind::Dynamic, 1080, Some(("ignored", 1))),
            )?;
            assert_eq!(get_rule(c, v, &socks)?.dest_port, None);
            Ok(list_rules(c, v)?.len())
        })
        .await
        .unwrap();
    assert_eq!(result, 3);
}

/// Against real OpenSSH (Docker server with `AllowTcpForwarding yes`): a local forward to the
/// container's own SSH port returns an SSH banner, and a SOCKS5 CONNECT to it does too.
#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn docker_local_and_socks_forwards_reach_the_container() {
    let h = harness(None).await;
    let hid = h.host_id.clone();
    h.state
        .store()
        .with_db_session(move |c, s| {
            let mut i =
                crate::ssh::tests::host_input("docker", crate::testservers::port("a"), None);
            i.id = Some(hid);
            i.password = Some("test".into());
            crate::hosts::save(c, s, &i)
        })
        .await
        .unwrap();
    let banner = |mut s: TcpStream| async move {
        let mut buf = [0u8; 8];
        tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..4], b"SSH-");
    };
    let bind = free_port().await;
    let mut live = begin(&h, rule(&h, ForwardKind::Local, bind, Some(2222)), true);
    live.until(TunnelState::Active).await;
    banner(TcpStream::connect(("127.0.0.1", bind)).await.unwrap()).await;
    live.mgr.stop_all();

    let bind = free_port().await;
    let mut r = rule(&h, ForwardKind::Dynamic, bind, None);
    r.id = "socks".into();
    let mut live = begin(&h, r, true);
    live.until(TunnelState::Active).await;
    let mut s = TcpStream::connect(("127.0.0.1", bind)).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut g = [0u8; 2];
    s.read_exact(&mut g).await.unwrap();
    s.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0x08, 0xAE])
        .await
        .unwrap(); // 127.0.0.1:2222
    let mut ok = [0u8; 10];
    s.read_exact(&mut ok).await.unwrap();
    assert_eq!(ok[1], 0);
    banner(s).await;
    live.mgr.stop_all();
}
