//! Backup and restore (S43, F26) with the engine's own tools: `pg_dump -Fc` and `pg_restore`, `mysqldump` and `mysql`, `mongodump --archive` and `mongorestore --archive`. On a route through a
//! host the tool runs ON the host over an SSH exec channel (next to the data, matching the server's version) and its
//! output streams to a file on this computer; on a direct route it runs on this computer if it is installed.
//!
//! The password never appears on a command line: it goes in as the first line of the tool's standard input, read by
//! a tiny `sh` wrapper into `PGPASSWORD` before the tool starts (locally it is set in the child's environment).

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use russh::ChannelMsg;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::SqlConfig;
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::Connected;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Dump,
    Restore,
}

/// The tools that can do the job, in the order to try (MariaDB's own names first, then the MySQL ones it still ships).
pub fn tool_candidates(engine: Engine, tool: Tool) -> AppResult<Vec<&'static str>> {
    Ok(match (engine, tool) {
        (Engine::Postgres, Tool::Dump) => vec!["pg_dump"],
        (Engine::Postgres, Tool::Restore) => vec!["pg_restore"],
        (Engine::Mysql, Tool::Dump) => vec!["mysqldump"],
        (Engine::Mysql, Tool::Restore) => vec!["mysql"],
        (Engine::Mariadb, Tool::Dump) => vec!["mariadb-dump", "mysqldump"],
        (Engine::Mariadb, Tool::Restore) => vec!["mariadb", "mysql"],
        (Engine::Mongodb, Tool::Dump) => vec!["mongodump"],
        (Engine::Mongodb, Tool::Restore) => vec!["mongorestore"],
        (e, _) => {
            return Err(AppError::validation(format!(
                "Backups of {} databases arrive with their own update.",
                e.label()
            )))
        }
    })
}

pub fn tool_name(engine: Engine, tool: Tool) -> AppResult<&'static str> {
    Ok(tool_candidates(engine, tool)?[0])
}

pub fn is_my(engine: Engine) -> bool {
    matches!(engine, Engine::Mysql | Engine::Mariadb)
}

pub fn is_mongo(engine: Engine) -> bool {
    engine == Engine::Mongodb
}

/// Arguments for `mongodump` and `mongorestore` (the config file comes first, added by the caller). The archive goes
/// through standard output (a dump) or standard input (a restore), gzipped.
pub fn args_mongo(cfg: &SqlConfig, tool: Tool) -> Vec<String> {
    let mut a: Vec<String> = vec!["--archive".into(), "--gzip".into()];
    // A seed list backs up from its first member; the tool follows a replica set from there.
    let first = cfg.address.split(',').next().unwrap_or("").trim();
    let (host, port) = match first
        .rsplit_once(':')
        .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h, p)))
    {
        Some((h, p)) => (h.to_string(), p),
        None => (first.to_string(), cfg.port),
    };
    // `--option=value` in one word: a value that starts with `-` can't be read as another option.
    a.extend([format!("--host={host}"), format!("--port={port}")]);
    if !cfg.user.is_empty() {
        a.extend([
            format!("--username={}", cfg.user),
            format!(
                "--authenticationDatabase={}",
                cfg.auth_source
                    .clone()
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| "admin".into())
            ),
        ]);
    }
    match cfg.tls {
        crate::dbs::transport::TlsMode::Off => {}
        // "Require" is encryption without checking the certificate, the same as in the app's own connection.
        crate::dbs::transport::TlsMode::Require => {
            a.extend(["--tls".into(), "--tlsInsecure".into()])
        }
        crate::dbs::transport::TlsMode::Verify => a.push("--tls".into()),
    }
    match (tool, cfg.database.as_deref().filter(|d| !d.is_empty())) {
        (Tool::Dump, Some(d)) => a.push(format!("--db={d}")),
        // Restore into this database only, replacing what is there (the dialog says so and asks for the name).
        (Tool::Restore, Some(d)) => a.extend([format!("--nsInclude={d}.*"), "--drop".into()]),
        (Tool::Restore, None) => a.push("--drop".into()),
        (Tool::Dump, None) => {}
    }
    a
}

/// The first line of the tool's input: a one-line YAML config with the password (read by the wrapper into a private
/// temporary file, removed when the tool ends), so the password is never on a command line.
pub fn config_mongo(cfg: &SqlConfig) -> AppResult<Vec<u8>> {
    let p = cfg.password.as_ref().map(|p| p.as_str()).unwrap_or("");
    // Control characters and the Unicode line separators are not valid inside a YAML string: refuse, don't alter.
    if p.chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(AppError::validation("This password has a control character or line break in it, so it can't be handed to the dump tool. Use a different one for backups."));
    }
    Ok(format!(
        "password: \"{}\"\n",
        p.replace('\\', "\\\\").replace('"', "\\\"")
    )
    .into_bytes())
}

/// What `sh -c` runs on the host for MongoDB: one input line becomes a private config file for the tool.
pub fn wrapper_mongo(tool: &str, args: &[String]) -> String {
    format!(
        "umask 077; f=$(mktemp) || exit 1; trap 'rm -f \"$f\"' EXIT HUP INT TERM; {{ IFS= read -r l1; printf '%s\\n' \"$l1\" > \"$f\"; }}; {tool} --config=\"$f\" {}",
        args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" ")
    )
}

/// The first candidate that is installed (on the host or here) and its version line.
pub async fn resolve_tool(
    ssh: Option<&Connected>,
    engine: Engine,
    tool: Tool,
) -> AppResult<(String, String)> {
    let candidates = tool_candidates(engine, tool)?;
    for c in &candidates {
        if let Ok(v) = tool_version(ssh, c).await {
            return Ok((c.to_string(), v));
        }
    }
    Err(missing(candidates[0], &runs_on(ssh)))
}

/// Arguments for `mysqldump` and `mysql` (the option file comes first, added by the caller).
pub fn args_my(cfg: &SqlConfig, tool: Tool) -> Vec<String> {
    let mut a: Vec<String> = match tool {
        Tool::Dump => vec![
            "--single-transaction".into(),
            "--no-tablespaces".into(),
            "--routines".into(),
            "--triggers".into(),
            "--events".into(),
        ],
        Tool::Restore => vec![],
    };
    // `-h localhost` would use the unix socket and ignore the port: always TCP.
    a.extend([
        "--protocol=tcp".into(),
        "-h".into(),
        cfg.address.clone(),
        "-P".into(),
        cfg.port.to_string(),
        "-u".into(),
        cfg.user.clone(),
    ]);
    match cfg.database.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => a.push(d.to_string()),
        None if tool == Tool::Dump => a.push("--all-databases".into()),
        None => {}
    }
    a
}

/// The first two lines of the tool's input: a MySQL option file with the password (read by the wrapper into a
/// private temporary file, removed when the tool ends). Quotes and backslashes are escaped as option files want.
pub fn config_lines(cfg: &SqlConfig) -> AppResult<Vec<u8>> {
    let p = cfg.password.as_ref().map(|p| p.as_str()).unwrap_or("");
    if p.contains('\n') || p.contains('\r') {
        return Err(AppError::validation("This password has a line break in it, so it can't be handed to the dump tool. Use a different one for backups."));
    }
    Ok(format!(
        "[client]\npassword=\"{}\"\n",
        p.replace('\\', "\\\\").replace('"', "\\\"")
    )
    .into_bytes())
}

/// What `sh -c` runs on the host for MySQL and MariaDB: make a private option file from the first two input lines,
/// run the tool with it, and remove the file whatever happens.
pub fn wrapper_my(tool: &str, args: &[String]) -> String {
    format!(
        "umask 077; f=$(mktemp) || exit 1; trap 'rm -f \"$f\"' EXIT HUP INT TERM; {{ IFS= read -r l1; IFS= read -r l2; printf '%s\\n%s\\n' \"$l1\" \"$l2\" > \"$f\"; }}; {tool} --defaults-file=\"$f\" {}",
        args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" ")
    )
}

/// A private option file on this computer for a local run; deleted when dropped.
pub struct LocalConfig(pub std::path::PathBuf);
impl LocalConfig {
    pub fn create(contents: &[u8]) -> AppResult<LocalConfig> {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("dalan-{}.cnf", uuid::Uuid::now_v7()));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o
            .open(&path)
            .map_err(|e| AppError::internal(format!("Couldn't make the option file: {e}")))?;
        f.write_all(contents)
            .map_err(|e| AppError::internal(format!("Couldn't write the option file: {e}")))?;
        Ok(LocalConfig(path))
    }
}
impl Drop for LocalConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Single-quotes a word for `sh`.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn args(cfg: &SqlConfig, tool: Tool) -> Vec<String> {
    let mut a: Vec<String> = match tool {
        Tool::Dump => vec!["-Fc".into()],
        // One transaction that stops at the first error: a failed restore leaves the database as it was.
        Tool::Restore => vec![
            "--no-owner".into(),
            "--clean".into(),
            "--if-exists".into(),
            "--single-transaction".into(),
            "--exit-on-error".into(),
        ],
    };
    a.extend([
        "-h".into(),
        cfg.address.clone(),
        "-p".into(),
        cfg.port.to_string(),
        "-U".into(),
        cfg.user.clone(),
        "-d".into(),
        cfg.database.clone().unwrap_or_default(),
        "-w".into(),
    ]);
    a
}

/// What `sh -c` runs on the host: read the password line, export it, become the tool.
pub fn wrapper(tool: &str, args: &[String]) -> String {
    format!(
        "IFS= read -r PGPASSWORD; export PGPASSWORD; exec {tool} {}",
        args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" ")
    )
}

pub fn password_line(cfg: &SqlConfig) -> AppResult<Vec<u8>> {
    let p = cfg.password.as_ref().map(|p| p.as_str()).unwrap_or("");
    if p.contains('\n') || p.contains('\r') {
        return Err(AppError::validation("This password has a line break in it, so it can't be handed to the dump tool. Use a different one for backups."));
    }
    Ok(format!("{p}\n").into_bytes())
}

#[cfg(test)]
pub fn local_tool(name: &str) -> std::path::PathBuf {
    match std::env::var("DALAN_TEST_TOOL_DIR") {
        Ok(d) => std::path::Path::new(&d).join(name),
        Err(_) => name.into(),
    }
}
/// The tool by name; a desktop app's PATH often misses where PostgreSQL's tools live, so the usual places are tried.
#[cfg(not(test))]
pub fn local_tool(name: &str) -> std::path::PathBuf {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()));
    if on_path {
        return name.into();
    }
    let mut dirs: Vec<std::path::PathBuf> = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/opt/homebrew/opt/libpq/bin",
        "/usr/local/opt/libpq/bin",
        "/Applications/Postgres.app/Contents/Versions/latest/bin",
    ]
    .iter()
    .map(Into::into)
    .collect();
    if let Ok(rd) = std::fs::read_dir("/usr/lib/postgresql") {
        dirs.extend(rd.flatten().map(|e| e.path().join("bin")));
    }
    dirs.into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| name.into())
}

/// Where a tool runs, for the dialog: "db-primary" or "this computer".
pub fn runs_on(ssh: Option<&Connected>) -> String {
    ssh.map_or_else(|| "this computer".to_string(), |s| s.label.clone())
}

pub fn missing(tool: &str, on: &str) -> AppError {
    let what = if tool.starts_with("mongo") {
        "the MongoDB Database Tools"
    } else if tool.starts_with("my") || tool.starts_with("maria") {
        "the MySQL or MariaDB client tools"
    } else {
        "the PostgreSQL client tools"
    };
    AppError::new(
        ErrorCode::DbToolMissing,
        format!("{tool} isn't installed on {on}. Install {what} there, then try again."),
    )
}

/// The tool's version line, or `DbToolMissing`.
pub async fn tool_version(ssh: Option<&Connected>, tool: &str) -> AppResult<String> {
    let on = runs_on(ssh);
    match ssh {
        Some(s) => {
            let (status, out) = exec_collect(s, &format!("{tool} --version")).await?;
            if status != Some(0) {
                return Err(missing(tool, &on));
            }
            Ok(out.trim().to_string())
        }
        None => {
            let out = tokio::process::Command::new(local_tool(tool))
                .arg("--version")
                .output()
                .await
                .map_err(|_| missing(tool, &on))?;
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
    }
}

pub async fn exec_collect(ssh: &Connected, cmd: &str) -> AppResult<(Option<u32>, String)> {
    let mut ch = ssh.handle.channel_open_session().await.map_err(|e| {
        AppError::new(
            ErrorCode::HostUnreachable,
            format!("Couldn't open a channel on the host: {e}"),
        )
    })?;
    ch.exec(true, cmd.to_string())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let (mut out, mut status) = (Vec::new(), None);
    while let Some(m) = ch.wait().await {
        match m {
            ChannelMsg::Data { data } => out.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok((status, String::from_utf8_lossy(&out).into_owned()))
}

/// Everything a run needs. `stdout` receives the tool's output (a dump); `stdin_file` is fed to its input (a restore).
pub struct Run<'a> {
    pub cfg: &'a SqlConfig,
    pub ssh: Option<&'a Connected>,
    pub tool: Tool,
    pub stdout: Option<&'a mut (dyn Write + Send)>,
    pub stdin_file: Option<&'a Path>,
    pub progress: &'a (dyn Fn(f64) + Sync),
    pub log: &'a (dyn Fn(String) + Sync),
    pub stop: &'a AtomicBool,
    /// PostgreSQL: the server's major version, so a dump is made with a pg_dump that doesn't write settings the
    /// server (or a pg_restore of its version) can't take. `None` when it isn't known.
    pub server_major: Option<u32>,
}

/// Where a PostgreSQL tool of one major version usually lives (Debian, Homebrew on Apple and Intel Macs,
/// Postgres.app).
pub fn versioned_pg_tool(name: &str, major: u32) -> Vec<String> {
    vec![
        format!("/usr/lib/postgresql/{major}/bin/{name}"),
        format!("/opt/homebrew/opt/postgresql@{major}/bin/{name}"),
        format!("/usr/local/opt/postgresql@{major}/bin/{name}"),
        format!("/Applications/Postgres.app/Contents/Versions/{major}/bin/{name}"),
    ]
}

/// The PostgreSQL tool to run against a server of `server_major`: the usual one, unless it is 17 or later and the
/// server is older (pg_dump and pg_restore 17 write `SET transaction_timeout`, which those servers refuse) and a
/// client of the server's own version is installed.
pub async fn pg_tool_for(ssh: Option<&Connected>, name: &str, server_major: Option<u32>) -> String {
    let Some(server) = server_major.filter(|m| *m > 0 && *m < 17) else {
        return name.to_string();
    };
    let usual = tool_version(ssh, name)
        .await
        .ok()
        .and_then(|v| crate::dbs::dumpfile::major(v.rsplit(' ').next().unwrap_or("")));
    if usual.is_some_and(|m| m < 17) {
        return name.to_string();
    }
    for c in versioned_pg_tool(name, server) {
        if tool_version(ssh, &c).await.is_ok() {
            return c;
        }
    }
    name.to_string()
}

fn finish(status: Option<i32>, stderr: &str, stopped: bool, tool: &str) -> AppResult<()> {
    if stopped {
        return Err(AppError::new(ErrorCode::Internal, "Stopped."));
    }
    if status == Some(0) {
        return Ok(());
    }
    // The tool echoes the failing SQL (`Command was: …`), which can hold row data: it is not shown or logged.
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty() && !l.starts_with("Command was"))
        .unwrap_or("it gave no reason");
    // `ERROR 1064 … near '<text>'` quotes the statement, which can be row data: keep the words, drop the quote.
    let last = last.split(" near '").next().unwrap_or(last);
    Err(AppError::new(
        ErrorCode::DbRefused,
        format!("{tool} stopped: {last}"),
    ))
}

pub async fn run(mut r: Run<'_>) -> AppResult<()> {
    let my = is_my(r.cfg.engine);
    let mongo = is_mongo(r.cfg.engine);
    let tool: String = if my || mongo {
        resolve_tool(r.ssh, r.cfg.engine, r.tool).await?.0
    } else {
        pg_tool_for(r.ssh, tool_name(r.cfg.engine, r.tool)?, r.server_major).await
    };
    let tool = tool.as_str();
    let a = if my {
        args_my(r.cfg, r.tool)
    } else if mongo {
        args_mongo(r.cfg, r.tool)
    } else {
        args(r.cfg, r.tool)
    };
    let pw = if my {
        config_lines(r.cfg)?
    } else if mongo {
        config_mongo(r.cfg)?
    } else {
        password_line(r.cfg)?
    };
    let io = |e: std::io::Error| AppError::internal(format!("File error: {e}"));
    let (mut bytes, mut stderr) = (0f64, String::new());
    if let Some(ssh) = r.ssh {
        let ch = ssh.handle.channel_open_session().await.map_err(|e| {
            AppError::new(
                ErrorCode::HostUnreachable,
                format!("Couldn't open a channel on the host: {e}"),
            )
        })?;
        let (mut read, write) = ch.split();
        write
            .exec(
                true,
                format!(
                    "sh -c {}",
                    shq(&if my {
                        wrapper_my(tool, &a)
                    } else if mongo {
                        wrapper_mongo(tool, &a)
                    } else {
                        wrapper(tool, &a)
                    })
                ),
            )
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        let file = r.stdin_file.map(Path::to_path_buf);
        // Feeding the tool is its own task: the tool's answers must be read meanwhile or both sides would wait.
        let feeder = tokio::spawn(async move {
            let _ = write.data_bytes(pw).await;
            if let Some(path) = file {
                let Ok(mut f) = tokio::fs::File::open(path).await else {
                    let _ = write.close().await;
                    return write;
                };
                let mut buf = vec![0u8; 32 * 1024];
                loop {
                    match f.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if write
                                .data_bytes(bytes::Bytes::copy_from_slice(&buf[..n]))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        // A file that can't be read to its end must not look like a finished restore.
                        Err(_) => {
                            let _ = write.close().await;
                            return write;
                        }
                    }
                }
            }
            let _ = write.eof().await;
            write
        });
        let mut status = None;
        loop {
            if r.stop.load(Ordering::SeqCst) {
                feeder.abort();
                return finish(None, "", true, tool);
            }
            // Wake up now and then, so Stop works while the tool is quiet.
            let m = match tokio::time::timeout(std::time::Duration::from_millis(250), read.wait())
                .await
            {
                Err(_) => continue,
                Ok(None) => break,
                Ok(Some(m)) => m,
            };
            match m {
                ChannelMsg::Data { data } => {
                    bytes += data.len() as f64;
                    if let Some(out) = r.stdout.as_mut() {
                        out.write_all(&data).map_err(io)?;
                    }
                    (r.progress)(bytes);
                }
                ChannelMsg::ExtendedData { data, .. } => {
                    let t = String::from_utf8_lossy(&data).into_owned();
                    t.lines()
                        .filter(|l| !l.starts_with("Command was"))
                        .for_each(|l| (r.log)(l.to_string()));
                    if stderr.len() < 16_384 {
                        stderr.push_str(&t);
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status as i32),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        feeder.abort();
        if status.is_none() && stderr.contains("not found") {
            return Err(missing(tool, &runs_on(r.ssh)));
        }
        if status == Some(127) {
            return Err(missing(tool, &runs_on(r.ssh)));
        }
        return finish(status, &stderr, false, tool);
    }
    // This computer: the same tool, the password in the child's environment (not on its command line).
    // MySQL's tools read the password from a private option file (listed first; `--defaults-file` so no other option file, such as ~/.my.cnf, can override it); PostgreSQL's from the environment.
    let cfg_file = if my || mongo {
        Some(LocalConfig::create(&pw)?)
    } else {
        None
    };
    let opt = if mongo { "--config" } else { "--defaults-file" };
    let mut full: Vec<String> = cfg_file
        .iter()
        .map(|c| format!("{opt}={}", c.0.display()))
        .collect();
    full.extend(a.iter().cloned());
    let mut cmd = tokio::process::Command::new(local_tool(tool));
    cmd.args(&full);
    if !my && !mongo {
        cmd.env(
            "PGPASSWORD",
            String::from_utf8_lossy(&pw).trim_end_matches('\n'),
        );
    }
    let mut child = cmd
        .stdin(if r.stdin_file.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| missing(tool, "this computer"))?;
    let feeder = r
        .stdin_file
        .map(|p| p.to_path_buf())
        .zip(child.stdin.take())
        .map(|(path, mut stdin)| {
            tokio::spawn(async move {
                if let Ok(mut f) = tokio::fs::File::open(path).await {
                    let _ = tokio::io::copy(&mut f, &mut stdin).await;
                }
                let _ = stdin.shutdown().await;
            })
        });
    let mut so = child
        .stdout
        .take()
        .ok_or_else(|| AppError::internal("no output"))?;
    let mut se = child
        .stderr
        .take()
        .ok_or_else(|| AppError::internal("no error output"))?;
    let err_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s).await;
        s
    });
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if r.stop.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            return finish(None, "", true, tool);
        }
        let n = match tokio::time::timeout(std::time::Duration::from_millis(250), so.read(&mut buf))
            .await
        {
            Err(_) => continue,
            Ok(n) => n.map_err(io)?,
        };
        if n == 0 {
            break;
        }
        bytes += n as f64;
        if let Some(out) = r.stdout.as_mut() {
            out.write_all(&buf[..n]).map_err(io)?;
        }
        (r.progress)(bytes);
    }
    let status = child.wait().await.map_err(io)?.code();
    if let Some(f) = feeder {
        f.abort();
    }
    let stderr = err_task.await.unwrap_or_default();
    stderr
        .lines()
        .filter(|l| !l.starts_with("Command was"))
        .for_each(|l| (r.log)(l.to_string()));
    finish(status, &stderr, false, tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_reads_the_password_from_input_and_quotes_every_argument() {
        let w = wrapper(
            "pg_dump",
            &["-U".into(), "o'brien".into(), "-d".into(), "a b;c".into()],
        );
        assert_eq!(w, "IFS= read -r PGPASSWORD; export PGPASSWORD; exec pg_dump '-U' 'o'\\''brien' '-d' 'a b;c'");
        assert!(!w.contains("hunter2"));
        assert_eq!(shq("it's"), "'it'\\''s'");
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::spike::sign_in_to_a;
    use crate::dbs::transport::TlsMode;
    use crate::redis::store::RedisRoute;
    use crate::ssh::tests::harness;
    use zeroize::Zeroizing;

    fn cfg() -> SqlConfig {
        SqlConfig {
            name: "t".into(),
            engine: Engine::Postgres,
            route: RedisRoute::Direct,
            host_id: None,
            address: "127.0.0.1".into(),
            port: 5432,
            database: Some("shop".into()),
            user: "app".into(),
            password: Some(Zeroizing::new("secret-pw".into())),
            tls: TlsMode::Off,
            server_name: None,
            auth_source: None,
        }
    }

    async fn dump(c: &SqlConfig, ssh: Option<&Connected>) -> (AppResult<()>, Vec<u8>, Vec<String>) {
        let mut out: Vec<u8> = Vec::new();
        let log = std::sync::Mutex::new(Vec::new());
        let r = run(Run {
            cfg: c,
            ssh,
            tool: Tool::Dump,
            stdout: Some(&mut out),
            stdin_file: None,
            progress: &|_| {},
            log: &|l| log.lock().unwrap().push(l),
            stop: &AtomicBool::new(false),
            server_major: None,
        })
        .await;
        (r, out, log.into_inner().unwrap())
    }

    /// A pretend `pg_dump` and `pg_restore` in a folder of their own, found through DALAN_TEST_TOOL_DIR (one test
    /// sets it, so nothing else in the run is affected).
    #[tokio::test]
    async fn the_local_tool_gets_the_password_in_its_environment_never_in_its_arguments_and_failures_say_why(
    ) {
        let dir = std::env::temp_dir().join(format!("dalan-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        std::env::set_var("DALAN_TEST_TOOL_DIR", &dir);
        script("pg_dump", "echo \"args: $*\" >&2\nprintf 'DUMP\\n'\n[ \"$PGPASSWORD\" = \"secret-pw\" ] && printf 'pw-ok\\n'\nexit 0");
        let (r, out, log) = dump(&cfg(), None).await;
        r.unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "DUMP\npw-ok\n");
        assert!(log
            .iter()
            .any(|l| l.contains("-Fc") && l.contains("-U app") && l.contains("-d shop")));
        assert!(
            !log.iter().any(|l| l.contains("secret-pw")),
            "the password is on the command line"
        );
        // Restore reads the file on its input.
        script("pg_restore", "cat > \"$0.in\"; echo \"args: $*\" >&2");
        let file = dir.join("backup.dump");
        std::fs::write(&file, b"FILE-BYTES").unwrap();
        let c = cfg();
        run(Run {
            cfg: &c,
            ssh: None,
            tool: Tool::Restore,
            stdout: None,
            stdin_file: Some(&file),
            progress: &|_| {},
            log: &|_| {},
            stop: &AtomicBool::new(false),
            server_major: None,
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(dir.join("pg_restore.in")).unwrap(),
            b"FILE-BYTES"
        );
        // A failure says what the tool said; a missing tool says which one and where.
        script(
            "pg_dump",
            "echo 'pg_dump: error: connection to server failed' >&2\nexit 1",
        );
        let (r, _, _) = dump(&c, None).await;
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbRefused);
        assert!(
            e.message.contains("connection to server failed"),
            "{}",
            e.message
        );
        std::fs::remove_file(dir.join("pg_dump")).unwrap();
        let e = dump(&c, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbToolMissing);
        assert!(
            e.message
                .contains("pg_dump isn't installed on this computer"),
            "{}",
            e.message
        );
        assert!(tool_version(None, "pg_dump").await.is_err());
        // MySQL and MariaDB: the password reaches the tool in a private option file, not in its arguments, and the file
        // is gone afterwards.
        script("mysqldump", "cfg=${1#--defaults-file=}\necho \"mode=$(stat -f %Lp \"$cfg\" 2>/dev/null || stat -c %a \"$cfg\")\"\ncat \"$cfg\"\necho \"$cfg\" > \"$0.cfgpath\"\necho \"args: $*\" >&2");
        let mut my = cfg();
        my.engine = Engine::Mysql;
        my.password = Some(Zeroizing::new("p\"w\\x".into()));
        let (r, out, log) = dump(&my, None).await;
        r.unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with("mode=600\n[client]\npassword=\"p\\\"w\\\\x\"\n"),
            "{out}"
        );
        assert!(
            log.iter().all(|l| !l.contains("p\"w")),
            "the password is on the command line: {log:?}"
        );
        assert!(
            log.iter().any(|l| l.contains("--single-transaction")
                && l.contains("-P 5432")
                && l.contains("--protocol=tcp")
                && l.contains("-u app")
                && l.contains(" shop")),
            "{log:?}"
        );
        let used = std::fs::read_to_string(dir.join("mysqldump.cfgpath")).unwrap();
        assert!(
            !std::path::Path::new(used.trim()).exists(),
            "the option file was not removed"
        );
        // MariaDB prefers its own tool name and falls back to mysqldump.
        my.engine = Engine::Mariadb;
        script("mariadb-dump", "echo MARIADB-DUMP");
        assert_eq!(
            String::from_utf8(dump(&my, None).await.1).unwrap(),
            "MARIADB-DUMP\n"
        );
        std::fs::remove_file(dir.join("mariadb-dump")).unwrap();
        assert!(String::from_utf8(dump(&my, None).await.1)
            .unwrap()
            .starts_with("mode=600"));
        // MongoDB: the password reaches the tool in a private one-line config file, never in its arguments, and the file
        // is gone afterwards; the archive goes through the tool's output.
        script("mongodump", "cfg=${1#--config=}\necho \"mode=$(stat -f %Lp \"$cfg\" 2>/dev/null || stat -c %a \"$cfg\")\"\ncat \"$cfg\"\necho \"$cfg\" > \"$0.cfgpath\"\necho \"args: $*\" >&2");
        let mut mg = cfg();
        mg.engine = Engine::Mongodb;
        mg.port = 27017;
        mg.address = "h1:27018,h2:27017".into();
        mg.database = Some("shop".into());
        mg.auth_source = Some("admin".into());
        mg.password = Some(Zeroizing::new("p\"w\\x".into()));
        let (r, out, log) = dump(&mg, None).await;
        r.unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.starts_with("mode=600\npassword: \"p\\\"w\\\\x\"\n"),
            "{out}"
        );
        assert!(
            log.iter().all(|l| !l.contains("p\"w")),
            "the password is on the command line: {log:?}"
        );
        assert!(log.iter().any(|l| l.contains("--archive --gzip --host=h1 --port=27018 --username=app --authenticationDatabase=admin --db=shop")), "{log:?}");
        let used = std::fs::read_to_string(dir.join("mongodump.cfgpath")).unwrap();
        assert!(
            !std::path::Path::new(used.trim()).exists(),
            "the config file was not removed"
        );
        // Restore replaces only this database's collections.
        let r = args_mongo(&mg, Tool::Restore).join(" ");
        assert!(r.contains("--nsInclude=shop.* --drop"), "{r}");
        let mut ctl = cfg();
        ctl.engine = Engine::Mongodb;
        ctl.password = Some(Zeroizing::new("a\u{7}b".into()));
        assert!(config_mongo(&ctl).is_err());
        ctl.password = Some(Zeroizing::new("a\u{2028}b".into()));
        assert!(config_mongo(&ctl).is_err());
        ctl.password = Some(Zeroizing::new("p#:x -y".into()));
        assert_eq!(config_mongo(&ctl).unwrap(), b"password: \"p#:x -y\"\n");
        mg.tls = TlsMode::Require;
        assert!(args_mongo(&mg, Tool::Dump)
            .join(" ")
            .contains("--tls --tlsInsecure"));
        assert_eq!(wrapper_mongo("mongodump", &["--archive".into()]), "umask 077; f=$(mktemp) || exit 1; trap 'rm -f \"$f\"' EXIT HUP INT TERM; { IFS= read -r l1; printf '%s\\n' \"$l1\" > \"$f\"; }; mongodump --config=\"$f\" '--archive'");
        // Stop: the child is killed and nothing is reported as a success.
        script("pg_dump", "sleep 30");
        let stop = AtomicBool::new(false);
        let mut out: Vec<u8> = Vec::new();
        let fut = run(Run {
            cfg: &c,
            ssh: None,
            tool: Tool::Dump,
            stdout: Some(&mut out),
            stdin_file: None,
            progress: &|_| {},
            log: &|_| {},
            stop: &stop,
            server_major: None,
        });
        let (res, _) = tokio::join!(fut, async {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            stop.store(true, Ordering::SeqCst)
        });
        assert_eq!(res.unwrap_err().message, "Stopped.");
        std::env::remove_var("DALAN_TEST_TOOL_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_over_ssh_the_tool_runs_on_the_host_the_password_comes_in_on_stdin_and_a_missing_tool_is_named(
    ) {
        let h = harness(None).await;
        let ssh = sign_in_to_a(&h).await;
        // The host has no pg_dump: the run says so, naming the host.
        let (r, out, _) = dump(&cfg(), Some(&ssh)).await;
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbToolMissing, "{}", e.message);
        assert!(out.is_empty());
        assert_eq!(
            tool_version(Some(&ssh), "pg_dump").await.unwrap_err().code,
            ErrorCode::DbToolMissing
        );
        // MySQL's wrapper: the first two input lines become a private option file for the tool, removed when it ends.
        let mk = ssh.handle.channel_open_session().await.unwrap();
        mk.exec(true, "mkdir -p /tmp/dalan-t && printf '#!/bin/sh\\ncfg=${1#--defaults-file=}\\necho \"path=$cfg\"\\nstat -c mode=%%a \"$cfg\"\\ncat \"$cfg\"\\ncat\\n' > /tmp/dalan-t/faketool && chmod +x /tmp/dalan-t/faketool").await.unwrap();
        let mut mk = mk;
        while let Some(m) = mk.wait().await {
            if matches!(m, ChannelMsg::Close) {
                break;
            }
        }
        let ch = ssh.handle.channel_open_session().await.unwrap();
        let (mut read, write) = ch.split();
        write
            .exec(
                true,
                format!("sh -c {}", shq(&wrapper_my("/tmp/dalan-t/faketool", &[]))),
            )
            .await
            .unwrap();
        write
            .data_bytes(b"[client]\npassword=\"secret\"\nSQL-BODY".to_vec())
            .await
            .unwrap();
        write.eof().await.unwrap();
        let mut got = Vec::new();
        while let Some(m) = read.wait().await {
            match m {
                ChannelMsg::Data { data } => got.extend_from_slice(&data),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let got = String::from_utf8(got).unwrap();
        assert!(
            got.contains("mode=600\n[client]\npassword=\"secret\"\nSQL-BODY"),
            "{got}"
        );
        let path = got
            .lines()
            .next()
            .unwrap()
            .trim_start_matches("path=")
            .to_string();
        let (status, listing) = exec_collect(
            &ssh,
            &format!("test -e {} && echo still-there || echo gone", shq(&path)),
        )
        .await
        .unwrap();
        assert_eq!(
            (status, listing.trim()),
            (Some(0), "gone"),
            "the option file was not removed on the host"
        );
        // The plumbing itself: the wrapper reads the first line of input into the environment, and the rest is the tool's input.
        let ch = ssh.handle.channel_open_session().await.unwrap();
        let (mut read, write) = ch.split();
        let script = wrapper(
            "sh",
            &["-c".into(), "printf 'pw=%s;' \"$PGPASSWORD\"; cat".into()],
        );
        write
            .exec(true, format!("sh -c {}", shq(&script)))
            .await
            .unwrap();
        write.data_bytes(b"secret-pw\nFILE".to_vec()).await.unwrap();
        write.eof().await.unwrap();
        let mut got = Vec::new();
        while let Some(m) = read.wait().await {
            match m {
                ChannelMsg::Data { data } => got.extend_from_slice(&data),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        assert_eq!(String::from_utf8(got).unwrap(), "pw=secret-pw;FILE");
    }
}
