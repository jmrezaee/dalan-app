//! Restore a dump made anywhere (D12, S48, F31; replica/databases/restore-copy.md §2). `dbs::dumpfile` says what the
//! file is; this module plans the restore against the open connection (target, clashes, tool, gate level), then runs
//! the engine's own tool where `backup.rs` runs it: on the host over SSH, or on this computer for a direct route.
//!
//! - Single-file dumps stream from this computer into the tool's standard input, after the password line the
//!   wrappers read. Plain SQL passes through `dumpscan` on the way, so what was checked is what the tool gets.
//! - Directory dumps are uploaded over SFTP into a private `mktemp -d` folder, which is removed when the tool ends,
//!   fails or is stopped.
//! - Stop kills the tool (on a host: its process group), never just closes its input: `psql --single-transaction`
//!   would take the end of its input as the end of the script and commit.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::ChannelMsg;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::backup::{self, shq};
use super::SqlConfig;
use crate::dbs::dumpfile::{self, Dump, DumpFormat};
use crate::dbs::dumpscan::{self, Content, Dialect, Existing, Rewrite};
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::Connected;

/// What the person chose in "Target and options".
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RestoreOptions {
    /// `Some(name)`: create this database on the same server and restore into it. `None`: the tab's database.
    pub new_database: Option<String>,
    /// Multi-database dumps: which one to restore.
    pub source_database: Option<String>,
    pub existing: Existing,
    pub content: Content,
    pub skip_owner_acl: bool,
    pub include_roles: bool,
    /// PostgreSQL: one transaction that stops at the first error. Elsewhere: stop at the first error.
    pub single_transaction: bool,
    /// Only these tables or collections (names as the preview shows them). `None`: everything.
    #[serde(default)]
    pub objects: Option<Vec<String>>,
    /// PostgreSQL: restore one schema of the dump under another name.
    #[serde(default)]
    pub rename_schema: Option<RenameSchema>,
    /// PostgreSQL custom and directory archives: parallel jobs (not with one transaction). 0 or 1: one at a time.
    #[serde(default)]
    pub jobs: u32,
    /// PostgreSQL archives, data only: turn triggers off while loading (needs a superuser).
    #[serde(default)]
    pub disable_triggers: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RenameSchema {
    pub from: String,
    pub to: String,
}

/// The review (S48 "Review"): everything the person confirms, in plain words.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RestorePlan {
    /// The held restore, or `None` when something blocks it (`blocked` says what).
    pub pending_id: Option<String>,
    pub blocked: Option<String>,
    /// `confirm` or `typed`.
    pub level: String,
    /// The name to type for a typed confirmation.
    pub typed_name: String,
    pub saved_name: String,
    pub environment: String,
    /// The database the dump goes into.
    pub target_db: String,
    pub new_database: bool,
    pub engine: Engine,
    pub tool: String,
    pub tool_version: String,
    pub runs_on: String,
    /// What exists at the target and will be dropped and replaced.
    pub replaces: Vec<String>,
    /// What exists at the target and stops a "stop if anything exists" restore.
    pub clashes: Vec<String>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    pub objects: f64,
    pub rows: f64,
    pub bytes: f64,
    /// Uploaded to the host first (directory dumps over a host route).
    pub upload: bool,
    /// The restore can be undone as a whole (one transaction).
    pub atomic: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RestoreProgress {
    /// `upload` while a folder is copied to the host, then `restore`.
    pub phase: String,
    pub bytes: f64,
    pub total: f64,
    pub current: Option<String>,
    pub objects: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RestoreWarning {
    pub text: String,
    pub count: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RestoreResult {
    /// `done`, `stopped` or `failed`.
    pub outcome: String,
    pub message: Option<String>,
    pub target_db: String,
    pub objects: f64,
    /// Objects the tool reported as created, in order (at most 200).
    pub done: Vec<String>,
    pub warnings: Vec<RestoreWarning>,
    pub seconds: f64,
    /// Stopped or failed inside one transaction: the database is as it was.
    pub nothing_changed: bool,
    /// A new database made for this restore was removed again after a failure or a Stop.
    pub removed_new_database: bool,
}

/// Which tool reads the dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    PgArchive,
    PgPlain,
    My,
    Mongo,
}

fn kind_of(f: DumpFormat) -> Kind {
    match f {
        DumpFormat::PgCustom | DumpFormat::PgTar | DumpFormat::PgDirectory => Kind::PgArchive,
        DumpFormat::PgPlain => Kind::PgPlain,
        DumpFormat::MySql => Kind::My,
        DumpFormat::MongoArchive | DumpFormat::MongoFolder => Kind::Mongo,
    }
}

/// Everything the commit needs, held by the gate between the review and the confirmation.
pub struct Job {
    pub dump_id: String,
    opts: RestoreOptions,
    kind: Kind,
    pub target_db: String,
    pub new_database: bool,
    tool: String,
    /// The `mysql` binary is MariaDB's (it keeps MariaDB's sandbox line).
    mariadb_client: bool,
    my_flags: Vec<String>,
    server_scs: bool,
    source_db: Option<String>,
    pub typed_name: String,
    pub atomic: bool,
    encoding: Option<String>,
    /// PostgreSQL archives: the TOC entries to restore (`-L`), when only some objects were chosen.
    list: Option<Vec<i64>>,
    /// The schema to rename, and when: in the stream (plain SQL) or after the tool (archives into this database).
    pub rename: Option<(String, String)>,
    /// A custom archive restored with parallel jobs: the tool needs it as a file, not on its input.
    as_file: bool,
}

// ---------------------------------------------------------------- planning

/// What the target already has: for PostgreSQL `schema.table`, for MySQL `table`, for MongoDB `collection`.
pub async fn existing_names(conn: &Conn<'_>, db: &str) -> AppResult<Vec<String>> {
    Ok(match conn {
        Conn::Pg(c) => c
            .client
            .query(
                "select n.nspname::text, c.relname::text from pg_class c join pg_namespace n on n.oid = c.relnamespace
                 where c.relkind in ('r','p','v','m','f','S') and n.nspname not in ('pg_catalog','information_schema') and n.nspname not like 'pg_toast%'",
                &[],
            )
            .await
            .map_err(|e| super::pg::map_error(&e))?
            .iter()
            .map(|r| format!("{}.{}", r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect(),
        Conn::My(m) => m
            .conn
            .query(&format!("select table_name from information_schema.tables where table_schema = {}", super::mydb::lit(db)), 0)
            .await?
            .rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect(),
        Conn::Mongo(m) => {
            let reply = m.command(db, bson::doc! { "listCollections": 1, "nameOnly": true }).await?;
            crate::mongo::docs::names(&reply, "name").iter().filter_map(|d| d.get_str("name").ok().map(str::to_string)).collect()
        }
    })
}

pub enum Conn<'a> {
    Pg(&'a super::pg::PgConn),
    My(&'a super::mydb::MyDb),
    Mongo(&'a crate::mongo::conn::MongoConn),
}

pub async fn database_exists(conn: &Conn<'_>, name: &str) -> AppResult<bool> {
    Ok(match conn {
        Conn::Pg(c) => !c
            .client
            .query("select 1 from pg_database where datname = $1", &[&name])
            .await
            .map_err(|e| super::pg::map_error(&e))?
            .is_empty(),
        Conn::My(m) => !m
            .conn
            .query(
                &format!(
                    "select schema_name from information_schema.schemata where schema_name = {}",
                    super::mydb::lit(name)
                ),
                0,
            )
            .await?
            .rows
            .is_empty(),
        Conn::Mongo(m) => {
            let reply = m
                .command(
                    "admin",
                    bson::doc! { "listDatabases": 1, "nameOnly": true, "filter": { "name": name } },
                )
                .await?;
            reply.get_array("databases").is_ok_and(|a| !a.is_empty())
        }
    })
}

pub async fn current_db(conn: &Conn<'_>, cfg: &SqlConfig) -> AppResult<Option<String>> {
    if let Some(d) = cfg.database.clone().filter(|d| !d.is_empty()) {
        return Ok(Some(d));
    }
    Ok(match conn {
        Conn::Pg(c) => Some(
            c.client
                .query_one("select current_database()::text", &[])
                .await
                .map_err(|e| super::pg::map_error(&e))?
                .get(0),
        ),
        _ => None,
    })
}

pub fn valid_new_name(engine: Engine, name: &str) -> AppResult<()> {
    let bad = name.is_empty()
        || name.len() > 63
        || name.chars().any(|c| c.is_control())
        || (engine == Engine::Mongodb
            && name.chars().any(|c| {
                matches!(
                    c,
                    '/' | '\\' | '.' | ' ' | '"' | '$' | '*' | '<' | '>' | ':' | '|' | '?'
                )
            }));
    if bad {
        return Err(AppError::validation("That name can't be a database name. Use letters, digits and underscores, up to 63 characters."));
    }
    Ok(())
}

/// pg_restore 17 and later write `SET transaction_timeout`, which a server before 17 refuses.
pub fn needs_older_restore(tool_major: u32, server_major: u32) -> bool {
    tool_major >= 17 && server_major > 0 && server_major < 17
}

/// Where a pg_restore of one major version usually lives (Debian, Homebrew on Apple and Intel Macs, Postgres.app).
pub fn versioned_pg_restore(major: u32) -> Vec<String> {
    vec![
        format!("/usr/lib/postgresql/{major}/bin/pg_restore"),
        format!("/opt/homebrew/opt/postgresql@{major}/bin/pg_restore"),
        format!("/usr/local/opt/postgresql@{major}/bin/pg_restore"),
        format!("/Applications/Postgres.app/Contents/Versions/{major}/bin/pg_restore"),
    ]
}

/// The candidates for the tool, as `backup.rs` names them.
fn tool_candidates(kind: Kind, engine: Engine) -> Vec<&'static str> {
    match kind {
        Kind::PgArchive => vec!["pg_restore"],
        Kind::PgPlain => vec!["psql"],
        Kind::My if engine == Engine::Mariadb => vec!["mariadb", "mysql"],
        Kind::My => vec!["mysql", "mariadb"],
        Kind::Mongo => vec!["mongorestore"],
    }
}

async fn resolve(
    ssh: Option<&Connected>,
    kind: Kind,
    engine: Engine,
) -> AppResult<(String, String)> {
    let c = tool_candidates(kind, engine);
    for t in &c {
        if let Ok(v) = backup::tool_version(ssh, t).await {
            return Ok((t.to_string(), v));
        }
    }
    Err(backup::missing(c[0], &backup::runs_on(ssh)))
}

/// The newest archive format a pg_restore of this major version reads.
fn pg_restore_reads(tool_major: u32) -> (u8, u8) {
    match tool_major {
        0..=9 => (1, 12),
        10 => (1, 13),
        11 => (1, 13),
        12..=15 => (1, 14),
        16 => (1, 15),
        _ => (1, 16),
    }
}

/// The tool's help text mentions a flag (MySQL's `--system-command`, MariaDB's `--sandbox`).
async fn supports(ssh: Option<&Connected>, tool: &str, flag: &str) -> bool {
    let help = match ssh {
        Some(s) => backup::exec_collect(s, &format!("{tool} --help 2>/dev/null"))
            .await
            .map(|(_, o)| o)
            .unwrap_or_default(),
        None => tokio::process::Command::new(backup::local_tool(tool))
            .arg("--help")
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default(),
    };
    help.contains(flag)
}

pub struct PlanInput<'a> {
    pub conn: Conn<'a>,
    pub cfg: &'a SqlConfig,
    pub ssh: Option<&'a Connected>,
    pub dump: &'a Dump,
    pub dump_id: String,
    pub opts: RestoreOptions,
    pub saved_name: String,
    pub environment: String,
    pub mode: crate::redis::store::RedisWriteMode,
}

/// Checks everything that can be checked before anything runs and builds the review. `Err` for what the person
/// must fix first (wrong engine, read-only, tool missing, dump too new); `blocked` for clashes they can choose around.
pub async fn plan(p: PlanInput<'_>) -> AppResult<(RestorePlan, Job)> {
    use crate::redis::store::RedisWriteMode as Mode;
    let d = &p.dump.info;
    if let Some(r) = &d.refused {
        return Err(AppError::new(ErrorCode::DbDumpUnsafe, r.clone()));
    }
    let engine = p.cfg.engine;
    let kind = kind_of(d.format);
    let fits = match d.engine {
        Engine::Mysql => matches!(engine, Engine::Mysql | Engine::Mariadb),
        e => e == engine,
    };
    if !fits {
        return Err(AppError::validation(format!(
            "This is a {} and {} is a {} database. Restore it into a {} database.",
            d.format_label,
            p.saved_name,
            engine.label(),
            if d.engine == Engine::Mysql {
                "MySQL or MariaDB"
            } else {
                d.engine.label()
            }
        )));
    }
    if p.mode == Mode::ReadOnly {
        return Err(crate::dbs::gate::read_only_error(&p.saved_name));
    }
    let production = p.environment == "production";
    let mut opts = p.opts.clone();
    if opts.existing == Existing::Append {
        opts.content = Content::Data;
    }
    let new_database = opts
        .new_database
        .clone()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    if new_database.is_none() && production && p.mode != Mode::Open {
        return Err(AppError::new(ErrorCode::DbReadOnlyMode, format!("{} is a production database. To restore into it, switch it to Open first (that needs the master password), or restore into a new database.", p.saved_name)));
    }
    let (target_db, new_db) = match &new_database {
        Some(n) => {
            valid_new_name(engine, n)?;
            if database_exists(&p.conn, n).await? {
                return Err(AppError::validation(format!("A database named {n} already exists on this server. Pick another name.")));
            }
            (n.clone(), true)
        }
        None => match current_db(&p.conn, p.cfg).await? {
            Some(db) => (db, false),
            None => return Err(AppError::validation("This connection isn't to one database. Restore into a new database, or set a default database for it.")),
        },
    };
    // Multi-database dumps: one part is restored.
    let source_db = if d.databases.len() > 1 {
        match opts
            .source_database
            .clone()
            .filter(|s| d.databases.contains(s))
        {
            Some(s) => Some(s),
            None => {
                return Err(AppError::validation(format!(
                    "The dump holds {} databases. Pick the one to restore.",
                    d.databases.len()
                )))
            }
        }
    } else {
        d.databases.first().cloned()
    };
    let ssh = p.ssh;
    let pg_engine = engine == Engine::Postgres;
    if (opts.rename_schema.is_some() || opts.jobs > 1 || opts.disable_triggers) && !pg_engine {
        return Err(AppError::validation(
            "Renaming a schema, parallel jobs and turning triggers off are for PostgreSQL dumps.",
        ));
    }
    let jobs = opts.jobs.max(1);
    if jobs > 1 {
        if !matches!(d.format, DumpFormat::PgCustom | DumpFormat::PgDirectory) {
            return Err(AppError::validation(
                "Parallel jobs work with custom and directory archives only.",
            ));
        }
        if opts.single_transaction {
            return Err(AppError::validation("Parallel jobs can't run in one transaction. Choose “Keep going and report the errors” to use them."));
        }
        if jobs > 16 {
            return Err(AppError::validation("Use at most 16 parallel jobs."));
        }
    }
    if opts.disable_triggers && (kind != Kind::PgArchive || opts.content != Content::Data) {
        return Err(AppError::validation(
            "Turning triggers off is for data-only restores of PostgreSQL archives.",
        ));
    }
    let mut rename = None;
    if let Some(r) = &opts.rename_schema {
        let (from, to) = (r.from.trim().to_string(), r.to.trim().to_string());
        if from.is_empty()
            || to.is_empty()
            || from == to
            || to.len() > 63
            || to.chars().any(|c| c.is_control())
        {
            return Err(AppError::validation(
                "Give the schema in the dump and a different new name for it.",
            ));
        }
        if !d.objects.iter().any(|o| {
            o.name.starts_with(&format!("{from}.")) || (o.kind == "schema" && o.name == from)
        }) {
            return Err(AppError::validation(format!(
                "The dump has no schema named {from}."
            )));
        }
        if kind == Kind::PgArchive && new_db {
            return Err(AppError::validation("Renaming a schema of an archive works when restoring into this database. Into a new database, restore it and rename the schema from the editor."));
        }
        if let Conn::Pg(c) = &p.conn {
            if !new_db {
                let have: Vec<String> = c
                    .client
                    .query("select nspname::text from pg_namespace", &[])
                    .await
                    .map_err(|e| super::pg::map_error(&e))?
                    .iter()
                    .map(|r| r.get(0))
                    .collect();
                for n in [&from, &to] {
                    if have.contains(n) {
                        return Err(AppError::validation(format!("{target_db} already has a schema named {n}, so the dump's {from} can't be restored as {to}. Restore into a new database instead.")));
                    }
                }
            }
        }
        rename = Some((from, to));
    }
    let (mut tool, mut tool_version) = resolve(ssh, kind, engine).await?;
    // pg_restore 17 and later set `transaction_timeout`, which servers before 17 refuse (and with one transaction the
    // whole restore stops on it). A client of the server's own version, when one is installed, doesn't.
    if kind == Kind::PgArchive {
        if let Conn::Pg(c) = &p.conn {
            let server = c
                .client
                .query_one("show server_version_num", &[])
                .await
                .ok()
                .and_then(|r| r.get::<_, String>(0).parse::<u32>().ok())
                .map_or(0, |n| n / 10_000);
            let tm = dumpfile::major(tool_version.rsplit(' ').next().unwrap_or("")).unwrap_or(0);
            if needs_older_restore(tm, server) {
                let reads = p
                    .dump
                    .archive_version
                    .is_none_or(|av| av <= pg_restore_reads(server));
                let mut found = None;
                for c in versioned_pg_restore(server) {
                    if reads {
                        if let Ok(v) = backup::tool_version(ssh, &c).await {
                            found = Some((c, v));
                            break;
                        }
                    }
                }
                match found {
                    Some((c, v)) => (tool, tool_version) = (c, v),
                    None => return Err(AppError::validation(format!(
                        "pg_restore {tm} {} sets transaction_timeout, which PostgreSQL {server} refuses, so this restore would stop at its first line. Install the PostgreSQL {server} client there (Dalan finds it in /usr/lib/postgresql/{server}/bin, Homebrew's postgresql@{server} or Postgres.app), or restore a plain SQL dump.",
                        backup::runs_on(ssh)
                    ))),
                }
            }
        }
    }
    let mut warnings = Vec::new();
    let mut skipped = Vec::new();
    // A dump newer than the tool.
    if kind == Kind::PgArchive {
        let tool_major =
            dumpfile::major(tool_version.rsplit(' ').next().unwrap_or("")).unwrap_or(0);
        if let (Some(av), true) = (p.dump.archive_version, tool_major > 0) {
            if av > pg_restore_reads(tool_major) {
                return Err(AppError::new(
                    ErrorCode::DbDumpTooNew,
                    format!("This dump came from pg_dump {}; {} has pg_restore {tool_major}, which can't read it. Install a newer PostgreSQL client there.", p.dump.dump_major.map_or("?".into(), |m| m.to_string()), backup::runs_on(ssh)),
                ));
            }
        }
    }
    if let (Some(dm), Some(tm)) = (
        p.dump.dump_major,
        dumpfile::major(tool_version.rsplit(' ').next().unwrap_or("")),
    ) {
        if matches!(kind, Kind::PgArchive | Kind::PgPlain) && dm > tm {
            warnings.push(format!("The dump came from pg_dump {dm} and {tool} here is version {tm}. Statements newer than the server may fail."));
        }
    }
    if d.mariadb && engine == Engine::Mysql {
        warnings.push("Made by MariaDB's dump tool. Most of it reads on MySQL; MariaDB-only syntax will fail.".to_string());
    }
    // What the target has.
    let have = if new_db {
        Vec::new()
    } else {
        existing_names(&p.conn, &target_db).await?
    };
    let have_lc: Vec<String> = have.iter().map(|h| h.to_ascii_lowercase()).collect();
    // Only some objects: each name must be one the dump has.
    let chosen = opts.objects.clone().filter(|o| !o.is_empty());
    if let Some(sel) = &chosen {
        let known: Vec<&str> = d
            .objects
            .iter()
            .filter(|o| {
                matches!(
                    o.kind.as_str(),
                    "table" | "table data" | "collection" | "time series"
                )
            })
            .map(|o| o.name.as_str())
            .collect();
        if let Some(bad) = sel.iter().find(|n| !known.contains(&n.as_str())) {
            return Err(AppError::validation(format!(
                "The dump has no table or collection named {bad}."
            )));
        }
    }
    let dump_name = |s: &str, t: &str| match kind {
        Kind::Mongo | Kind::My => t.to_string(),
        _ if s.is_empty() => t.to_string(),
        _ => format!("{s}.{t}"),
    };
    let wanted: Vec<String> = p
        .dump
        .tables
        .iter()
        .filter(|(db, _)| kind != Kind::Mongo || source_db.as_deref().is_none_or(|s| s == db))
        .filter(|(s, t)| chosen.as_ref().is_none_or(|c| c.contains(&dump_name(s, t))))
        .map(|(s, t)| match kind {
            Kind::Mongo | Kind::My => t.clone(),
            _ => format!("{}.{}", if s.is_empty() { "public" } else { s }, t),
        })
        .collect();
    let overlap: Vec<String> = wanted
        .iter()
        .filter(|w| have_lc.contains(&w.to_ascii_lowercase()))
        .cloned()
        .collect();
    let (mut replaces, mut clashes) = (Vec::new(), Vec::new());
    match opts.existing {
        Existing::Stop => clashes = overlap,
        Existing::Replace => replaces = overlap,
        Existing::Append => {
            let missing: Vec<&String> = wanted
                .iter()
                .filter(|w| !have_lc.contains(&w.to_ascii_lowercase()))
                .collect();
            if !missing.is_empty() && !new_db {
                warnings.push(format!(
                    "{} tables aren't in {target_db} yet ({}); their rows will fail.",
                    missing.len(),
                    missing
                        .iter()
                        .take(5)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    // Extensions the server can't install.
    if engine == Engine::Postgres {
        let exts: Vec<String> = d
            .objects
            .iter()
            .filter(|o| o.kind == "extension")
            .map(|o| o.name.clone())
            .collect();
        if let (false, Conn::Pg(c)) = (exts.is_empty(), &p.conn) {
            let avail: Vec<String> = c
                .client
                .query("select name::text from pg_available_extensions", &[])
                .await
                .map(|rows| rows.iter().map(|r| r.get(0)).collect())
                .unwrap_or_default();
            let missing: Vec<String> = exts.into_iter().filter(|e| !avail.contains(e)).collect();
            if !missing.is_empty() {
                warnings.push(format!(
                    "The server can't install the extensions {}; objects that need them will fail.",
                    missing.join(", ")
                ));
            }
        }
    }
    // How psql will read the file depends on the server's standard_conforming_strings; MySQL's client on its sql_mode.
    let mut server_scs = true;
    match &p.conn {
        Conn::Pg(c) if kind == Kind::PgPlain => {
            server_scs = c
                .client
                .query_one("show standard_conforming_strings", &[])
                .await
                .map(|r| r.get::<_, String>(0) != "off")
                .unwrap_or(true);
        }
        Conn::My(m) => {
            let mode = m
                .conn
                .query("select @@GLOBAL.sql_mode", 0)
                .await
                .ok()
                .and_then(|r| r.rows.into_iter().next())
                .and_then(|r| r.into_iter().next().flatten())
                .unwrap_or_default();
            if mode.to_ascii_uppercase().contains("NO_BACKSLASH_ESCAPES") {
                return Err(AppError::new(ErrorCode::DbDumpUnsafe, "This server runs with NO_BACKSLASH_ESCAPES, which changes how the mysql client reads a dump. Dalan won't restore a script into it."));
            }
        }
        _ => {}
    }
    let mut my_flags = Vec::new();
    let mariadb_client = kind == Kind::My && tool_version.contains("MariaDB");
    if kind == Kind::My {
        my_flags.push("--local-infile=0".to_string());
        if !opts.single_transaction {
            my_flags.push("--force".to_string());
        }
        if supports(ssh, &tool, "--system-command").await {
            my_flags.push("--system-command=OFF".to_string());
        }
        if mariadb_client && supports(ssh, &tool, "--sandbox").await {
            my_flags.push("--sandbox".to_string());
        }
    }
    // What is left out.
    if opts.skip_owner_acl
        && d.notes
            .iter()
            .any(|n| n.code == "owners" || n.code == "grants")
    {
        skipped.push(
            "Ownership and privileges (everything belongs to the user you sign in as)".to_string(),
        );
    }
    if d.notes.iter().any(|n| n.code == "roles") && !opts.include_roles {
        skipped.push("Roles and tablespaces".to_string());
    }
    if d.notes.iter().any(|n| n.code == "superuser") {
        skipped.push("Statements that make a superuser (never restored)".to_string());
    }
    if d.notes.iter().any(|n| n.code == "definer") {
        skipped.push("DEFINER clauses".to_string());
    }
    if d.notes
        .iter()
        .any(|n| n.code == "connect" || n.code == "databases")
    {
        skipped.push(format!(
            "CREATE DATABASE, \\connect and USE lines (everything goes into {target_db})"
        ));
    }
    if d.notes.iter().any(|n| n.code == "gtid") {
        skipped.push("GTID_PURGED and SQL_LOG_BIN".to_string());
    }
    if d.mariadb && !mariadb_client {
        skipped.push("MariaDB's sandbox line (MySQL's client can't read it)".to_string());
    }
    let atomic = matches!(kind, Kind::PgArchive | Kind::PgPlain) && opts.single_transaction;
    if kind == Kind::My {
        warnings.push(if new_db {
            "MySQL and MariaDB can't undo table changes: a failure leaves what ran so far. Into a new database, Dalan removes it again.".to_string()
        } else {
            "MySQL and MariaDB can't undo table changes: a failure leaves what ran so far. Restoring into a new database is safer.".to_string()
        });
    }
    let class = if new_db && !production {
        crate::sql::classify::Class::Schema
    } else {
        crate::sql::classify::Class::Dangerous
    };
    let level = match crate::dbs::gate::decide(p.mode, class) {
        crate::dbs::gate::Decision::Typed => "typed",
        _ => "confirm",
    };
    let typed_name = if new_db {
        target_db.clone()
    } else {
        p.saved_name.clone()
    };
    let picked = |o: &&dumpfile::DumpObject| chosen.as_ref().is_none_or(|c| c.contains(&o.name));
    let rows: f64 = d.objects.iter().filter(picked).filter_map(|o| o.rows).sum();
    let blocked = (!clashes.is_empty()).then(|| {
        format!(
            "{} of the dump's tables already exist in {target_db}: {}. Choose “Drop and replace” or “Add rows”, or restore into a new database.",
            clashes.len(),
            clashes.iter().take(6).map(String::as_str).collect::<Vec<_>>().join(", ")
        )
    });
    let list = match (&chosen, kind) {
        (Some(c), Kind::PgArchive) if !p.dump.toc.is_empty() => Some(toc_list(&p.dump.toc, c)),
        (Some(_), Kind::PgArchive) => {
            return Err(AppError::validation(
                "This archive's contents couldn't be listed, so it can only be restored whole.",
            ))
        }
        _ => None,
    };
    if rename.is_some() && kind == Kind::PgArchive {
        warnings.push("The schema is renamed after pg_restore ends: if the rename fails, the objects stay under their old schema.".to_string());
    }
    if opts.disable_triggers {
        warnings.push(
            "Turning triggers off needs a superuser; the restore fails without one.".to_string(),
        );
    }
    let plan = RestorePlan {
        pending_id: None,
        blocked,
        level: level.into(),
        typed_name: typed_name.clone(),
        saved_name: p.saved_name.clone(),
        environment: p.environment.clone(),
        target_db: target_db.clone(),
        new_database: new_db,
        engine,
        tool: tool.clone(),
        tool_version,
        runs_on: backup::runs_on(ssh),
        replaces,
        clashes,
        skipped,
        warnings,
        objects: match &chosen {
            Some(c) => c.len() as f64,
            None => d.objects.len() as f64 + d.objects_more,
        },
        rows,
        bytes: d.bytes,
        upload: d.format.is_folder() && ssh.is_some(),
        atomic,
    };
    let job = Job {
        dump_id: p.dump_id,
        opts,
        kind,
        target_db,
        new_database: new_db,
        tool,
        mariadb_client,
        my_flags,
        server_scs,
        source_db,
        typed_name,
        atomic,
        encoding: d.encoding.clone(),
        list,
        rename,
        as_file: jobs > 1 && d.format == DumpFormat::PgCustom,
    };
    Ok((plan, job))
}

/// The TOC entries of an archive to restore when only some tables are chosen: everything except the other tables and
/// whatever depends on them (their data, keys, indexes, triggers, comments, grants, and views over them).
pub fn toc_list(toc: &[dumpfile::TocEntry], keep: &[String]) -> Vec<i64> {
    let name = |e: &dumpfile::TocEntry| {
        if e.namespace.is_empty() {
            e.tag.clone()
        } else {
            format!("{}.{}", e.namespace, e.tag)
        }
    };
    let mut out: std::collections::HashSet<i64> = toc
        .iter()
        .filter(|e| matches!(e.desc.as_str(), "TABLE" | "TABLE DATA") && !keep.contains(&name(e)))
        .map(|e| e.id)
        .collect();
    // Whatever depends on something left out is left out too, to the end of the chain.
    loop {
        let more: Vec<i64> = toc
            .iter()
            .filter(|e| !out.contains(&e.id) && e.deps.iter().any(|d| out.contains(d)))
            .map(|e| e.id)
            .collect();
        if more.is_empty() {
            break;
        }
        out.extend(more);
    }
    toc.iter()
        .map(|e| e.id)
        .filter(|id| !out.contains(id))
        .collect()
}

/// `pg_restore --list` where the tool runs, with the archive on its input: the contents of an archive newer than
/// Dalan's reader. Only for single-file archives; the output is a list of names, never row data.
pub async fn list_archive(ssh: Option<&Connected>, dump: &Dump) -> AppResult<String> {
    let mut r = dumpfile::open(&dump.path, dump.outer).await?;
    let mut out = Vec::new();
    match ssh {
        Some(s) => {
            let ch = s.handle.channel_open_session().await.map_err(|e| {
                AppError::new(
                    ErrorCode::HostUnreachable,
                    format!("Couldn't open a channel on {}: {e}", s.label),
                )
            })?;
            let (mut reader, writer) = ch.split();
            writer
                .exec(true, "pg_restore --list")
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            let feeder = tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                while let Ok(n) = r.read(&mut buf).await {
                    if n == 0
                        || writer
                            .data_bytes(bytes::Bytes::copy_from_slice(&buf[..n]))
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                let _ = writer.eof().await;
                writer
            });
            while let Some(m) = reader.wait().await {
                match m {
                    ChannelMsg::Data { data } if out.len() < 64 << 20 => {
                        out.extend_from_slice(&data)
                    }
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            feeder.abort();
        }
        None => {
            let mut child = tokio::process::Command::new(backup::local_tool("pg_restore"))
                .arg("--list")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| backup::missing("pg_restore", "this computer"))?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| AppError::internal("no input"))?;
            let feeder = tokio::spawn(async move {
                let _ = tokio::io::copy(&mut r, &mut stdin).await;
            });
            let mut so = child
                .stdout
                .take()
                .ok_or_else(|| AppError::internal("no output"))?;
            (&mut so)
                .take(64 << 20)
                .read_to_end(&mut out)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            feeder.abort();
            let _ = child.wait().await;
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The gate's class for a held job.
pub fn class_of(job: &Job, environment: &str) -> crate::sql::classify::Class {
    if job.new_database && environment != "production" {
        crate::sql::classify::Class::Schema
    } else {
        crate::sql::classify::Class::Dangerous
    }
}

// ---------------------------------------------------------------- the new database

pub async fn create_database(conn: &Conn<'_>, name: &str, encoding: Option<&str>) -> AppResult<()> {
    match conn {
        Conn::Pg(c) => {
            let enc = encoding.filter(|e| {
                !e.eq_ignore_ascii_case("UTF8")
                    && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            let sql = match enc {
                Some(e) => format!(
                    "create database {} template template0 encoding '{e}'",
                    super::pg::quote(name)
                ),
                None => format!("create database {}", super::pg::quote(name)),
            };
            c.client
                .batch_execute(&sql)
                .await
                .map_err(|e| super::pg::map_error(&e))
        }
        Conn::My(m) => m
            .conn
            .query(&format!("create database {}", super::mydb::q(name)), 0)
            .await
            .map(|_| ()),
        // MongoDB makes a database when the first collection goes in.
        Conn::Mongo(_) => Ok(()),
    }
}

pub(crate) async fn drop_database(conn: &Conn<'_>, name: &str) -> bool {
    match conn {
        Conn::Pg(c) => c
            .client
            .batch_execute(&format!(
                "drop database if exists {}",
                super::pg::quote(name)
            ))
            .await
            .is_ok(),
        Conn::My(m) => m
            .conn
            .query(
                &format!("drop database if exists {}", super::mydb::q(name)),
                0,
            )
            .await
            .is_ok(),
        Conn::Mongo(m) => m
            .command(name, bson::doc! { "dropDatabase": 1 })
            .await
            .is_ok(),
    }
}

// ---------------------------------------------------------------- arguments

fn pg_conn_args(cfg: &SqlConfig, db: &str) -> Vec<String> {
    vec![
        "-h".into(),
        cfg.address.clone(),
        "-p".into(),
        cfg.port.to_string(),
        "-U".into(),
        cfg.user.clone(),
        "-d".into(),
        db.into(),
        "-w".into(),
    ]
}

/// The tool's arguments. `folder`: where a directory dump is, on the machine the tool runs on.
fn args(job: &Job, cfg: &SqlConfig, folder: Option<&str>, mongo_gzip: bool) -> Vec<String> {
    let o = &job.opts;
    let mut a: Vec<String> = Vec::new();
    match job.kind {
        Kind::PgArchive => {
            a.push("--verbose".into());
            if o.skip_owner_acl {
                a.extend(["--no-owner".into(), "--no-privileges".into()]);
            }
            match (o.existing, o.content) {
                (Existing::Append, _) | (_, Content::Data) => a.push("--data-only".into()),
                (_, Content::Schema) => a.push("--schema-only".into()),
                _ => {}
            }
            if o.existing == Existing::Replace && o.content != Content::Data {
                a.extend(["--clean".into(), "--if-exists".into()]);
            }
            if o.single_transaction {
                a.extend(["--single-transaction".into(), "--exit-on-error".into()]);
            }
            if o.jobs > 1 {
                a.push(format!("--jobs={}", o.jobs));
            }
            if o.disable_triggers {
                a.push("--disable-triggers".into());
            }
            if job.list.is_some() {
                // Replaced by the list file's path where the tool runs (see `remote_command` and `run`).
                a.extend(["-L".into(), LIST.into()]);
            }
            a.extend(pg_conn_args(cfg, &job.target_db));
            match folder {
                Some(f) if job.as_file => a.extend(["-Fc".into(), f.into()]),
                Some(f) => a.extend(["-Fd".into(), f.into()]),
                None => {}
            }
        }
        Kind::PgPlain => {
            // -X: no ~/.psqlrc, whose commands would run with the dump's.
            a.extend([
                "-X".into(),
                "-q".into(),
                "-v".into(),
                format!("ON_ERROR_STOP={}", u8::from(o.single_transaction)),
            ]);
            if o.single_transaction {
                a.push("--single-transaction".into());
            }
            a.extend(pg_conn_args(cfg, &job.target_db));
        }
        Kind::My => {
            a.extend(job.my_flags.iter().cloned());
            a.extend([
                "--protocol=tcp".into(),
                "-h".into(),
                cfg.address.clone(),
                "-P".into(),
                cfg.port.to_string(),
                "-u".into(),
                cfg.user.clone(),
                job.target_db.clone(),
            ]);
        }
        Kind::Mongo => {
            let c = SqlConfig {
                database: None,
                password: None,
                ..clone_cfg(cfg)
            };
            a = backup::args_mongo(&c, backup::Tool::Restore)
                .into_iter()
                .filter(|x| x != "--archive" && x != "--gzip" && x != "--drop")
                .collect();
            match folder {
                Some(f) => {
                    a.push(format!("--dir={f}"));
                    if mongo_gzip {
                        a.push("--gzip".into());
                    }
                }
                None => a.push("--archive".into()),
            }
            if let Some(src) = &job.source_db {
                match o.objects.as_ref().filter(|x| !x.is_empty()) {
                    Some(colls) => a.extend(colls.iter().map(|c| format!("--nsInclude={src}.{c}"))),
                    None => a.push(format!("--nsInclude={src}.*")),
                }
                if *src != job.target_db {
                    a.extend([
                        format!("--nsFrom={src}.*"),
                        format!("--nsTo={}.*", job.target_db),
                    ]);
                }
            }
            if o.existing == Existing::Replace {
                a.push("--drop".into());
            }
            if o.single_transaction {
                a.push("--stopOnError".into());
            }
        }
    }
    a
}

fn clone_cfg(c: &SqlConfig) -> SqlConfig {
    SqlConfig {
        name: c.name.clone(),
        engine: c.engine,
        route: c.route,
        host_id: c.host_id.clone(),
        address: c.address.clone(),
        port: c.port,
        database: c.database.clone(),
        user: c.user.clone(),
        password: c.password.clone(),
        tls: c.tls,
        server_name: c.server_name.clone(),
        auth_source: c.auth_source.clone(),
    }
}

fn password_lines(job: &Job, cfg: &SqlConfig) -> AppResult<Vec<u8>> {
    match job.kind {
        Kind::PgArchive | Kind::PgPlain => backup::password_line(cfg),
        Kind::My => backup::config_lines(cfg),
        Kind::Mongo => backup::config_mongo(cfg),
    }
}

/// What `sh -c` runs on the host: first the process group's id on stderr (so Stop can end everything the command
/// started), then the engine's wrapper from `backup.rs`.
fn remote_command(job: &Job, a: &[String]) -> String {
    let mut inner = match job.kind {
        Kind::PgArchive | Kind::PgPlain => backup::wrapper(&job.tool, a),
        Kind::My => backup::wrapper_my(&job.tool, a),
        Kind::Mongo => backup::wrapper_mongo(&job.tool, a),
    };
    if let Some(ids) = &job.list {
        // The entry list goes in a private file that is removed when the shell ends (so no `exec` here). The ids are
        // numbers Dalan wrote, nothing from the dump.
        let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
        inner = format!(
            "umask 077; L=$(mktemp) || exit 1; trap 'rm -f \"$L\"' EXIT HUP INT TERM; printf '%s\\n' {} > \"$L\"; {}",
            ids.join(" "),
            inner.replacen("exec ", "", 1).replace(&shq(LIST), "\"$L\"")
        );
    }
    format!(
        "sh -c {}",
        shq(&format!(
            "echo \"dalan-pgid $(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ') $$\" >&2; {inner}"
        ))
    )
}

/// Stands in for the `-L` list file's path in the arguments.
const LIST: &str = "@DALAN_LIST@";

// ---------------------------------------------------------------- the stream into the tool

/// Reads the dump (counting file bytes), passes plain SQL through the scanner, and sends chunks to the tool's input.
/// A refusal on the way (the file changed since it was checked) ends the stream with an error.
// Each argument is one input of the stream; a struct for one caller would only rename them.
#[allow(clippy::too_many_arguments)]
async fn produce(
    stdin_dump: bool,
    tail: Option<Vec<u8>>,
    dump: Arc<Dump>,
    rewrite: Option<(Dialect, Rewrite, bool)>,
    password: Vec<u8>,
    tx: mpsc::Sender<Result<Vec<u8>, String>>,
    read: Arc<AtomicU64>,
    current: Arc<Mutex<Option<String>>>,
) {
    if tx.send(Ok(password)).await.is_err() {
        return;
    }
    if !stdin_dump {
        return;
    }
    let mut r = match dumpfile::open_counted(&dump.path, dump.outer, Some(read)).await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(e.message)).await;
            return;
        }
    };
    let mut scanner = rewrite.map(|(d, rw, scs)| dumpscan::Scanner::new(d, Some(rw), scs));
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = match r.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(Err(format!("Couldn't read the dump: {e}"))).await;
                return;
            }
        };
        let chunk = match scanner.as_mut() {
            None => buf[..n].to_vec(),
            Some(s) => {
                let mut out = Vec::with_capacity(n + 1024);
                if let Err(e) = s.feed(&buf[..n], &mut out) {
                    let _ = tx.send(Err(e.message())).await;
                    return;
                }
                if let Ok(mut c) = current.lock() {
                    *c = s.current();
                }
                out
            }
        };
        if !chunk.is_empty() && tx.send(Ok(chunk)).await.is_err() {
            return;
        }
    }
    if let Some(s) = scanner.as_mut() {
        let mut out = Vec::new();
        if let Err(e) = s.finish(&mut out) {
            let _ = tx.send(Err(e.message())).await;
            return;
        }
        if !out.is_empty() {
            let _ = tx.send(Ok(out)).await;
        }
    }
    // A schema renamed in the same transaction, after everything else.
    if let Some(t) = tail {
        let _ = tx.send(Ok(t)).await;
    }
}

// ---------------------------------------------------------------- the tool's output

/// Reads the tool's stdout and stderr lines: the current object, objects done, and warnings (deduplicated).
#[derive(Default)]
struct Report {
    done: Vec<String>,
    objects: u64,
    current: Option<String>,
    warnings: Vec<(String, u64)>,
    last_error: Option<String>,
    pgid: Option<String>,
    pid: Option<String>,
    partial_out: Vec<u8>,
    partial_err: Vec<u8>,
}

impl Report {
    fn take_lines(buf: &mut Vec<u8>, data: &[u8]) -> Vec<String> {
        buf.extend_from_slice(data);
        let mut lines = Vec::new();
        while let Some(p) = buf.iter().position(|&b| b == b'\n') {
            let l: Vec<u8> = buf.drain(..=p).collect();
            lines.push(String::from_utf8_lossy(&l).trim_end().to_string());
        }
        if buf.len() > 64 * 1024 {
            buf.clear();
        }
        lines
    }

    fn warn(&mut self, text: String) {
        // The quoted statement after `near '` can be row data: keep the words before it.
        let text = text
            .split(" near '")
            .next()
            .unwrap_or(&text)
            .trim()
            .to_string();
        if text.is_empty() {
            return;
        }
        self.last_error = Some(text.clone());
        if let Some(w) = self.warnings.iter_mut().find(|(t, _)| *t == text) {
            w.1 += 1;
        } else if self.warnings.len() < 50 {
            self.warnings.push((text, 1));
        }
    }

    fn object(&mut self, what: String) {
        self.objects += 1;
        if self.done.len() < 200 {
            self.done.push(what);
        }
    }

    fn stdout(&mut self, kind: Kind, data: &[u8]) {
        let mut buf = std::mem::take(&mut self.partial_out);
        for l in Self::take_lines(&mut buf, data) {
            // psql prints a command tag per statement: CREATE TABLE, COPY 4, ALTER TABLE …
            if kind == Kind::PgPlain && l.starts_with("CREATE ") {
                self.objects += 1;
            }
        }
        self.partial_out = buf;
    }

    fn stderr(&mut self, kind: Kind, data: &[u8]) {
        let mut buf = std::mem::take(&mut self.partial_err);
        for l in Self::take_lines(&mut buf, data) {
            if let Some(rest) = l.strip_prefix("dalan-pgid ") {
                let mut it = rest.split_whitespace();
                let (a, b) = (it.next().map(str::to_string), it.next().map(str::to_string));
                // With ps: "pgid pid"; without: "pid".
                (self.pgid, self.pid) = match b {
                    Some(b) => (a, Some(b)),
                    None => (None, a),
                };
                continue;
            }
            match kind {
                Kind::PgArchive => {
                    // Lines that don't start with the tool's name continue a quoted statement: never shown.
                    let Some(rest) = l.strip_prefix("pg_restore: ") else {
                        continue;
                    };
                    if let Some(w) = rest
                        .strip_prefix("error: ")
                        .or_else(|| rest.strip_prefix("warning: "))
                    {
                        let w = w.strip_prefix("could not execute query: ").unwrap_or(w);
                        self.warn(w.replace("  ", " "));
                    } else if let Some(obj) = rest.strip_prefix("creating ") {
                        let obj = obj.replace('"', "");
                        self.current = Some(obj.to_ascii_lowercase());
                        let mut parts = obj.splitn(2, ' ');
                        let (k, n) = (
                            parts.next().unwrap_or("").to_ascii_lowercase(),
                            parts.next().unwrap_or(""),
                        );
                        self.object(format!("{k} {n}"));
                    } else if let Some(t) = rest.strip_prefix("processing data for table ") {
                        self.current = Some(format!("rows of {}", t.replace('"', "")));
                    }
                }
                Kind::PgPlain => {
                    // `psql:<stdin>:45: ERROR:  relation "t" already exists`; context lines (LINE 1: …) quote SQL.
                    let Some(rest) = l.strip_prefix("psql:") else {
                        continue;
                    };
                    let msg = rest.splitn(3, ':').nth(2).unwrap_or(rest).trim();
                    if msg.starts_with("ERROR")
                        || msg.starts_with("WARNING")
                        || msg.starts_with("FATAL")
                    {
                        self.warn(msg.replace("  ", " "));
                    }
                }
                Kind::My => {
                    if l.starts_with("ERROR") {
                        // `ERROR 1050 (42S01) at line 25: Table 't' already exists` → without the line number.
                        let msg = match (l.find(" at line "), l.find(": ")) {
                            (Some(a), Some(b)) if b > a => format!("{}{}", &l[..a], &l[b..]),
                            _ => l.clone(),
                        };
                        self.warn(msg);
                    } else if l.contains("[Warning]") && !l.contains("password on the command line")
                    {
                        self.warn(l.clone());
                    }
                }
                Kind::Mongo => {
                    // `2024-01-01T00:00:00.000+0000\trestoring shop.orders from archive`
                    let msg = l
                        .split_once('\t')
                        .map_or(l.as_str(), |(_, m)| m)
                        .trim()
                        .to_string();
                    if let Some(ns) = msg
                        .strip_prefix("restoring ")
                        .and_then(|m| m.split(" from ").next())
                    {
                        if !ns.starts_with("indexes") && !ns.starts_with("users") {
                            self.current = Some(ns.to_string());
                        }
                    } else if let Some(rest) = msg.strip_prefix("finished restoring ") {
                        let ns = rest.split(" (").next().unwrap_or(rest).to_string();
                        if let Some(f) = rest
                            .split(", ")
                            .nth(1)
                            .and_then(|x| x.split(' ').next())
                            .and_then(|n| n.parse::<u64>().ok())
                        {
                            if f > 0 {
                                self.warn(format!("{ns}: {f} documents failed"));
                            }
                        }
                        self.object(format!("collection {ns}"));
                    } else if msg.contains("error")
                        || msg.contains("Failed")
                        || msg.starts_with("failed")
                    {
                        self.warn(msg);
                    }
                }
            }
        }
        self.partial_err = buf;
    }
}

// ---------------------------------------------------------------- running

pub struct RunInput<'a> {
    pub job: &'a Job,
    pub cfg: &'a SqlConfig,
    pub ssh: Option<Arc<Connected>>,
    pub dump: Arc<Dump>,
    pub stop: Arc<AtomicBool>,
    pub progress: &'a (dyn Fn(RestoreProgress) + Sync),
}

/// A custom archive for parallel jobs, as a file on the host: uploaded (unwrapped from any compression) as
/// `<private folder>/dump`. Returns the folder.
async fn upload_file(
    ssh: &Connected,
    dump: &Dump,
    stop: &AtomicBool,
    progress: &(dyn Fn(RestoreProgress) + Sync),
) -> AppResult<String> {
    let tmp = dumpfile::TempDir::new("upload")?;
    let file = tmp.0.join("dump");
    // Unwrapped here first when compressed, so the host gets the archive pg_restore reads.
    if dump.outer.is_some() {
        let mut r = dumpfile::open(&dump.path, dump.outer).await?;
        let mut f = tokio::fs::File::create(&file)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        tokio::io::copy(&mut r, &mut f)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    } else {
        // ponytail: a local copy before the upload (twice the disk for a moment); stream it straight to SFTP if big
        // archives make that hurt.
        tokio::fs::copy(&dump.path, &file)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    }
    let size = std::fs::metadata(&file)
        .map(|m| m.len())
        .unwrap_or(dump.info.bytes as u64);
    upload(ssh, &tmp.0, size, stop, progress).await
}

/// Here, with parallel jobs: a compressed custom archive is unwrapped into a private folder first.
async fn unpack_local(dump: &Dump) -> AppResult<dumpfile::TempDir> {
    let tmp = dumpfile::TempDir::new("restore")?;
    let mut r = dumpfile::open(&dump.path, dump.outer).await?;
    let mut f = tokio::fs::File::create(tmp.0.join("dump"))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    tokio::io::copy(&mut r, &mut f)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(tmp)
}

/// Uploads a folder to a fresh private folder on the host and returns its path.
async fn upload(
    ssh: &Connected,
    folder: &Path,
    total: u64,
    stop: &AtomicBool,
    progress: &(dyn Fn(RestoreProgress) + Sync),
) -> AppResult<String> {
    let (status, out) = backup::exec_collect(ssh, "umask 077; mktemp -d").await?;
    let dir = out.trim().to_string();
    if status != Some(0) || !dir.starts_with('/') || dir.contains('\n') {
        return Err(AppError::internal(format!(
            "Couldn't make a temporary folder on {}.",
            ssh.label
        )));
    }
    let space = backup::exec_collect(ssh, &format!("df -Pk {}", shq(&dir)))
        .await?
        .1;
    let avail_kb = space
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|n| n.parse::<u64>().ok());
    if let Some(kb) = avail_kb {
        if kb.saturating_mul(1024) < total {
            cleanup(ssh, &dir).await;
            return Err(AppError::new(ErrorCode::DbHostNoSpace, format!("{} has {} MB free where the dump would go, and the dump is {} MB. Free some space there, then try again.", ssh.label, kb / 1024, total / (1 << 20))));
        }
    }
    let result = async {
        let ch = ssh.handle.channel_open_session().await.map_err(|e| {
            AppError::new(
                ErrorCode::HostUnreachable,
                format!("Couldn't open a channel on {}: {e}", ssh.label),
            )
        })?;
        ch.request_subsystem(true, "sftp").await.map_err(|_| {
            AppError::new(
                ErrorCode::SftpDisabled,
                format!(
                    "SFTP is turned off on {}, so the dump folder can't be copied there.",
                    ssh.label
                ),
            )
        })?;
        let sftp = russh_sftp::client::SftpSession::new(ch.into_stream())
            .await
            .map_err(|e| AppError::internal(format!("{} didn't start SFTP: {e}", ssh.label)))?;
        let mut sent = 0u64;
        let mut stack = vec![(folder.to_path_buf(), dir.clone())];
        while let Some((local, remote)) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&local)
                .map_err(|e| AppError::validation(format!("Couldn't read the folder: {e}")))?
                .flatten()
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if stop.load(Ordering::SeqCst) {
                    return Err(AppError::new(ErrorCode::Internal, "Stopped."));
                }
                let name = e.file_name().to_string_lossy().into_owned();
                if name.contains('/') || name == ".." {
                    continue;
                }
                let r = format!("{remote}/{name}");
                let ft = e
                    .file_type()
                    .map_err(|e| AppError::validation(e.to_string()))?;
                if ft.is_dir() {
                    sftp.create_dir(r.clone()).await.map_err(|e| {
                        AppError::internal(format!("Couldn't make a folder on {}: {e}", ssh.label))
                    })?;
                    stack.push((e.path(), r));
                } else if ft.is_file() {
                    let mut src = tokio::fs::File::open(e.path())
                        .await
                        .map_err(|e| AppError::validation(e.to_string()))?;
                    let mut dst = sftp.create(r).await.map_err(|e| {
                        AppError::internal(format!("Couldn't write on {}: {e}", ssh.label))
                    })?;
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        let n = src
                            .read(&mut buf)
                            .await
                            .map_err(|e| AppError::validation(e.to_string()))?;
                        if n == 0 {
                            break;
                        }
                        dst.write_all(&buf[..n]).await.map_err(|e| {
                            AppError::internal(format!("Couldn't write on {}: {e}", ssh.label))
                        })?;
                        sent += n as u64;
                        progress(RestoreProgress {
                            phase: "upload".into(),
                            bytes: sent as f64,
                            total: total as f64,
                            current: Some(name.clone()),
                            objects: 0.0,
                        });
                    }
                    dst.shutdown().await.map_err(|e| {
                        AppError::internal(format!("Couldn't write on {}: {e}", ssh.label))
                    })?;
                }
            }
        }
        let _ = sftp.close().await;
        Ok::<(), AppError>(())
    }
    .await;
    match result {
        Ok(()) => Ok(dir),
        Err(e) => {
            cleanup(ssh, &dir).await;
            Err(e)
        }
    }
}

async fn cleanup(ssh: &Connected, dir: &str) {
    // Only a folder mktemp made (an absolute path, checked on the way in).
    if dir.starts_with('/') && dir.len() > 5 {
        let _ = backup::exec_collect(ssh, &format!("rm -rf -- {}", shq(dir))).await;
    }
}

/// Runs the restore. Never returns `Err` for the tool's own failure or a Stop: those are outcomes for the result
/// screen. `Err` is for what stopped it before the tool ran.
pub async fn run(r: RunInput<'_>) -> AppResult<(String, Option<String>, Report2)> {
    let job = r.job;
    let dump = Arc::clone(&r.dump);
    let started = Instant::now();
    let total = dump.info.bytes.max(1.0);
    let ssh = r.ssh.clone();
    // Directory dumps: on the host, uploaded first; here, used where they are.
    let mut remote_dir: Option<String> = None;
    // Holds an unpacked archive until the tool is done with it.
    let mut _local_tmp: Option<dumpfile::TempDir> = None;
    let folder_arg: Option<String> = match (&dump.folder, ssh.as_deref()) {
        (Some(f), Some(s)) if dump.info.format.is_folder() => {
            let d = upload(s, f, dump.info.bytes as u64, &r.stop, r.progress).await?;
            remote_dir = Some(d.clone());
            Some(d)
        }
        (Some(f), None) if dump.info.format.is_folder() => Some(f.display().to_string()),
        _ if job.as_file => match ssh.as_deref() {
            Some(s) => {
                let d = upload_file(s, &dump, &r.stop, r.progress).await?;
                remote_dir = Some(d.clone());
                Some(format!("{d}/dump"))
            }
            None if dump.outer.is_none() => Some(dump.path.display().to_string()),
            None => {
                let t = unpack_local(&dump).await?;
                let p = t.0.join("dump").display().to_string();
                _local_tmp = Some(t);
                Some(p)
            }
        },
        _ => None,
    };
    let a = args(job, r.cfg, folder_arg.as_deref(), dump.mongo_gzip);
    let pw = password_lines(job, r.cfg)?;
    let rewrite = match job.kind {
        Kind::PgPlain | Kind::My => Some((
            if job.kind == Kind::My {
                Dialect::My
            } else {
                Dialect::Pg
            },
            Rewrite {
                section: job
                    .source_db
                    .clone()
                    .filter(|_| dump.info.databases.len() > 1),
                skip_owner_acl: job.opts.skip_owner_acl,
                include_roles: job.opts.include_roles,
                existing: job.opts.existing,
                content: job.opts.content,
                strip_definer: true,
                keep_sandbox_line: job.mariadb_client,
                tables: job.opts.objects.clone().filter(|o| !o.is_empty()),
            },
            job.server_scs,
        )),
        _ => None,
    };
    let read = Arc::new(AtomicU64::new(0));
    let current = Arc::new(Mutex::new(None));
    let (tx, mut rx) = mpsc::channel::<Result<Vec<u8>, String>>(8);
    let tail = match (&job.rename, job.kind) {
        (Some((from, to)), Kind::PgPlain) => Some(
            format!(
                "\nALTER SCHEMA {} RENAME TO {};\n",
                super::pg::quote(from),
                super::pg::quote(to)
            )
            .into_bytes(),
        ),
        _ => None,
    };
    let producer = tokio::spawn(produce(
        !dump.info.format.is_folder() && !job.as_file,
        tail,
        Arc::clone(&dump),
        rewrite,
        pw.clone(),
        tx,
        Arc::clone(&read),
        Arc::clone(&current),
    ));
    let mut rep = Report::default();
    let mut refusal: Option<String> = None;
    let mut stopped = false;
    let mut status: Option<i32> = None;
    let show = |rep: &Report, phase_bytes: f64| {
        let cur = rep
            .current
            .clone()
            .or_else(|| current.lock().ok().and_then(|c| c.clone()));
        (r.progress)(RestoreProgress {
            phase: "restore".into(),
            bytes: phase_bytes,
            total,
            current: cur,
            objects: rep.objects as f64,
        });
    };
    if let Some(ssh) = ssh.as_deref() {
        let ch = ssh.handle.channel_open_session().await.map_err(|e| {
            AppError::new(
                ErrorCode::HostUnreachable,
                format!("Couldn't open a channel on the host: {e}"),
            )
        })?;
        let (mut reader, writer) = ch.split();
        writer
            .exec(true, remote_command(job, &a))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        // The feeder is its own task: the tool's output must be read meanwhile, or both sides would wait.
        let (ref_tx, mut ref_rx) = mpsc::channel::<String>(1);
        let feeder = tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                match m {
                    Ok(chunk) => {
                        if writer.data_bytes(bytes::Bytes::from(chunk)).await.is_err() {
                            return writer;
                        }
                    }
                    Err(e) => {
                        let _ = ref_tx.send(e).await;
                        // Not `eof`, and the channel stays open: the tool must not take a cut-off file as a finished
                        // one. It is killed, then this task is aborted (which drops the writer).
                        std::future::pending::<()>().await;
                    }
                }
            }
            let _ = writer.eof().await;
            writer
        });
        let mut last = Instant::now();
        loop {
            if let Ok(e) = ref_rx.try_recv() {
                refusal = Some(e);
            }
            if r.stop.load(Ordering::SeqCst) || refusal.is_some() {
                stopped = refusal.is_none();
                kill_remote(ssh, &rep).await;
                break;
            }
            let m = match tokio::time::timeout(Duration::from_millis(250), reader.wait()).await {
                Err(_) => continue,
                Ok(None) => break,
                Ok(Some(m)) => m,
            };
            match m {
                ChannelMsg::Data { data } => rep.stdout(job.kind, &data),
                ChannelMsg::ExtendedData { data, .. } => rep.stderr(job.kind, &data),
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status as i32),
                ChannelMsg::Close => break,
                _ => {}
            }
            if last.elapsed() > Duration::from_millis(200) {
                last = Instant::now();
                show(&rep, read.load(Ordering::Relaxed) as f64);
            }
        }
        feeder.abort();
        if let Some(d) = &remote_dir {
            cleanup(ssh, d).await;
        }
        if status == Some(127) {
            producer.abort();
            return Err(backup::missing(&job.tool, &ssh.label));
        }
    } else {
        // This computer: MySQL's and MongoDB's password in a private file named first; PostgreSQL's in the environment.
        let cfg_file = match job.kind {
            Kind::My | Kind::Mongo => Some(backup::LocalConfig::create(&pw)?),
            _ => None,
        };
        let mut full: Vec<String> = cfg_file
            .iter()
            .map(|c| {
                format!(
                    "{}={}",
                    if job.kind == Kind::Mongo {
                        "--config"
                    } else {
                        "--defaults-file"
                    },
                    c.0.display()
                )
            })
            .collect();
        // The entry list for `-L`, in a private file removed with `list_file`.
        let list_file = match &job.list {
            Some(ids) => Some(backup::LocalConfig::create(
                ids.iter()
                    .map(|i| format!("{i}\n"))
                    .collect::<String>()
                    .as_bytes(),
            )?),
            None => None,
        };
        full.extend(a.iter().map(|x| match (&list_file, x.as_str()) {
            (Some(l), LIST) => l.0.display().to_string(),
            _ => x.clone(),
        }));
        let mut cmd = tokio::process::Command::new(backup::local_tool(&job.tool));
        cmd.args(&full).kill_on_drop(true);
        if matches!(job.kind, Kind::PgArchive | Kind::PgPlain) {
            cmd.env(
                "PGPASSWORD",
                String::from_utf8_lossy(&pw).trim_end_matches('\n'),
            );
        }
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| backup::missing(&job.tool, "this computer"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::internal("no input"))?;
        let (ref_tx, mut ref_rx) = mpsc::channel::<String>(1);
        let feeder = tokio::spawn(async move {
            // The first message is the password line for the remote wrapper; here it went in the environment or a file.
            let _ = rx.recv().await;
            while let Some(m) = rx.recv().await {
                match m {
                    Ok(chunk) => {
                        if stdin.write_all(&chunk).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = ref_tx.send(e).await;
                        // The pipe stays open until the tool is killed and this task aborted: never a cut-off file
                        // that looks complete.
                        std::future::pending::<()>().await;
                    }
                }
            }
            let _ = stdin.shutdown().await;
        });
        let mut so = child
            .stdout
            .take()
            .ok_or_else(|| AppError::internal("no output"))?;
        let mut se = child
            .stderr
            .take()
            .ok_or_else(|| AppError::internal("no error output"))?;
        let (mut ob, mut eb) = (vec![0u8; 16 * 1024], vec![0u8; 16 * 1024]);
        let (mut out_done, mut err_done) = (false, false);
        let mut last = Instant::now();
        loop {
            if let Ok(e) = ref_rx.try_recv() {
                refusal = Some(e);
            }
            if r.stop.load(Ordering::SeqCst) || refusal.is_some() {
                stopped = refusal.is_none();
                let _ = child.kill().await;
                break;
            }
            if out_done && err_done {
                break;
            }
            tokio::select! {
                n = so.read(&mut ob), if !out_done => match n { Ok(0) | Err(_) => out_done = true, Ok(n) => rep.stdout(job.kind, &ob[..n]) },
                n = se.read(&mut eb), if !err_done => match n { Ok(0) | Err(_) => err_done = true, Ok(n) => rep.stderr(job.kind, &eb[..n]) },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            if last.elapsed() > Duration::from_millis(200) {
                last = Instant::now();
                show(&rep, read.load(Ordering::Relaxed) as f64);
            }
        }
        feeder.abort();
        status = if stopped || refusal.is_some() {
            None
        } else {
            child.wait().await.ok().and_then(|s| s.code())
        };
        drop(cfg_file);
    }
    producer.abort();
    let seconds = started.elapsed().as_secs_f64();
    let outcome = if refusal.is_some() {
        "failed"
    } else if stopped {
        "stopped"
    } else if status == Some(0) {
        "done"
    } else {
        "failed"
    };
    let message = match outcome {
        "failed" => Some(refusal.clone().unwrap_or_else(|| match &rep.last_error {
            Some(e) => format!("{} stopped: {e}", job.tool),
            None => format!("{} stopped with an error and gave no reason.", job.tool),
        })),
        _ => None,
    };
    show(&rep, read.load(Ordering::Relaxed) as f64);
    Ok((
        outcome.to_string(),
        message,
        Report2 {
            done: rep.done,
            objects: rep.objects,
            warnings: rep.warnings,
            seconds,
        },
    ))
}

/// The tool's report, for the result.
pub struct Report2 {
    pub done: Vec<String>,
    pub objects: u64,
    pub warnings: Vec<(String, u64)>,
    pub seconds: f64,
}

/// Ends the tool on the host: its whole process group when `ps` told us it, else the wrapper's process.
async fn kill_remote(ssh: &Connected, rep: &Report) {
    let digits = |s: &Option<String>| {
        s.as_ref()
            .filter(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
            .cloned()
    };
    let cmd = match (digits(&rep.pgid), digits(&rep.pid)) {
        (Some(g), _) => {
            format!("kill -TERM -- -{g} 2>/dev/null; sleep 1; kill -KILL -- -{g} 2>/dev/null; true")
        }
        (None, Some(p)) => {
            format!("pkill -TERM -P {p} 2>/dev/null; kill -TERM {p} 2>/dev/null; true")
        }
        _ => return,
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), backup::exec_collect(ssh, &cmd)).await;
}

/// After the tool: a new database made for a failed or stopped restore is removed again.
/// An archive restored into this database under a renamed schema: the rename runs once pg_restore is done.
/// Returns the server's refusal, if any, for the result's message.
pub async fn rename_after(conn: &Conn<'_>, job: &Job) -> Option<String> {
    let (Some((from, to)), Kind::PgArchive, false, Conn::Pg(c)) =
        (&job.rename, job.kind, job.new_database, conn)
    else {
        return None;
    };
    c.client
        .batch_execute(&format!(
            "alter schema {} rename to {}",
            super::pg::quote(from),
            super::pg::quote(to)
        ))
        .await
        .err()
        .map(|e| {
            format!(
                "Restored, but the schema couldn't be renamed: {}",
                super::pg::map_error(&e).message
            )
        })
}

pub async fn after(conn: &Conn<'_>, job: &Job, outcome: &str) -> bool {
    job.new_database && outcome != "done" && drop_database(conn, &job.target_db).await
}

pub fn result(
    job: &Job,
    outcome: String,
    message: Option<String>,
    rep: Report2,
    removed: bool,
) -> RestoreResult {
    RestoreResult {
        nothing_changed: outcome != "done" && job.atomic,
        outcome,
        message,
        target_db: job.target_db.clone(),
        objects: rep.objects as f64,
        done: rep.done,
        warnings: rep
            .warnings
            .into_iter()
            .map(|(text, count)| RestoreWarning {
                text,
                count: count as f64,
            })
            .collect(),
        seconds: rep.seconds,
        removed_new_database: removed,
    }
}

pub fn summary(file: &str, r: &RestoreResult) -> String {
    format!(
        "restore {file} into {}: {}, {} objects, {} warnings, {:.0} s",
        r.target_db,
        r.outcome,
        r.objects,
        r.warnings.iter().map(|w| w.count).sum::<f64>(),
        r.seconds
    )
}

pub fn encoding(job: &Job) -> Option<&str> {
    job.encoding.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_are_deduplicated_and_quoted_sql_never_shown() {
        let mut r = Report::default();
        r.stderr(Kind::PgArchive, b"dalan-pgid 4242 4250\npg_restore: creating TABLE \"public.t\"\npg_restore: error: could not execute query: ERROR:  role \"app_owner\" does not exist\nCommand was: ALTER TABLE public.t OWNER TO app_owner;\n  secret row data\npg_restore: error: could not execute query: ERROR:  role \"app_owner\" does not exist\n");
        assert_eq!(
            (r.pgid.as_deref(), r.pid.as_deref()),
            (Some("4242"), Some("4250"))
        );
        assert_eq!(
            r.warnings,
            vec![("ERROR: role \"app_owner\" does not exist".to_string(), 2)]
        );
        assert_eq!(r.done, vec!["table public.t".to_string()]);
        let mut p = Report::default();
        p.stderr(Kind::PgPlain, b"psql:<stdin>:45: ERROR:  relation \"t\" already exists\nLINE 1: CREATE TABLE t (secret text);\n        ^\npsql:<stdin>:99: NOTICE:  table \"x\" does not exist, skipping\npsql:<stdin>:46: ERROR:  relation \"t\" already exists\n");
        assert_eq!(
            p.warnings,
            vec![("ERROR: relation \"t\" already exists".to_string(), 2)]
        );
        let mut m = Report::default();
        m.stderr(Kind::My, b"mysql: [Warning] Using a password on the command line interface can be insecure.\nERROR 1064 (42000) at line 12: You have an error in your SQL syntax; check the manual near 'secret' at line 1\n");
        assert_eq!(m.warnings.len(), 1);
        assert!(
            !m.warnings[0].0.contains("secret") && !m.warnings[0].0.contains("line 12"),
            "{:?}",
            m.warnings
        );
        let mut g = Report::default();
        g.stderr(Kind::Mongo, b"2024-01-01T00:00:00.000+0000\trestoring shop.orders from archive\n2024-01-01T00:00:00.000+0000\tfinished restoring shop.orders (2 documents, 1 failure)\n");
        assert_eq!(g.done, vec!["collection shop.orders".to_string()]);
        assert_eq!(g.warnings[0].0, "shop.orders: 1 documents failed");
        // No pgid without ps: the pid alone.
        let mut n = Report::default();
        n.stderr(Kind::PgPlain, b"dalan-pgid  777\n");
        assert_eq!((n.pgid, n.pid.as_deref()), (None, Some("777")));
    }

    #[test]
    fn newer_archives_need_newer_tools() {
        assert!((1, 16) > pg_restore_reads(16));
        assert!((1, 15) <= pg_restore_reads(16));
        assert!((1, 14) <= pg_restore_reads(13));
        assert!((1, 16) <= pg_restore_reads(17));
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod docker_tests;

/// The server a dump is restored on for a compare or a backup check (its saved name, environment and write mode,
/// for the restore's own checks).
pub struct Scratch<'a> {
    pub name: &'a str,
    pub environment: &'a str,
    pub mode: crate::redis::store::RedisWriteMode,
}

/// Restores `dump` into a new scratch database `<prefix>_<id>` on a server that isn't production, hands its name to
/// `then`, and drops it again, also when the restore or `then` fails (D16 compare with a dump, D17 backup check).
#[allow(clippy::too_many_arguments)]
pub async fn with_scratch<T>(
    conn: Conn<'_>,
    mut cfg: SqlConfig,
    ssh: Option<Arc<Connected>>,
    dump: Arc<Dump>,
    dump_id: &str,
    server: Scratch<'_>,
    prefix: &str,
    content: crate::dbs::dumpscan::Content,
    then: impl AsyncFnOnce(String) -> AppResult<T>,
) -> AppResult<T> {
    if server.environment == "production" {
        return Err(AppError::validation(format!(
            "This needs a scratch database, and {} is production. Pick a tab of another server.",
            server.name
        )));
    }
    let scratch = format!(
        "{prefix}_{}",
        &uuid::Uuid::now_v7().simple().to_string()[24..]
    );
    let opts = RestoreOptions {
        new_database: Some(scratch.clone()),
        source_database: dump
            .info
            .databases
            .first()
            .cloned()
            .filter(|_| dump.info.databases.len() > 1),
        existing: crate::dbs::dumpscan::Existing::Stop,
        content,
        skip_owner_acl: true,
        include_roles: false,
        single_transaction: false,
        objects: None,
        rename_schema: None,
        jobs: 0,
        disable_triggers: false,
    };
    let (plan, job) = plan(PlanInput {
        conn: match &conn {
            Conn::Pg(c) => Conn::Pg(c),
            Conn::My(c) => Conn::My(c),
            Conn::Mongo(c) => Conn::Mongo(c),
        },
        cfg: &cfg,
        ssh: ssh.as_deref(),
        dump: &dump,
        dump_id: dump_id.into(),
        opts,
        saved_name: server.name.into(),
        environment: server.environment.into(),
        mode: server.mode,
    })
    .await?;
    if let Some(why) = plan.blocked {
        return Err(AppError::validation(why));
    }
    create_database(&conn, &scratch, encoding(&job)).await?;
    cfg.database = Some(scratch.clone());
    let out = async {
        let (outcome, message, _) = run(RunInput {
            job: &job,
            cfg: &cfg,
            ssh,
            dump: Arc::clone(&dump),
            stop: Arc::new(AtomicBool::new(false)),
            progress: &|_| {},
        })
        .await?;
        if outcome != "done" {
            return Err(AppError::validation(message.unwrap_or_else(|| {
                "The dump couldn't be restored into the scratch database.".into()
            })));
        }
        then(scratch.clone()).await
    }
    .await;
    drop_database(&conn, &scratch).await;
    out
}
