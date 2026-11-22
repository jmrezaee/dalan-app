//! Connection strings (F20, S35 "paste a connection string"): `postgres://`, `mysql://`, `mariadb://`,
//! `mongodb://`, `mongodb+srv://`, `redis://`, `rediss://`. Parsed by hand because MongoDB's can list several
//! members, which a URL parser would refuse. The password is read here and goes to the vault with the save; the
//! command that answers the window leaves it out (`DbUrlFields` has no password, only `has_password`).

use serde::Serialize;
use specta::Type;
use zeroize::Zeroizing;

use super::store::DbOptions;
use super::Engine;
use crate::error::{AppError, AppResult};

/// Everything a connection string says. Keeps the password; never serialised.
pub struct ParsedUrl {
    pub engine: Engine,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub default_database: Option<String>,
    pub tls: String,
    pub options: DbOptions,
}

/// What the window gets back: the fields to fill in, and whether a password was in the string.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DbUrlFields {
    pub engine: Engine,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub has_password: bool,
    pub default_database: Option<String>,
    pub tls: String,
    pub options: DbOptions,
}

impl From<ParsedUrl> for DbUrlFields {
    fn from(p: ParsedUrl) -> Self {
        DbUrlFields {
            engine: p.engine,
            address: p.address,
            port: p.port,
            username: p.username,
            has_password: p.password.as_deref().is_some_and(|x| !x.is_empty()),
            default_database: p.default_database,
            tls: p.tls,
            options: p.options,
        }
    }
}

fn bad(msg: &str) -> AppError {
    AppError::validation(msg)
}

fn decode(s: &str) -> AppResult<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| bad("The connection string has a broken % escape."))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| bad("The connection string is not valid text."))
}

fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.to_ascii_lowercase(), decode(v).unwrap_or_default()),
            None => (p.to_ascii_lowercase(), String::new()),
        })
        .collect()
}

pub fn parse(url: &str) -> AppResult<ParsedUrl> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| bad("That isn't a connection string. It starts with postgres://, mysql://, mariadb://, mongodb:// or redis://."))?;
    let scheme = scheme.to_ascii_lowercase();
    let (engine, srv, scheme_tls) = match scheme.as_str() {
        "postgres" | "postgresql" => (Engine::Postgres, false, false),
        "mysql" => (Engine::Mysql, false, false),
        "mariadb" => (Engine::Mariadb, false, false),
        "mongodb" => (Engine::Mongodb, false, false),
        "mongodb+srv" => (Engine::Mongodb, true, true),
        "redis" => (Engine::Redis, false, false),
        "rediss" => (Engine::Redis, false, true),
        _ => {
            return Err(bad(
                "This connection string's scheme isn't one Dalan opens.",
            ))
        }
    };
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    // The last @ splits the user info from the hosts: a password may hold an escaped @, but never a bare one.
    let (userinfo, hosts) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (username, password) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((n, p)) => (Some(decode(n)?), Some(decode(p)?)),
            None => (Some(decode(u)?), None),
        },
        None => (None, None),
    };
    // Held in zeroizing memory while we work; the caller decides where the copy goes.
    let password = password.map(Zeroizing::new);
    if hosts.is_empty() {
        return Err(bad("The connection string has no host."));
    }
    let q = query_pairs(query);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());

    let mut options = DbOptions::default();
    let (address, port) = if engine == Engine::Mongodb {
        options.srv = srv;
        options.auth_source = get("authsource").map(str::to_string);
        options.replica_set = get("replicaset").map(str::to_string);
        // A seed list keeps its ports in the address; they are the members' own.
        (hosts.to_string(), None)
    } else {
        split_host_port(hosts, engine)?
    };

    let default_database = match decode(path)? {
        d if d.is_empty() => None,
        d => Some(d),
    };
    let flag = |k: &str| matches!(get(k), Some("true" | "1" | "yes" | ""));
    let tls = match engine {
        Engine::Postgres => match get("sslmode") {
            Some("require") => "require",
            Some("verify-ca" | "verify-full") => "verify",
            _ => "off",
        },
        Engine::Mysql | Engine::Mariadb => {
            match get("ssl-mode").map(str::to_ascii_lowercase).as_deref() {
                Some("required" | "preferred") => "require",
                Some("verify_ca" | "verify_identity") => "verify",
                _ if flag("tls") || flag("ssl") => "verify",
                _ => "off",
            }
        }
        Engine::Mongodb => {
            if get("tlsallowinvalidcertificates") == Some("true") {
                "require"
            } else if scheme_tls || flag("tls") || flag("ssl") {
                "verify"
            } else {
                "off"
            }
        }
        Engine::Redis => {
            if scheme_tls {
                "verify"
            } else {
                "off"
            }
        }
    }
    .to_string();
    Ok(ParsedUrl {
        engine,
        address,
        port,
        username: username.filter(|u| !u.is_empty()),
        password: password.map(|p| (*p).clone()),
        default_database,
        tls,
        options,
    })
}

fn split_host_port(hosts: &str, engine: Engine) -> AppResult<(String, Option<u16>)> {
    if hosts.contains(',') {
        return Err(bad("Only MongoDB connection strings list several hosts."));
    }
    // [::1]:5432
    if let Some(rest) = hosts.strip_prefix('[') {
        let (addr, after) = rest
            .split_once(']')
            .ok_or_else(|| bad("The IPv6 address is missing its closing ]."))?;
        let port = after.strip_prefix(':').map(parse_port).transpose()?;
        return Ok((
            addr.to_string(),
            Some(port.unwrap_or(engine.default_port())),
        ));
    }
    match hosts.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), Some(parse_port(p)?))),
        None => Ok((hosts.to_string(), Some(engine.default_port()))),
    }
}

fn parse_port(p: &str) -> AppResult<u16> {
    p.parse::<u16>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| bad("The port in the connection string must be between 1 and 65535."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_with_escapes_and_tls() {
        let p = parse("postgres://app:p%40ss%3Aword@db.internal:6432/shop?sslmode=verify-full")
            .unwrap();
        assert_eq!(p.engine, Engine::Postgres);
        assert_eq!((p.address.as_str(), p.port), ("db.internal", Some(6432)));
        assert_eq!(p.username.as_deref(), Some("app"));
        assert_eq!(p.password.as_deref(), Some("p@ss:word"));
        assert_eq!(p.default_database.as_deref(), Some("shop"));
        assert_eq!(p.tls, "verify");
    }

    #[test]
    fn defaults_ports_and_ipv6() {
        let p = parse("mysql://root@[::1]/").unwrap();
        assert_eq!((p.address.as_str(), p.port), ("::1", Some(3306)));
        assert!(p.password.is_none() && p.default_database.is_none());
        assert_eq!(parse("mariadb://u:p@h").unwrap().engine, Engine::Mariadb);
        assert_eq!(parse("postgresql://h").unwrap().port, Some(5432));
        assert_eq!(parse("postgres://h/db?sslmode=disable").unwrap().tls, "off");
        assert_eq!(parse("mysql://h?ssl-mode=REQUIRED").unwrap().tls, "require");
    }

    #[test]
    fn mongodb_seed_lists_srv_and_options() {
        let p =
            parse("mongodb://u:p@h1:27017,h2:27018/shop?replicaSet=rs0&authSource=admin&tls=true")
                .unwrap();
        assert_eq!(p.address, "h1:27017,h2:27018");
        assert_eq!(p.port, None);
        assert_eq!(p.options.replica_set.as_deref(), Some("rs0"));
        assert_eq!(p.options.auth_source.as_deref(), Some("admin"));
        assert_eq!(p.tls, "verify");
        let s = parse("mongodb+srv://cluster0.example.net/app").unwrap();
        assert!(s.options.srv);
        assert_eq!(s.tls, "verify");
        assert_eq!(
            parse("mongodb://h/?tls=true&tlsAllowInvalidCertificates=true")
                .unwrap()
                .tls,
            "require"
        );
    }

    #[test]
    fn redis_strings_and_refusals() {
        let r = parse("rediss://:secret@cache:6380/2").unwrap();
        assert_eq!((r.engine, r.tls.as_str()), (Engine::Redis, "verify"));
        assert_eq!(r.password.as_deref(), Some("secret"));
        assert_eq!(r.default_database.as_deref(), Some("2"));
        assert!(parse("ftp://x").is_err());
        assert!(parse("not a url").is_err());
        assert!(parse("postgres://").is_err());
        assert!(parse("postgres://h:0/db").is_err());
        assert!(parse("postgres://a,b/db").is_err());
        assert!(parse("postgres://u:%zz@h").is_err());
    }

    #[test]
    fn the_answer_for_the_window_has_no_password() {
        let f: DbUrlFields = parse("postgres://u:topsecret@h/db").unwrap().into();
        assert!(f.has_password);
        assert!(!serde_json::to_string(&f).unwrap().contains("topsecret"));
    }
}
