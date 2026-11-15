//! Saved Redis databases (S26, S27): the `redis_databases` table. The password lives in `secrets` like every
//! other password; nothing here ever returns it (hard rule 1), only `has_password`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroizing;

use super::gate;
use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::vault::secrets::{self, SecretKind};
use crate::vault::VaultSession;

/// Stored as lowercase text; `production` adds the amber line and chip to the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisEnvironment {
    None,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisRoute {
    Direct,
    /// Sign in to a saved host (with its jump chain) and talk to Redis from there.
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisKind {
    Single,
    Cluster,
    Sentinel,
}

/// What the write gate allows (spec section 4). New databases start as `Confirm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisWriteMode {
    ReadOnly,
    Confirm,
    Open,
}

// The enums are stored as the same snake_case words serde uses. One macro keeps the two directions together.
macro_rules! text_enum {
    ($t:ty { $($v:ident => $s:literal),+ }) => {
        impl $t {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$v => $s),+ }
            }
            fn parse(s: &str) -> rusqlite::Result<Self> {
                match s {
                    $($s => Ok(Self::$v),)+
                    other => Err(rusqlite::Error::InvalidColumnType(0, other.to_string(), rusqlite::types::Type::Text)),
                }
            }
        }
    };
}
text_enum!(RedisEnvironment { None => "none", Staging => "staging", Production => "production" });
text_enum!(RedisRoute { Direct => "direct", Host => "host" });
text_enum!(RedisKind { Single => "single", Cluster => "cluster", Sentinel => "sentinel" });
text_enum!(RedisWriteMode { ReadOnly => "read_only", Confirm => "confirm", Open => "open" });

/// One row of the Databases table, as the UI sees it.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisDatabase {
    pub id: String,
    pub name: String,
    pub environment: RedisEnvironment,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    /// The host's label, for "Through db-primary".
    pub host_label: Option<String>,
    pub address: String,
    pub port: u16,
    pub kind: RedisKind,
    pub sentinel_master: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
    pub tls: bool,
    pub tls_server_name: Option<String>,
    pub write_mode: RedisWriteMode,
    pub default_db: u32,
    pub key_separator: String,
    #[specta(type = Option<f64>)]
    pub last_opened_at: Option<i64>,
}

/// The editor's form (S27). `password: Some` sets a new one; `clear_password` removes the saved one.
#[derive(Debug, Deserialize, Type)]
pub struct RedisDatabaseInput {
    /// `None` creates a database.
    pub id: Option<String>,
    pub name: String,
    pub environment: RedisEnvironment,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    pub address: String,
    pub port: u16,
    pub kind: RedisKind,
    pub sentinel_master: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub clear_password: bool,
    pub tls: bool,
    pub tls_server_name: Option<String>,
    pub write_mode: RedisWriteMode,
    pub default_db: u32,
    pub key_separator: String,
}

const COLUMNS: &str = "d.id, d.name, d.environment, d.route, d.host_id, h.label, d.address, d.port, d.kind, d.sentinel_master, d.username, d.password_secret_id IS NOT NULL, d.tls, d.tls_server_name, d.write_mode, d.default_db, d.key_separator, d.last_opened_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RedisDatabase> {
    Ok(RedisDatabase {
        id: r.get(0)?,
        name: r.get(1)?,
        environment: RedisEnvironment::parse(&r.get::<_, String>(2)?)?,
        route: RedisRoute::parse(&r.get::<_, String>(3)?)?,
        host_id: r.get(4)?,
        host_label: r.get(5)?,
        address: r.get(6)?,
        port: r.get(7)?,
        kind: RedisKind::parse(&r.get::<_, String>(8)?)?,
        sentinel_master: r.get(9)?,
        username: r.get(10)?,
        has_password: r.get(11)?,
        tls: r.get(12)?,
        tls_server_name: r.get(13)?,
        write_mode: RedisWriteMode::parse(&r.get::<_, String>(14)?)?,
        default_db: r.get(15)?,
        key_separator: r.get(16)?,
        last_opened_at: r.get(17)?,
    })
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Vec<RedisDatabase>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM redis_databases d LEFT JOIN hosts h ON h.id = d.host_id WHERE d.vault_id = ?1 ORDER BY d.name COLLATE NOCASE"
    ))?;
    let rows = st.query_map([vault_id], row)?.collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, vault_id: &str, id: &str) -> AppResult<RedisDatabase> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM redis_databases d LEFT JOIN hosts h ON h.id = d.host_id WHERE d.id = ?1 AND d.vault_id = ?2"),
        params![id, vault_id],
        row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("That database is no longer saved."))
}

/// The saved password, for the connection code only. It stays in `Zeroizing` memory.
pub fn password(
    conn: &Connection,
    s: &VaultSession,
    id: &str,
) -> AppResult<Option<Zeroizing<String>>> {
    let secret: Option<String> = conn
        .query_row(
            "SELECT password_secret_id FROM redis_databases WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(sid) = secret else { return Ok(None) };
    let bytes = secrets::get(conn, s, &sid)?;
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| AppError::internal("a stored password is not text"))?;
    Ok(Some(Zeroizing::new(text)))
}

/// The names of the databases that go through this host: deleting the host is refused while there are any.
pub fn names_using_host(conn: &Connection, host_id: &str) -> AppResult<Vec<String>> {
    let mut st =
        conn.prepare("SELECT name FROM redis_databases WHERE host_id = ?1 ORDER BY name")?;
    let rows = st
        .query_map([host_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Checks the form. The address is a host name or an IP as the route's last hop sees it: no spaces, no scheme.
pub fn validate(input: &RedisDatabaseInput) -> AppResult<()> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::validation(
            "Give the database a name of up to 64 characters.",
        ));
    }
    let address = input.address.trim();
    if address.is_empty()
        || address.len() > 255
        || address
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '/')
    {
        return Err(AppError::validation(
            "The address is a host name or an IP address, without spaces or a path.",
        ));
    }
    if input.port == 0 {
        return Err(AppError::validation(
            "The port must be between 1 and 65535.",
        ));
    }
    if (input.route == RedisRoute::Host) != clean(&input.host_id).is_some() {
        return Err(AppError::validation(if input.route == RedisRoute::Host {
            "Pick the host to go through."
        } else {
            "A direct database has no host."
        }));
    }
    if input.kind == RedisKind::Sentinel && clean(&input.sentinel_master).is_none() {
        return Err(AppError::validation(
            "A Sentinel database needs the name of its primary.",
        ));
    }
    if input.default_db > 255 {
        return Err(AppError::validation(
            "The database number must be between 0 and 255.",
        ));
    }
    let sep = &input.key_separator;
    if sep.is_empty() || sep.chars().count() > 4 || sep.chars().any(char::is_whitespace) {
        return Err(AppError::validation(
            "The key separator is one to four characters, such as : or /.",
        ));
    }
    Ok(())
}

/// Creates or updates a database in one transaction, password included. Returns the saved row.
pub fn save(
    conn: &mut Connection,
    s: &VaultSession,
    input: RedisDatabaseInput,
) -> AppResult<RedisDatabase> {
    validate(&input)?;
    let tx = conn.transaction()?;
    let host_id = clean(&input.host_id);
    if let Some(h) = &host_id {
        let found: Option<String> = tx
            .query_row(
                "SELECT id FROM hosts WHERE id = ?1 AND vault_id = ?2",
                params![h, s.vault_id],
                |r| r.get(0),
            )
            .optional()?;
        if found.is_none() {
            return Err(AppError::validation("That host is gone. Pick another one."));
        }
    }
    // The editor cannot loosen a production database's mode: that goes through `set_mode`, which asks for the
    // master password.
    if let Some(existing) = &input.id {
        let old = get(&tx, &s.vault_id, existing)?;
        if old.environment == RedisEnvironment::Production
            && gate::protection(input.write_mode) < gate::protection(old.write_mode)
        {
            return Err(AppError::validation(
                "A production database's write mode changes from the mode button in its tab, with the master password.",
            ));
        }
    }
    let now = now_ms();
    let id = match &input.id {
        Some(id) => id.clone(),
        None => uuid::Uuid::now_v7().to_string(),
    };
    let old_secret: Option<String> = match &input.id {
        Some(existing) => tx
            .query_row(
                "SELECT password_secret_id FROM redis_databases WHERE id = ?1 AND vault_id = ?2",
                params![existing, s.vault_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::not_found("That database is no longer saved."))?,
        None => None,
    };
    // An empty password field means "keep what is saved"; only `clear_password` removes it.
    let new_password = input.password.as_deref().filter(|p| !p.is_empty());
    let secret_id = match (new_password, &old_secret, input.clear_password) {
        (Some(p), Some(sid), _) => {
            secrets::replace(&tx, s, sid, p.as_bytes())?;
            Some(sid.clone())
        }
        (Some(p), None, _) => Some(secrets::put(&tx, s, SecretKind::Password, p.as_bytes())?),
        (None, _, true) => None,
        (None, old, false) => old.clone(),
    };
    let values = params![
        id,
        s.vault_id,
        input.name.trim(),
        input.environment.as_str(),
        input.route.as_str(),
        host_id,
        input.address.trim(),
        input.port,
        input.kind.as_str(),
        clean(&input.sentinel_master),
        clean(&input.username),
        secret_id,
        input.tls,
        clean(&input.tls_server_name),
        input.write_mode.as_str(),
        input.default_db,
        input.key_separator,
        now,
    ];
    if input.id.is_some() {
        tx.execute(
            "UPDATE redis_databases SET name = ?3, environment = ?4, route = ?5, host_id = ?6, address = ?7, port = ?8, kind = ?9,
               sentinel_master = ?10, username = ?11, password_secret_id = ?12, tls = ?13, tls_server_name = ?14, write_mode = ?15,
               default_db = ?16, key_separator = ?17, updated_at = ?18 WHERE id = ?1 AND vault_id = ?2",
            values,
        )?;
    } else {
        tx.execute(
            "INSERT INTO redis_databases (id, vault_id, name, environment, route, host_id, address, port, kind, sentinel_master, username,
               password_secret_id, tls, tls_server_name, write_mode, default_db, key_separator, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18)",
            values,
        )?;
    }
    // The saved password was removed: its secret goes too (after the row stopped pointing at it).
    if let (Some(old), None) = (&old_secret, &secret_id) {
        secrets::delete(&tx, s, old)?;
    }
    let saved = get(&tx, &s.vault_id, &id)?;
    tx.commit()?;
    Ok(saved)
}

/// Changes only the write mode. The caller has already checked the master password where one is needed.
pub fn set_mode(
    conn: &Connection,
    vault_id: &str,
    id: &str,
    mode: RedisWriteMode,
) -> AppResult<RedisDatabase> {
    conn.execute(
        "UPDATE redis_databases SET write_mode = ?3, updated_at = ?4 WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id, mode.as_str(), now_ms()],
    )?;
    get(conn, vault_id, id)
}

pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let tx = conn.transaction()?;
    let secrets_of: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT password_secret_id, sentinel_password_secret_id FROM redis_databases WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((a, b)) = secrets_of else {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "That database is no longer saved.",
        ));
    };
    tx.execute("DELETE FROM redis_databases WHERE id = ?1", [id])?;
    for sid in [a, b].into_iter().flatten() {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn touch_opened(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE redis_databases SET last_opened_at = ?1 WHERE id = ?2 AND vault_id = ?3",
        params![now_ms(), id, vault_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn fresh() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    fn host(conn: &mut Connection, s: &VaultSession, label: &str) -> String {
        crate::hosts::save(
            conn,
            s,
            &crate::hosts::HostInput {
                id: None,
                label: label.into(),
                address: "10.0.0.5".into(),
                port: None,
                username: Some("deploy".into()),
                password: None,
                clear_password: false,
                group_id: None,
                key_id: None,
                identity_id: None,
                startup_snippet_id: None,
                jump_mode: "inherit".into(),
                jump_host_ids: vec![],
                tags: vec![],
            },
        )
        .unwrap()
        .id
    }

    fn input(name: &str) -> RedisDatabaseInput {
        RedisDatabaseInput {
            id: None,
            name: name.into(),
            environment: RedisEnvironment::None,
            route: RedisRoute::Direct,
            host_id: None,
            address: "localhost".into(),
            port: 6379,
            kind: RedisKind::Single,
            sentinel_master: None,
            username: None,
            password: None,
            clear_password: false,
            tls: false,
            tls_server_name: None,
            write_mode: RedisWriteMode::Confirm,
            default_db: 0,
            key_separator: ":".into(),
        }
    }

    #[test]
    fn a_database_is_saved_listed_and_never_gives_its_password_back() {
        let (mut conn, s) = fresh();
        let h = host(&mut conn, &s, "db-primary");
        let saved = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                environment: RedisEnvironment::Production,
                route: RedisRoute::Host,
                host_id: Some(h.clone()),
                address: " 127.0.0.1 ".into(),
                username: Some("app-ro".into()),
                password: Some("s3cret-s3cret".into()),
                tls: true,
                ..input("cache-prod")
            },
        )
        .unwrap();
        assert_eq!(saved.address, "127.0.0.1");
        assert_eq!(saved.host_label.as_deref(), Some("db-primary"));
        assert!(saved.has_password);
        assert_eq!(saved.write_mode, RedisWriteMode::Confirm);
        // The row as the UI gets it has no password field at all; the text must not appear in its JSON.
        let json = serde_json::to_string(&list(&conn, &s.vault_id).unwrap()).unwrap();
        assert!(!json.contains("s3cret"), "{json}");
        assert_eq!(
            password(&conn, &s, &saved.id).unwrap().unwrap().as_str(),
            "s3cret-s3cret"
        );
    }

    #[test]
    fn an_empty_password_keeps_the_saved_one_and_clear_removes_it_with_its_secret() {
        let (mut conn, s) = fresh();
        let saved = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                password: Some("first-password".into()),
                ..input("local")
            },
        )
        .unwrap();
        let secrets_now = |c: &Connection| {
            c.query_row("SELECT count(*) FROM secrets", [], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        assert_eq!(secrets_now(&conn), 1);

        // Edit with the password field left empty: still the first password.
        let again = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                id: Some(saved.id.clone()),
                password: Some(String::new()),
                ..input("local renamed")
            },
        )
        .unwrap();
        assert_eq!(again.name, "local renamed");
        assert_eq!(
            password(&conn, &s, &saved.id).unwrap().unwrap().as_str(),
            "first-password"
        );

        // A new password replaces it in place (one secret, not two).
        save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                id: Some(saved.id.clone()),
                password: Some("second-password".into()),
                ..input("local")
            },
        )
        .unwrap();
        assert_eq!(secrets_now(&conn), 1);
        assert_eq!(
            password(&conn, &s, &saved.id).unwrap().unwrap().as_str(),
            "second-password"
        );

        let cleared = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                id: Some(saved.id.clone()),
                clear_password: true,
                ..input("local")
            },
        )
        .unwrap();
        assert!(!cleared.has_password);
        assert_eq!(secrets_now(&conn), 0);
    }

    #[test]
    fn the_orphan_sweep_at_unlock_leaves_a_database_password_alone() {
        let (mut conn, s) = fresh();
        let saved = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                password: Some("keep-me-please".into()),
                ..input("local")
            },
        )
        .unwrap();
        secrets::sweep_orphans(&conn, &s.vault_id).unwrap();
        assert_eq!(
            password(&conn, &s, &saved.id).unwrap().unwrap().as_str(),
            "keep-me-please"
        );
    }

    #[test]
    fn deleting_a_database_removes_its_secret_and_frees_its_host() {
        let (mut conn, s) = fresh();
        let h = host(&mut conn, &s, "bastion");
        let saved = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                route: RedisRoute::Host,
                host_id: Some(h.clone()),
                password: Some("a-long-password".into()),
                ..input("queue")
            },
        )
        .unwrap();

        // The host is the route to a database: it can't be deleted, and the message names the database.
        let err = crate::hosts::delete(&mut conn, &s, &h).unwrap_err();
        assert_eq!(err.code, ErrorCode::InUse);
        assert!(err.message.contains("queue"), "{}", err.message);

        delete(&mut conn, &s, &saved.id).unwrap();
        assert!(list(&conn, &s.vault_id).unwrap().is_empty());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM secrets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        crate::hosts::delete(&mut conn, &s, &h).unwrap();
        assert_eq!(
            delete(&mut conn, &s, &saved.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn the_form_is_checked() {
        let bad = |i: RedisDatabaseInput| validate(&i).unwrap_err().code == ErrorCode::Validation;
        assert!(validate(&input("ok")).is_ok());
        assert!(bad(input("  ")));
        assert!(bad(input(&"x".repeat(65))));
        assert!(bad(RedisDatabaseInput {
            address: "".into(),
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            address: "redis://host".into(),
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            address: "two words".into(),
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            port: 0,
            ..input("a")
        }));
        // The route and the host go together
        assert!(bad(RedisDatabaseInput {
            route: RedisRoute::Host,
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            host_id: Some("h".into()),
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            kind: RedisKind::Sentinel,
            ..input("a")
        }));
        assert!(validate(&RedisDatabaseInput {
            kind: RedisKind::Sentinel,
            sentinel_master: Some("mymaster".into()),
            ..input("a")
        })
        .is_ok());
        assert!(bad(RedisDatabaseInput {
            default_db: 256,
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            key_separator: "".into(),
            ..input("a")
        }));
        assert!(bad(RedisDatabaseInput {
            key_separator: " ".into(),
            ..input("a")
        }));
    }

    #[test]
    fn a_host_that_is_gone_is_refused_and_another_vaults_database_is_not_found() {
        let (mut conn, s) = fresh();
        let err = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                route: RedisRoute::Host,
                host_id: Some("no-such-host".into()),
                ..input("a")
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation);
        assert_eq!(
            get(&conn, "another-vault", "x").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn the_editor_cannot_loosen_a_production_databases_mode_but_set_mode_can() {
        let (mut conn, s) = fresh();
        let prod = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                environment: RedisEnvironment::Production,
                write_mode: RedisWriteMode::ReadOnly,
                ..input("cache-prod")
            },
        )
        .unwrap();
        let edit = |mode| RedisDatabaseInput {
            id: Some(prod.id.clone()),
            environment: RedisEnvironment::Production,
            write_mode: mode,
            ..input("cache-prod")
        };
        for looser in [RedisWriteMode::Confirm, RedisWriteMode::Open] {
            let err = save(&mut conn, &s, edit(looser)).unwrap_err();
            assert_eq!(err.code, ErrorCode::Validation);
        }
        assert_eq!(
            get(&conn, &s.vault_id, &prod.id).unwrap().write_mode,
            RedisWriteMode::ReadOnly
        );
        // The same mode, and any other edit, still save.
        assert!(save(&mut conn, &s, edit(RedisWriteMode::ReadOnly)).is_ok());
        // `set_mode` is the way (the command checks the master password before it calls this).
        let after = set_mode(&conn, &s.vault_id, &prod.id, RedisWriteMode::Confirm).unwrap();
        assert_eq!(after.write_mode, RedisWriteMode::Confirm);
        // Tightening is always allowed from the editor, and a database that isn't production is free.
        assert!(save(&mut conn, &s, edit(RedisWriteMode::ReadOnly)).is_ok());
        let dev = save(&mut conn, &s, input("cache-dev")).unwrap();
        let open = save(
            &mut conn,
            &s,
            RedisDatabaseInput {
                id: Some(dev.id),
                write_mode: RedisWriteMode::Open,
                ..input("cache-dev")
            },
        )
        .unwrap();
        assert_eq!(open.write_mode, RedisWriteMode::Open);
        assert_eq!(
            set_mode(&conn, &s.vault_id, "nope", RedisWriteMode::Open)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn the_enums_are_stored_as_the_words_the_schema_checks() {
        assert_eq!(RedisWriteMode::ReadOnly.as_str(), "read_only");
        assert_eq!(RedisEnvironment::Production.as_str(), "production");
        assert_eq!(RedisRoute::Host.as_str(), "host");
        assert_eq!(RedisKind::Sentinel.as_str(), "sentinel");
        assert!(RedisWriteMode::parse("nope").is_err());
    }
}
