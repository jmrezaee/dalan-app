//! Saved databases of every engine (S34, S35; F20): the `databases` table. Redis keeps its own editor and commands
//! (`redis::store`); this module lists all engines together and saves the others. The password lives in `secrets`
//! and is never returned (hard rule 1): the window only sees `has_password`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::Engine;
use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::gate;
use crate::redis::store::{clean, RedisEnvironment, RedisRoute, RedisWriteMode};
use crate::vault::secrets::{self, SecretKind};
use crate::vault::VaultSession;

/// What only some engines have. Stored as the `options` JSON and checked here per engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
pub struct DbOptions {
    /// MongoDB: the database that holds the user (`authSource`).
    pub auth_source: Option<String>,
    /// MongoDB: the replica set's name.
    pub replica_set: Option<String>,
    /// MongoDB: `mongodb+srv`, the address is a DNS name that lists the members.
    #[serde(default)]
    pub srv: bool,
}

/// One saved database of any engine, as the list (S34) sees it.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DatabaseRow {
    pub id: String,
    pub engine: Engine,
    pub name: String,
    pub environment: RedisEnvironment,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    pub host_label: Option<String>,
    pub address: String,
    pub port: Option<u16>,
    pub default_database: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
    /// `off`, `require` or `verify`.
    pub tls: String,
    pub tls_server_name: Option<String>,
    pub write_mode: RedisWriteMode,
    pub options: DbOptions,
    #[specta(type = Option<f64>)]
    pub last_opened_at: Option<i64>,
}

/// The editor's form (S35) for PostgreSQL, MySQL, MariaDB and MongoDB. `password: Some` sets a new one;
/// `clear_password` removes the saved one. `from_url` is a connection string the form was filled from: its password
/// is used when the password field is empty.
#[derive(Debug, Deserialize, Type)]
pub struct DatabaseInput {
    pub id: Option<String>,
    pub engine: Engine,
    pub name: String,
    pub environment: RedisEnvironment,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    pub address: String,
    pub port: Option<u16>,
    pub default_database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub clear_password: bool,
    pub tls: String,
    pub tls_server_name: Option<String>,
    pub write_mode: RedisWriteMode,
    pub options: DbOptions,
    pub from_url: Option<String>,
}

const COLUMNS: &str = "d.id, d.engine, d.name, d.environment, d.route, d.host_id, h.label, d.address, d.port, d.default_database, d.username, d.password_secret_id IS NOT NULL, d.tls, d.tls_server_name, d.write_mode, d.options, d.last_opened_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DatabaseRow> {
    let engine: String = r.get(1)?;
    let options: String = r.get(15)?;
    Ok(DatabaseRow {
        id: r.get(0)?,
        engine: Engine::parse(&engine).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(1, engine, rusqlite::types::Type::Text)
        })?,
        name: r.get(2)?,
        environment: RedisEnvironment::parse(&r.get::<_, String>(3)?)?,
        route: RedisRoute::parse(&r.get::<_, String>(4)?)?,
        host_id: r.get(5)?,
        host_label: r.get(6)?,
        address: r.get(7)?,
        port: r.get(8)?,
        default_database: r.get(9)?,
        username: r.get(10)?,
        has_password: r.get(11)?,
        tls: r.get(12)?,
        tls_server_name: r.get(13)?,
        write_mode: RedisWriteMode::parse(&r.get::<_, String>(14)?)?,
        // Redis rows keep their own keys here (kind, key separator); a row we can't read as options has none.
        options: serde_json::from_str(&options).unwrap_or_default(),
        last_opened_at: r.get(16)?,
    })
}

/// S34: every saved database of every engine, by name.
pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Vec<DatabaseRow>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM databases d LEFT JOIN hosts h ON h.id = d.host_id WHERE d.vault_id = ?1 ORDER BY d.name COLLATE NOCASE"
    ))?;
    let rows = st.query_map([vault_id], row)?.collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, vault_id: &str, id: &str) -> AppResult<DatabaseRow> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM databases d LEFT JOIN hosts h ON h.id = d.host_id WHERE d.id = ?1 AND d.vault_id = ?2"),
        params![id, vault_id],
        row,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("That database is no longer saved."))
}

/// Checks the form. Redis is saved by its own editor.
pub fn validate(input: &DatabaseInput) -> AppResult<()> {
    if input.engine == Engine::Redis {
        return Err(AppError::validation(
            "Redis databases are saved from the Redis form.",
        ));
    }
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::validation(
            "Give the database a name of up to 64 characters.",
        ));
    }
    let address = input.address.trim();
    // MongoDB lists its members: "h1:27017,h2:27017". Each entry is a host name or IP (with an optional port).
    let entries: Vec<&str> = if input.engine == Engine::Mongodb {
        address
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .collect()
    } else {
        vec![address]
    };
    let bad = |e: &str| {
        e.is_empty()
            || e.chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '/')
    };
    if address.is_empty()
        || address.len() > 255
        || entries.is_empty()
        || entries.iter().any(|e| bad(e))
        || (input.engine != Engine::Mongodb && address.contains(','))
    {
        return Err(AppError::validation(if input.engine == Engine::Mongodb {
            "List the members as addresses with a comma between them, such as db1:27017,db2:27017."
        } else {
            "The address is a host name or an IP address, without spaces or a path."
        }));
    }
    if input.port == Some(0) {
        return Err(AppError::validation(
            "The port must be between 1 and 65535.",
        ));
    }
    if input.engine != Engine::Mongodb && input.port.is_none() {
        return Err(AppError::validation("Give the port."));
    }
    if (input.route == RedisRoute::Host) != clean(&input.host_id).is_some() {
        return Err(AppError::validation(if input.route == RedisRoute::Host {
            "Pick the host to go through."
        } else {
            "A direct database has no host."
        }));
    }
    if !matches!(input.tls.as_str(), "off" | "require" | "verify") {
        return Err(AppError::validation("Pick off, require or verify for TLS."));
    }
    if input.engine != Engine::Mongodb
        && (input.options.auth_source.is_some()
            || input.options.replica_set.is_some()
            || input.options.srv)
    {
        return Err(AppError::validation(
            "Replica set, auth source and SRV are MongoDB settings.",
        ));
    }
    if input.options.srv && (entries.len() != 1 || input.port.is_some()) {
        return Err(AppError::validation(
            "An SRV address is one DNS name, with no port.",
        ));
    }
    Ok(())
}

/// Whether saving this form would make a saved production database less protected (see `redis::store`).
pub fn lowers_protection(
    conn: &Connection,
    vault_id: &str,
    input: &DatabaseInput,
) -> AppResult<bool> {
    let Some(id) = &input.id else {
        return Ok(false);
    };
    let old = get(conn, vault_id, id)?;
    Ok(old.environment == RedisEnvironment::Production
        && (input.environment != RedisEnvironment::Production
            || gate::protection(input.write_mode) < gate::protection(old.write_mode)))
}

/// The password to save: typed, else the one in the connection string the form was filled from.
fn new_password(input: &DatabaseInput) -> Option<String> {
    input
        .password
        .clone()
        .filter(|p| !p.is_empty())
        .or_else(|| {
            input
                .from_url
                .as_deref()
                .and_then(|u| super::url::parse(u).ok())
                .and_then(|p| p.password)
                .filter(|p| !p.is_empty())
        })
}

/// Creates or updates a database in one transaction, password included.
/// `master_checked`: the caller has just checked the master password, so the save may lower a production
/// database's protection.
pub fn save(
    conn: &mut Connection,
    s: &VaultSession,
    input: DatabaseInput,
    master_checked: bool,
) -> AppResult<DatabaseRow> {
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
    if !master_checked && lowers_protection(&tx, &s.vault_id, &input)? {
        return Err(AppError::new(
            ErrorCode::WrongPassword,
            "Type the master password to make a production database less protected.",
        ));
    }
    let now = now_ms();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let old_secret: Option<String> = match &input.id {
        Some(existing) => tx
            .query_row(
                "SELECT password_secret_id FROM databases WHERE id = ?1 AND vault_id = ?2 AND engine = ?3",
                params![existing, s.vault_id, input.engine.as_str()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::not_found("That database is no longer saved."))?,
        None => None,
    };
    // An empty password field means "keep what is saved"; only `clear_password` removes it.
    let typed = new_password(&input);
    let secret_id = match (typed.as_deref(), &old_secret, input.clear_password) {
        (Some(p), Some(sid), _) => {
            secrets::replace(&tx, s, sid, p.as_bytes())?;
            Some(sid.clone())
        }
        (Some(p), None, _) => Some(secrets::put(&tx, s, SecretKind::Password, p.as_bytes())?),
        (None, _, true) => None,
        (None, old, false) => old.clone(),
    };
    let options = serde_json::to_string(&input.options)
        .map_err(|e| AppError::internal(format!("options: {e}")))?;
    let values = params![
        id,
        s.vault_id,
        input.engine.as_str(),
        input.name.trim(),
        input.environment.as_str(),
        input.route.as_str(),
        host_id,
        input.address.trim(),
        input.port,
        clean(&input.default_database),
        clean(&input.username),
        secret_id,
        input.tls,
        clean(&input.tls_server_name),
        input.write_mode.as_str(),
        options,
        now,
    ];
    if input.id.is_some() {
        tx.execute(
            "UPDATE databases SET name = ?4, environment = ?5, route = ?6, host_id = ?7, address = ?8, port = ?9,
               default_database = ?10, username = ?11, password_secret_id = ?12, tls = ?13, tls_server_name = ?14,
               write_mode = ?15, options = ?16, updated_at = ?17 WHERE id = ?1 AND vault_id = ?2 AND engine = ?3",
            values,
        )?;
    } else {
        tx.execute(
            "INSERT INTO databases (id, vault_id, engine, name, environment, route, host_id, address, port,
               default_database, username, password_secret_id, tls, tls_server_name, write_mode, options, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
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

/// Deletes a database of any engine and its secrets. Its history goes with it (cascade); saved queries stay,
/// no longer tied to it.
pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let tx = conn.transaction()?;
    let found: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT password_secret_id, extra_secret_id FROM databases WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((a, b)) = found else {
        return Err(AppError::not_found("That database is no longer saved."));
    };
    tx.execute("DELETE FROM databases WHERE id = ?1", [id])?;
    for sid in [a, b].into_iter().flatten() {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn input(engine: Engine, name: &str) -> DatabaseInput {
        DatabaseInput {
            id: None,
            engine,
            name: name.into(),
            environment: RedisEnvironment::None,
            route: RedisRoute::Direct,
            host_id: None,
            address: "db.internal".into(),
            port: Some(engine.default_port()),
            default_database: Some("shop".into()),
            username: Some("app".into()),
            password: Some("s3cret".into()),
            clear_password: false,
            tls: "off".into(),
            tls_server_name: None,
            write_mode: RedisWriteMode::Confirm,
            options: DbOptions::default(),
            from_url: None,
        }
    }

    fn setup() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    #[test]
    fn a_saved_database_has_a_flag_for_its_password_never_the_password() {
        let (mut conn, s) = setup();
        let saved = save(&mut conn, &s, input(Engine::Postgres, "shop"), false).unwrap();
        assert!(saved.has_password);
        assert_eq!(saved.engine, Engine::Postgres);
        // The serialised row has no field that could hold it.
        let json = serde_json::to_string(&saved).unwrap();
        assert!(!json.contains("s3cret"), "{json}");
        let sid: String = conn
            .query_row("SELECT password_secret_id FROM databases", [], |r| r.get(0))
            .unwrap();
        assert_eq!(secrets::get(&conn, &s, &sid).unwrap().as_slice(), b"s3cret");
    }

    #[test]
    fn the_list_shows_every_engine_and_delete_removes_the_secret() {
        let (mut conn, s) = setup();
        let a = save(&mut conn, &s, input(Engine::Postgres, "b-shop"), false).unwrap();
        save(&mut conn, &s, input(Engine::Mongodb, "a-logs"), false).unwrap();
        let names: Vec<_> = list(&conn, &s.vault_id)
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["a-logs", "b-shop"]);
        delete(&mut conn, &s, &a.id).unwrap();
        let secrets_left: i64 = conn
            .query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(secrets_left, 1);
    }

    #[test]
    fn the_form_is_checked_per_engine() {
        let mut i = input(Engine::Postgres, "x");
        i.options.replica_set = Some("rs0".into());
        assert!(validate(&i).is_err(), "replica set is MongoDB only");
        let mut m = input(Engine::Mongodb, "x");
        m.address = "h1:27017, h2:27017".into();
        m.port = None;
        assert!(validate(&m).is_ok());
        m.options.srv = true;
        assert!(validate(&m).is_err(), "SRV is one name");
        assert!(validate(&input(Engine::Redis, "x")).is_err());
        let mut host = input(Engine::Mysql, "x");
        host.route = RedisRoute::Host;
        assert!(validate(&host).is_err(), "a host route needs a host");
        let mut tls = input(Engine::Mysql, "x");
        tls.tls = "maybe".into();
        assert!(validate(&tls).is_err());
    }

    #[test]
    fn loosening_a_production_database_needs_the_master_password() {
        let (mut conn, s) = setup();
        let mut p = input(Engine::Postgres, "prod");
        p.environment = RedisEnvironment::Production;
        p.write_mode = RedisWriteMode::ReadOnly;
        let saved = save(&mut conn, &s, p, false).unwrap();
        let mut loose = input(Engine::Postgres, "prod");
        loose.id = Some(saved.id.clone());
        loose.environment = RedisEnvironment::Production;
        loose.write_mode = RedisWriteMode::Open;
        assert_eq!(
            save(&mut conn, &s, loose, false).unwrap_err().code,
            ErrorCode::WrongPassword
        );
        // Un-marking production first is the same thing.
        let mut unmark = input(Engine::Postgres, "prod");
        unmark.id = Some(saved.id.clone());
        unmark.write_mode = RedisWriteMode::ReadOnly;
        assert_eq!(
            save(&mut conn, &s, unmark, false).unwrap_err().code,
            ErrorCode::WrongPassword
        );
        // Tightening is free.
        let mut tight = input(Engine::Postgres, "prod");
        tight.id = Some(saved.id);
        tight.environment = RedisEnvironment::Production;
        tight.write_mode = RedisWriteMode::ReadOnly;
        assert!(save(&mut conn, &s, tight, false).is_ok());
    }

    #[test]
    fn an_empty_password_keeps_the_saved_one_and_clear_removes_it() {
        let (mut conn, s) = setup();
        let saved = save(&mut conn, &s, input(Engine::Mysql, "m"), false).unwrap();
        let mut keep = input(Engine::Mysql, "m");
        keep.id = Some(saved.id.clone());
        keep.password = Some(String::new());
        assert!(save(&mut conn, &s, keep, false).unwrap().has_password);
        let mut clear = input(Engine::Mysql, "m");
        clear.id = Some(saved.id);
        clear.password = None;
        clear.clear_password = true;
        assert!(!save(&mut conn, &s, clear, false).unwrap().has_password);
        let left: i64 = conn
            .query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }
}
