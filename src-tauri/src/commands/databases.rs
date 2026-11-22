//! Commands shared by every engine (spec section 3 "Commands"; F20; S34, S35). Thin: the logic is in `crate::dbs`.
//! A password goes UI -> Rust once and is never returned (`has_password`).

use tauri::State;
use zeroize::Zeroizing;

use crate::dbs::route::{self, DbTestResult};
use crate::dbs::saved::{self, SavedQuery, SavedQueryInput};
use crate::dbs::store::{self, DatabaseInput, DatabaseRow};
use crate::dbs::transport::Target;
use crate::dbs::url::{self, DbUrlFields};
use crate::dbs::Engine;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// S34: every saved database of every engine.
#[tauri::command]
#[specta::specta]
pub async fn db_list(state: State<'_, AppState>) -> AppResult<Vec<DatabaseRow>> {
    state
        .with_db_session(|c, s| store::list(c, &s.vault_id))
        .await
}

/// S35: creates or updates a PostgreSQL, MySQL, MariaDB or MongoDB database. A save that makes a production
/// database less protected needs the master password (the same rule and cooldown as Redis).
#[tauri::command]
#[specta::specta]
pub async fn db_save(
    state: State<'_, AppState>,
    input: DatabaseInput,
    master_password: Option<String>,
) -> AppResult<DatabaseRow> {
    state.require_unlocked()?;
    let (lowers, input) = state
        .with_db_session(move |c, s| Ok((store::lowers_protection(c, &s.vault_id, &input)?, input)))
        .await?;
    if lowers {
        super::redis::check_master(&state, master_password).await?;
        log::info!(
            "databases: {} was made less protected from the editor",
            input.name.trim()
        );
    }
    state
        .with_db_session(move |c, s| store::save(c, s, input, lowers))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn db_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| store::delete(c, s, &id))
        .await
}

/// S35 "Test connection". For now it proves the route (see `dbs::route`); each engine adds its sign-in with its phase.
#[tauri::command]
#[specta::specta]
pub async fn db_test(state: State<'_, AppState>, input: DatabaseInput) -> AppResult<DbTestResult> {
    state.require_unlocked()?;
    store::validate(&input)?;
    // The first member of a MongoDB seed list is enough to prove the route.
    let first = input.address.split(',').next().unwrap_or("").trim();
    let (address, port) = match (input.engine, input.port, first.rsplit_once(':')) {
        (_, Some(p), _) => (first.to_string(), p),
        (_, None, Some((h, p))) => (
            h.to_string(),
            p.parse()
                .map_err(|_| AppError::validation("The port must be between 1 and 65535."))?,
        ),
        (e, None, None) => (first.to_string(), e.default_port()),
    };
    if input.options.srv {
        return Err(AppError::validation(
            "An SRV address is looked up when you open the database, so it can't be tested from here yet.",
        ));
    }
    if input.engine == Engine::Postgres {
        // The whole sign-in: the saved password stands in for an empty field (as in the Redis form).
        let typed = input
            .password
            .clone()
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new);
        let saved_id = input.id.clone();
        let clear = input.clear_password;
        let saved = match (&typed, saved_id, clear) {
            (None, Some(id), false) => {
                state
                    .with_db_session(move |c, s| store::password(c, s, &id))
                    .await?
            }
            _ => None,
        };
        let cfg = crate::sql::SqlConfig {
            name: input.name.trim().to_string(),
            engine: input.engine,
            route: input.route,
            host_id: input.host_id.clone().filter(|h| !h.is_empty()),
            address,
            port,
            database: input
                .default_database
                .clone()
                .filter(|d| !d.trim().is_empty()),
            user: input.username.clone().unwrap_or_default(),
            password: typed.or(saved),
            tls: crate::dbs::transport::TlsMode::parse(&input.tls)
                .ok_or_else(|| AppError::validation("Pick off, require or verify for TLS."))?,
            server_name: input.tls_server_name.clone(),
        };
        let started = std::time::Instant::now();
        let info = crate::sql::test(&state.store(), cfg).await?;
        return Ok(DbTestResult {
            engine: input.engine,
            latency_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            detail: format!(
                "Connected · {} · {} as {}",
                info.version
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" "),
                info.database,
                info.user
            ),
        });
    }
    let target = Target {
        address,
        port,
        // PostgreSQL and MySQL start TLS inside their own protocol; MongoDB's TLS arrives with its client.
        tls: false,
        tls_server_name: input.tls_server_name.clone(),
    };
    route::reach(
        &state.store(),
        input.engine,
        input.name.trim(),
        input.route,
        input.host_id.as_deref().filter(|h| !h.is_empty()),
        &target,
    )
    .await
}

/// S35 "paste a connection string": the fields to fill in. The password is not sent back; the form keeps the string
/// (`DatabaseInput::from_url`) and the save reads the password from it.
#[tauri::command]
#[specta::specta]
pub fn db_parse_url(url: String) -> AppResult<DbUrlFields> {
    let url = Zeroizing::new(url);
    url::parse(&url).map(DbUrlFields::from)
}

/// The engines this build can open, in picker order.
#[tauri::command]
#[specta::specta]
pub fn db_engines() -> Vec<Engine> {
    vec![
        Engine::Postgres,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Mongodb,
        Engine::Redis,
    ]
}

#[tauri::command]
#[specta::specta]
pub async fn saved_list(state: State<'_, AppState>) -> AppResult<Vec<SavedQuery>> {
    state.with_db_session(|c, s| saved::list(c, s)).await
}

#[tauri::command]
#[specta::specta]
pub async fn saved_save(
    state: State<'_, AppState>,
    input: SavedQueryInput,
) -> AppResult<SavedQuery> {
    state
        .with_db_session(move |c, s| saved::save(c, s, input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn saved_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| saved::delete(c, s, &id))
        .await
}
