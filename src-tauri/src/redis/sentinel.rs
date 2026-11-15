//! Redis Sentinel (F19). The database's address is a list of sentinels. Each is asked, through the tab's route,
//! where the primary is; the tab then connects to that address and checks it really is the primary. After a drop
//! the question is asked again, because the primary may have moved.

use redis::aio::MultiplexedConnection;
use redis::Value;

use super::cluster::{join, RedisNode};
use super::conn::Opener;
use crate::error::{AppError, AppResult, ErrorCode};

/// The sentinel port when an entry names none.
pub const DEFAULT_PORT: u16 = 26379;

/// `10.0.2.21:26379, 10.0.2.22, [::1]:26380` as (host, port) pairs. An entry without a port uses `default_port`.
pub fn parse_list(address: &str, default_port: u16) -> Vec<(String, u16)> {
    address
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|entry| {
            // An IPv6 address in brackets may carry a port after them; a bare one has colons but no port.
            if let Some(rest) = entry.strip_prefix('[') {
                let (host, tail) = rest.split_once(']').unwrap_or((rest, ""));
                let port = tail
                    .strip_prefix(':')
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(default_port);
                return (host.to_string(), port);
            }
            match entry.rsplit_once(':') {
                Some((host, port)) if !host.contains(':') => match port.parse() {
                    Ok(p) => (host.to_string(), p),
                    Err(_) => (entry.to_string(), default_port),
                },
                _ => (entry.to_string(), default_port),
            }
        })
        .collect()
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(b) => String::from_utf8(b.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// The answer to `SENTINEL get-master-addr-by-name`: `[host, port]`, or nil when the sentinel doesn't know the name.
pub fn primary_address(reply: &Value) -> Option<(String, u16)> {
    let Value::Array(parts) = reply else {
        return None;
    };
    let host = text(parts.first()?)?;
    let port = match parts.get(1)? {
        Value::Int(i) => u16::try_from(*i).ok()?,
        other => text(other)?.parse().ok()?,
    };
    (!host.is_empty()).then_some((host, port))
}

/// The first sentinel that answers, with its connection. Each one that doesn't is remembered for the message.
async fn first_answering(
    opener: &Opener,
    sentinels: &[(String, u16)],
) -> AppResult<(MultiplexedConnection, String)> {
    let mut why = Vec::new();
    for (host, port) in sentinels {
        match opener.open_plain(host, *port).await {
            Ok(conn) => return Ok((conn, join(host, *port))),
            Err(e) => why.push(format!("{}: {}", join(host, *port), e.message)),
        }
    }
    Err(AppError::new(
        ErrorCode::RedisUnreachable,
        if why.is_empty() {
            "List at least one sentinel in the address.".to_string()
        } else {
            format!("No sentinel answered. {}", why.join(" "))
        },
    ))
}

/// Asks the sentinels, in the order listed, where the primary called `master` is.
pub async fn find_primary(
    opener: &Opener,
    sentinels: &[(String, u16)],
    master: &str,
) -> AppResult<(String, u16)> {
    let mut unknown = false;
    let mut why = Vec::new();
    for (host, port) in sentinels {
        let mut conn = match opener.open_plain(host, *port).await {
            Ok(c) => c,
            Err(e) => {
                why.push(format!("{}: {}", join(host, *port), e.message));
                continue;
            }
        };
        let reply = redis::cmd("SENTINEL")
            .arg("get-master-addr-by-name")
            .arg(master)
            .query_async::<Value>(&mut conn)
            .await;
        match reply {
            Ok(v) => match primary_address(&v) {
                Some(found) => return Ok(found),
                None => unknown = true,
            },
            Err(e) => why.push(format!("{}: {e}", join(host, *port))),
        }
    }
    Err(if unknown {
        AppError::new(
            ErrorCode::RedisRefused,
            format!("The sentinels don't know a primary called {master}. Check the name in the database's settings."),
        )
    } else if why.is_empty() {
        AppError::validation("List at least one sentinel in the address.")
    } else {
        AppError::new(
            ErrorCode::RedisUnreachable,
            format!("No sentinel answered. {}", why.join(" ")),
        )
    })
}

/// A RESP2 flat list or a RESP3 map as (name, value) text pairs.
fn pairs(v: &Value) -> Vec<(String, String)> {
    match v {
        Value::Map(m) => m
            .iter()
            .filter_map(|(k, val)| Some((text(k)?, text(val)?)))
            .collect(),
        Value::Array(items) => items
            .chunks(2)
            .filter_map(|c| Some((text(c.first()?)?, text(c.get(1)?)?)))
            .collect(),
        _ => Vec::new(),
    }
}

/// One row of the Nodes panel from an entry of `SENTINEL replicas` or `SENTINEL sentinels`.
pub fn node_row(entry: &Value, role: &str, primary: &str) -> Option<RedisNode> {
    let fields = pairs(entry);
    let get = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let address = join(get("ip")?, get("port")?.parse().ok()?);
    let flags = get("flags").unwrap_or_default();
    let state =
        if flags.contains("o_down") || flags.contains("s_down") || flags.contains("disconnected") {
            "down"
        } else if role == "replica" && get("master-link-status").is_some_and(|s| s != "ok") {
            "catching up"
        } else {
            "ok"
        };
    Some(RedisNode {
        role: role.to_string(),
        address,
        detail: if role == "replica" {
            format!("of {primary}")
        } else {
            "watches the primary".to_string()
        },
        state: state.to_string(),
    })
}

/// The Nodes panel for a Sentinel-managed database: the primary, its replicas, the sentinels.
pub async fn nodes(
    opener: &Opener,
    sentinels: &[(String, u16)],
    master: &str,
) -> AppResult<Vec<RedisNode>> {
    let (mut conn, asked) = first_answering(opener, sentinels).await?;
    let list = |v: Value| match v {
        Value::Array(items) => items,
        _ => Vec::new(),
    };
    let primary = redis::cmd("SENTINEL")
        .arg("get-master-addr-by-name")
        .arg(master)
        .query_async::<Value>(&mut conn)
        .await
        .ok()
        .and_then(|v| primary_address(&v))
        .map(|(h, p)| join(&h, p))
        .unwrap_or_default();
    let mut out = vec![RedisNode {
        role: "primary".into(),
        address: primary.clone(),
        detail: master.to_string(),
        state: "ok".into(),
    }];
    // `replicas` is the word since Redis 5; older sentinels only know `slaves`.
    let replicas = match redis::cmd("SENTINEL")
        .arg("replicas")
        .arg(master)
        .query_async::<Value>(&mut conn)
        .await
    {
        Ok(v) => list(v),
        Err(_) => redis::cmd("SENTINEL")
            .arg("slaves")
            .arg(master)
            .query_async::<Value>(&mut conn)
            .await
            .map(list)
            .unwrap_or_default(),
    };
    out.extend(
        replicas
            .iter()
            .filter_map(|r| node_row(r, "replica", &primary)),
    );
    // The sentinel that answered does not list itself.
    out.push(RedisNode {
        role: "sentinel".into(),
        address: asked,
        detail: "watches the primary".into(),
        state: "ok".into(),
    });
    let others = redis::cmd("SENTINEL")
        .arg("sentinels")
        .arg(master)
        .query_async::<Value>(&mut conn)
        .await
        .map(list)
        .unwrap_or_default();
    out.extend(
        others
            .iter()
            .filter_map(|s| node_row(s, "sentinel", &primary)),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }

    #[test]
    fn the_sentinel_list_is_read_with_and_without_ports() {
        let p = |s: &str| parse_list(s, DEFAULT_PORT);
        assert_eq!(
            p("10.0.2.21:26379, 10.0.2.22:26380,10.0.2.23"),
            [
                ("10.0.2.21".to_string(), 26379),
                ("10.0.2.22".to_string(), 26380),
                ("10.0.2.23".to_string(), 26379)
            ]
        );
        assert_eq!(
            p("sentinel-a.internal"),
            [("sentinel-a.internal".to_string(), 26379)]
        );
        assert_eq!(
            p("[::1]:26380, [fd00::2], fd00::3"),
            [
                ("::1".to_string(), 26380),
                ("fd00::2".to_string(), 26379),
                ("fd00::3".to_string(), 26379)
            ]
        );
        assert_eq!(
            parse_list("a, b", 5000),
            [("a".to_string(), 5000), ("b".to_string(), 5000)]
        );
        assert!(p("").is_empty());
        assert!(p(" , ,").is_empty());
    }

    #[test]
    fn the_primarys_address_is_read_and_an_unknown_name_is_none() {
        assert_eq!(
            primary_address(&Value::Array(vec![b("10.0.2.11"), b("6379")])),
            Some(("10.0.2.11".to_string(), 6379))
        );
        assert_eq!(
            primary_address(&Value::Array(vec![b("10.0.2.11"), Value::Int(6380)])),
            Some(("10.0.2.11".to_string(), 6380))
        );
        for nothing in [
            Value::Nil,
            Value::Array(vec![]),
            Value::Array(vec![b("10.0.2.11")]),
            Value::Array(vec![b(""), b("6379")]),
            Value::Array(vec![b("h"), b("port")]),
        ] {
            assert_eq!(primary_address(&nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn replicas_and_sentinels_become_rows_with_a_state() {
        let entry = |pairs: &[(&str, &str)]| {
            Value::Array(pairs.iter().flat_map(|(k, v)| [b(k), b(v)]).collect())
        };
        let ok = node_row(
            &entry(&[
                ("name", "10.0.2.12:6379"),
                ("ip", "10.0.2.12"),
                ("port", "6379"),
                ("flags", "slave"),
                ("master-link-status", "ok"),
            ]),
            "replica",
            "10.0.2.11:6379",
        )
        .unwrap();
        assert_eq!(
            (
                ok.role.as_str(),
                ok.address.as_str(),
                ok.detail.as_str(),
                ok.state.as_str()
            ),
            ("replica", "10.0.2.12:6379", "of 10.0.2.11:6379", "ok")
        );
        let syncing = node_row(
            &entry(&[
                ("ip", "10.0.2.13"),
                ("port", "6379"),
                ("flags", "slave"),
                ("master-link-status", "err"),
            ]),
            "replica",
            "p",
        )
        .unwrap();
        assert_eq!(syncing.state, "catching up");
        let down = node_row(
            &entry(&[
                ("ip", "10.0.2.14"),
                ("port", "6379"),
                ("flags", "slave,s_down,disconnected"),
            ]),
            "replica",
            "p",
        )
        .unwrap();
        assert_eq!(down.state, "down");
        let sentinel = node_row(
            &Value::Map(vec![
                (b("ip"), b("10.0.2.22")),
                (b("port"), b("26379")),
                (b("flags"), b("sentinel")),
            ]),
            "sentinel",
            "p",
        )
        .unwrap();
        assert_eq!(
            (
                sentinel.role.as_str(),
                sentinel.address.as_str(),
                sentinel.detail.as_str(),
                sentinel.state.as_str()
            ),
            ("sentinel", "10.0.2.22:26379", "watches the primary", "ok")
        );
        assert_eq!(
            node_row(&entry(&[("ip", "10.0.2.14")]), "replica", "p"),
            None,
            "an entry without a port is skipped"
        );
    }
}
