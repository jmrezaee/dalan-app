//! SOCKS5 and HTTP proxy for the first connection of a host (S03, S04). With a jump chain, the
//! proxy carries the connection to the first hop; later hops ride SSH channels as before.
//!
//! No proxy sign-in yet: only proxies that accept anyone (typical inside a company network).
//! Stored per host or group in the profile (`proxy_type` NULL = inherit, `none` = direct on purpose).

use std::io;
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::groups;

/// What the editors show and save. `kind`: `inherit`, `none`, `socks5` or `http`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct ProxyChoice {
    pub kind: String,
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Serialize, Type)]
pub struct ProxyConfig {
    pub own: ProxyChoice,
    /// What the groups above give: a one-line summary like `socks5 proxy.corp:1080`, and the group.
    pub inherited: Option<String>,
    pub inherited_from: Option<String>,
}

/// A proxy ready to use.
#[derive(Debug, Clone, PartialEq)]
pub struct Proxy {
    pub socks: bool,
    pub host: String,
    pub port: u16,
}

fn summary(p: &Proxy) -> String {
    format!(
        "{} {}:{}",
        if p.socks { "SOCKS5" } else { "HTTP" },
        p.host,
        p.port
    )
}

/// Per level: (label, kind, host, port). The nearest level that sets a kind decides.
fn load(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
) -> AppResult<Vec<(Option<String>, ProxyChoice)>> {
    groups::profile_chain(conn, vault_id, kind, id)?
        .into_iter()
        .map(|(label, pid)| {
            let (k, h, p): (Option<String>, Option<String>, Option<u16>) = conn.query_row(
                "SELECT proxy_type, proxy_host, proxy_port FROM connection_profiles WHERE id = ?1",
                [pid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            Ok((
                label,
                ProxyChoice {
                    kind: k.unwrap_or_else(|| "inherit".into()),
                    host: h.unwrap_or_default(),
                    port: p,
                },
            ))
        })
        .collect()
}

fn decide(levels: &[(Option<String>, ProxyChoice)]) -> Option<(Option<Proxy>, Option<String>)> {
    levels
        .iter()
        .find(|(_, c)| c.kind != "inherit")
        .map(|(label, c)| {
            let proxy = match (c.kind.as_str(), c.port) {
                ("socks5" | "http", Some(port)) if !c.host.is_empty() => Some(Proxy {
                    socks: c.kind == "socks5",
                    host: c.host.clone(),
                    port,
                }),
                _ => None, // `none`
            };
            (proxy, label.clone())
        })
}

pub fn get(conn: &Connection, vault_id: &str, kind: &str, id: &str) -> AppResult<ProxyConfig> {
    let lv = load(conn, vault_id, kind, id)?;
    let (inherited, inherited_from) = match decide(&lv[1..]) {
        Some((Some(p), from)) => (Some(summary(&p)), from),
        Some((None, from)) => (Some("No proxy".into()), from),
        None => (None, None),
    };
    Ok(ProxyConfig {
        own: lv[0].1.clone(),
        inherited,
        inherited_from,
    })
}

pub fn set(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
    c: &ProxyChoice,
) -> AppResult<()> {
    let host = c.host.trim();
    let (t, h, p) = match c.kind.as_str() {
        "inherit" => (None, None, None),
        "none" => (Some("none"), None, None),
        k @ ("socks5" | "http") => {
            if host.is_empty() || host.chars().any(char::is_whitespace) {
                return Err(AppError::validation("Enter the proxy's address."));
            }
            match c.port {
                Some(port) if port > 0 => (Some(k), Some(host.to_string()), Some(port)),
                _ => return Err(AppError::validation("Port must be between 1 and 65535.")),
            }
        }
        _ => return Err(AppError::validation("Unknown proxy type.")),
    };
    let table = if kind == "group" { "groups" } else { "hosts" };
    let n = conn.execute(
        &format!("UPDATE connection_profiles SET proxy_type = ?1, proxy_host = ?2, proxy_port = ?3, updated_at = ?4 WHERE id = (SELECT profile_id FROM {table} WHERE id = ?5 AND vault_id = ?6)"),
        params![t, h, p, now_ms(), id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That item no longer exists."));
    }
    Ok(())
}

/// The proxy a host's first connection uses, after inheritance.
pub fn resolve_for_host(
    conn: &Connection,
    vault_id: &str,
    host_id: &str,
) -> AppResult<Option<Proxy>> {
    Ok(decide(&load(conn, vault_id, "host", host_id)?).and_then(|(p, _)| p))
}

const STEP_TIMEOUT: Duration = Duration::from_secs(10);

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// Opens a TCP connection to `dest` through the proxy.
pub async fn connect(proxy: &Proxy, dest_host: &str, dest_port: u16) -> io::Result<TcpStream> {
    let mut s = TcpStream::connect((proxy.host.as_str(), proxy.port)).await?;
    tokio::time::timeout(STEP_TIMEOUT, async {
        if proxy.socks {
            socks5(&mut s, dest_host, dest_port).await
        } else {
            http_connect(&mut s, dest_host, dest_port).await
        }
    })
    .await
    .map_err(|_| bad("the proxy didn't answer"))??;
    Ok(s)
}

async fn socks5(s: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
    s.write_all(&[5, 1, 0]).await?; // version 5, one method: no authentication
    let mut ans = [0u8; 2];
    s.read_exact(&mut ans).await?;
    if ans != [5, 0] {
        return Err(bad(
            "the proxy needs a sign-in, which Dalan doesn't support yet",
        ));
    }
    let name = host.as_bytes();
    if name.is_empty() || name.len() > 255 {
        return Err(bad("the address is too long for SOCKS5"));
    }
    // CONNECT, address type 3 = domain name (the proxy resolves it).
    let mut req = vec![5, 1, 0, 3, name.len() as u8];
    req.extend_from_slice(name);
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0 {
        return Err(bad(match head[1] {
            2 => "the proxy's rules don't allow that connection".to_string(),
            3 | 4 => "the proxy can't reach that network or host".to_string(),
            5 => "the server refused the connection".to_string(),
            c => format!("the proxy refused (code {c})"),
        }));
    }
    // Skip the bound address the proxy reports.
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(s.read_u8().await?),
        _ => return Err(bad("the proxy sent something unexpected")),
    } + 2;
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest).await?;
    Ok(())
}

async fn http_connect(s: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await?;
    // Read the answer one byte at a time up to the blank line, so no SSH bytes are swallowed.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 {
            return Err(bad("the proxy's answer is too long"));
        }
        head.push(s.read_u8().await?);
    }
    let line = String::from_utf8_lossy(&head);
    let code = line.split_whitespace().nth(1).unwrap_or("");
    match code {
        "200" => Ok(()),
        "407" => Err(bad(
            "the proxy needs a sign-in, which Dalan doesn't support yet",
        )),
        c => Err(bad(format!("the proxy refused (HTTP {c})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};
    use tokio::net::TcpListener;

    /// An echo server, and a proxy of the requested kind that forwards to whatever it is asked for.
    async fn echo() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut b = [0u8; 64];
                    while let Ok(n) = s.read(&mut b).await {
                        if n == 0 || s.write_all(&b[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        port
    }

    async fn fake_proxy(socks: bool) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                tokio::spawn(async move {
                    let (host, dport) = if socks {
                        let mut g = [0u8; 3];
                        c.read_exact(&mut g).await.unwrap();
                        c.write_all(&[5, 0]).await.unwrap();
                        let mut h = [0u8; 5];
                        c.read_exact(&mut h).await.unwrap();
                        let mut name = vec![0u8; usize::from(h[4])];
                        c.read_exact(&mut name).await.unwrap();
                        let dport = c.read_u16().await.unwrap();
                        c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                        (String::from_utf8(name).unwrap(), dport)
                    } else {
                        let mut head = Vec::new();
                        while !head.ends_with(b"\r\n\r\n") {
                            head.push(c.read_u8().await.unwrap());
                        }
                        let target = String::from_utf8_lossy(&head)
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                            .to_string();
                        c.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                            .await
                            .unwrap();
                        let (h, p) = target.rsplit_once(':').unwrap();
                        (h.to_string(), p.parse().unwrap())
                    };
                    let mut up = TcpStream::connect((host.as_str(), dport)).await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn socks5_and_http_connect_reach_the_destination() {
        let dest = echo().await;
        for socks in [true, false] {
            let proxy = Proxy {
                socks,
                host: "127.0.0.1".into(),
                port: fake_proxy(socks).await,
            };
            let mut s = connect(&proxy, "127.0.0.1", dest).await.unwrap();
            s.write_all(b"ping").await.unwrap();
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await.unwrap();
            assert_eq!(&b, b"ping", "socks={socks}");
        }
        // Nothing is listening: a plain error, not a hang.
        let dead = Proxy {
            socks: true,
            host: "127.0.0.1".into(),
            port: 1,
        };
        assert!(connect(&dead, "127.0.0.1", dest).await.is_err());
    }

    #[test]
    fn choice_inherits_from_groups_and_none_overrides() {
        use crate::groups::GroupInput;
        use crate::hosts::{self, HostInput};
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let g = groups::save(
            &mut c,
            &s,
            &GroupInput {
                id: None,
                parent_id: None,
                label: "Corp".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                key_id: None,
                identity_id: None,
                startup_snippet_id: None,
                jump_mode: "inherit".into(),
                jump_host_ids: vec![],
            },
        )
        .unwrap();
        let h = hosts::save(
            &mut c,
            &s,
            &HostInput {
                id: None,
                label: "h".into(),
                address: "10.0.0.1".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                group_id: Some(g.id.clone()),
                key_id: None,
                identity_id: None,
                startup_snippet_id: None,
                jump_mode: "inherit".into(),
                jump_host_ids: vec![],
                tags: vec![],
            },
        )
        .unwrap();
        let v = &s.vault_id;
        let choice = |k: &str, h: &str, p| ProxyChoice {
            kind: k.into(),
            host: h.into(),
            port: p,
        };
        assert_eq!(resolve_for_host(&c, v, &h.id).unwrap(), None);
        set(
            &c,
            v,
            "group",
            &g.id,
            &choice("socks5", "proxy.corp", Some(1080)),
        )
        .unwrap();
        let p = resolve_for_host(&c, v, &h.id).unwrap().unwrap();
        assert_eq!(
            (p.socks, p.host.as_str(), p.port),
            (true, "proxy.corp", 1080)
        );
        let cfg = get(&c, v, "host", &h.id).unwrap();
        assert_eq!(
            (cfg.inherited.as_deref(), cfg.inherited_from.as_deref()),
            (Some("SOCKS5 proxy.corp:1080"), Some("Corp"))
        );
        set(&c, v, "host", &h.id, &choice("none", "", None)).unwrap(); // direct on purpose
        assert_eq!(resolve_for_host(&c, v, &h.id).unwrap(), None);
        set(&c, v, "host", &h.id, &choice("http", "p2", Some(3128))).unwrap();
        assert!(!resolve_for_host(&c, v, &h.id).unwrap().unwrap().socks);
        assert!(set(&c, v, "host", &h.id, &choice("socks5", "", Some(1))).is_err());
        assert!(set(&c, v, "host", &h.id, &choice("socks5", "x", None)).is_err());
        assert!(set(&c, v, "host", &h.id, &choice("ftp", "x", Some(1))).is_err());
    }
}
