//! Backup and restore (S43, F26) with the engine's own tools: `pg_dump -Fc` and `pg_restore`. On a route through a
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

pub fn tool_name(engine: Engine, tool: Tool) -> AppResult<&'static str> {
    match (engine, tool) {
        (Engine::Postgres, Tool::Dump) => Ok("pg_dump"),
        (Engine::Postgres, Tool::Restore) => Ok("pg_restore"),
        (e, _) => Err(AppError::validation(format!("Backups of {} databases arrive with their own update.", e.label()))),
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
        Tool::Restore => vec!["--no-owner".into(), "--clean".into(), "--if-exists".into(), "--single-transaction".into(), "--exit-on-error".into()],
    };
    a.extend(["-h".into(), cfg.address.clone(), "-p".into(), cfg.port.to_string(), "-U".into(), cfg.user.clone(), "-d".into(), cfg.database.clone().unwrap_or_default(), "-w".into()]);
    a
}

/// What `sh -c` runs on the host: read the password line, export it, become the tool.
pub fn wrapper(tool: &str, args: &[String]) -> String {
    format!("IFS= read -r PGPASSWORD; export PGPASSWORD; exec {tool} {}", args.iter().map(|a| shq(a)).collect::<Vec<_>>().join(" "))
}

fn password_line(cfg: &SqlConfig) -> AppResult<Vec<u8>> {
    let p = cfg.password.as_ref().map(|p| p.as_str()).unwrap_or("");
    if p.contains('\n') || p.contains('\r') {
        return Err(AppError::validation("This password has a line break in it, so it can't be handed to the dump tool. Use a different one for backups."));
    }
    Ok(format!("{p}\n").into_bytes())
}

#[cfg(test)]
fn local_tool(name: &str) -> std::path::PathBuf {
    match std::env::var("DALAN_TEST_TOOL_DIR") {
        Ok(d) => std::path::Path::new(&d).join(name),
        Err(_) => name.into(),
    }
}
/// The tool by name; a desktop app's PATH often misses where PostgreSQL's tools live, so the usual places are tried.
#[cfg(not(test))]
fn local_tool(name: &str) -> std::path::PathBuf {
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()));
    if on_path {
        return name.into();
    }
    let mut dirs: Vec<std::path::PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/homebrew/opt/libpq/bin", "/usr/local/opt/libpq/bin", "/Applications/Postgres.app/Contents/Versions/latest/bin"].iter().map(Into::into).collect();
    if let Ok(rd) = std::fs::read_dir("/usr/lib/postgresql") {
        dirs.extend(rd.flatten().map(|e| e.path().join("bin")));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file()).unwrap_or_else(|| name.into())
}

/// Where a tool runs, for the dialog: "db-primary" or "this computer".
pub fn runs_on(ssh: Option<&Connected>) -> String {
    ssh.map_or_else(|| "this computer".to_string(), |s| s.label.clone())
}

fn missing(tool: &str, on: &str) -> AppError {
    AppError::new(ErrorCode::DbToolMissing, format!("{tool} isn't installed on {on}. Install the PostgreSQL client tools there, then try again."))
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
            let out = tokio::process::Command::new(local_tool(tool)).arg("--version").output().await.map_err(|_| missing(tool, &on))?;
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
    }
}

async fn exec_collect(ssh: &Connected, cmd: &str) -> AppResult<(Option<u32>, String)> {
    let mut ch = ssh.handle.channel_open_session().await.map_err(|e| AppError::new(ErrorCode::HostUnreachable, format!("Couldn't open a channel on the host: {e}")))?;
    ch.exec(true, cmd.to_string()).await.map_err(|e| AppError::internal(e.to_string()))?;
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
}

fn finish(status: Option<i32>, stderr: &str, stopped: bool, tool: &str) -> AppResult<()> {
    if stopped {
        return Err(AppError::new(ErrorCode::Internal, "Stopped."));
    }
    if status == Some(0) {
        return Ok(());
    }
    // The tool echoes the failing SQL (`Command was: …`), which can hold row data: it is not shown or logged.
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty() && !l.starts_with("Command was")).unwrap_or("it gave no reason");
    Err(AppError::new(ErrorCode::DbRefused, format!("{tool} stopped: {last}")))
}

pub async fn run(mut r: Run<'_>) -> AppResult<()> {
    let tool = tool_name(r.cfg.engine, r.tool)?;
    let a = args(r.cfg, r.tool);
    let pw = password_line(r.cfg)?;
    let io = |e: std::io::Error| AppError::internal(format!("File error: {e}"));
    let (mut bytes, mut stderr) = (0f64, String::new());
    if let Some(ssh) = r.ssh {
        let ch = ssh.handle.channel_open_session().await.map_err(|e| AppError::new(ErrorCode::HostUnreachable, format!("Couldn't open a channel on the host: {e}")))?;
        let (mut read, write) = ch.split();
        write.exec(true, format!("sh -c {}", shq(&wrapper(tool, &a)))).await.map_err(|e| AppError::internal(e.to_string()))?;
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
                            if write.data_bytes(bytes::Bytes::copy_from_slice(&buf[..n])).await.is_err() {
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
            let m = match tokio::time::timeout(std::time::Duration::from_millis(250), read.wait()).await {
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
                    t.lines().filter(|l| !l.starts_with("Command was")).for_each(|l| (r.log)(l.to_string()));
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
    let mut child = tokio::process::Command::new(local_tool(tool))
        .args(&a)
        .env("PGPASSWORD", String::from_utf8_lossy(&pw).trim_end_matches('\n'))
        .stdin(if r.stdin_file.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| missing(tool, "this computer"))?;
    let feeder = r.stdin_file.map(|p| p.to_path_buf()).zip(child.stdin.take()).map(|(path, mut stdin)| {
        tokio::spawn(async move {
            if let Ok(mut f) = tokio::fs::File::open(path).await {
                let _ = tokio::io::copy(&mut f, &mut stdin).await;
            }
            let _ = stdin.shutdown().await;
        })
    });
    let mut so = child.stdout.take().ok_or_else(|| AppError::internal("no output"))?;
    let mut se = child.stderr.take().ok_or_else(|| AppError::internal("no error output"))?;
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
        let n = match tokio::time::timeout(std::time::Duration::from_millis(250), so.read(&mut buf)).await {
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
    stderr.lines().filter(|l| !l.starts_with("Command was")).for_each(|l| (r.log)(l.to_string()));
    finish(status, &stderr, false, tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_reads_the_password_from_input_and_quotes_every_argument() {
        let w = wrapper("pg_dump", &["-U".into(), "o'brien".into(), "-d".into(), "a b;c".into()]);
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
        }
    }

    async fn dump(c: &SqlConfig, ssh: Option<&Connected>) -> (AppResult<()>, Vec<u8>, Vec<String>) {
        let mut out: Vec<u8> = Vec::new();
        let log = std::sync::Mutex::new(Vec::new());
        let r = run(Run { cfg: c, ssh, tool: Tool::Dump, stdout: Some(&mut out), stdin_file: None, progress: &|_| {}, log: &|l| log.lock().unwrap().push(l), stop: &AtomicBool::new(false) }).await;
        (r, out, log.into_inner().unwrap())
    }

    /// A pretend `pg_dump` and `pg_restore` in a folder of their own, found through DALAN_TEST_TOOL_DIR (one test
    /// sets it, so nothing else in the run is affected).
    #[tokio::test]
    async fn the_local_tool_gets_the_password_in_its_environment_never_in_its_arguments_and_failures_say_why() {
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
        assert!(log.iter().any(|l| l.contains("-Fc") && l.contains("-U app") && l.contains("-d shop")));
        assert!(!log.iter().any(|l| l.contains("secret-pw")), "the password is on the command line");
        // Restore reads the file on its input.
        script("pg_restore", "cat > \"$0.in\"; echo \"args: $*\" >&2");
        let file = dir.join("backup.dump");
        std::fs::write(&file, b"FILE-BYTES").unwrap();
        let c = cfg();
        run(Run { cfg: &c, ssh: None, tool: Tool::Restore, stdout: None, stdin_file: Some(&file), progress: &|_| {}, log: &|_| {}, stop: &AtomicBool::new(false) }).await.unwrap();
        assert_eq!(std::fs::read(dir.join("pg_restore.in")).unwrap(), b"FILE-BYTES");
        // A failure says what the tool said; a missing tool says which one and where.
        script("pg_dump", "echo 'pg_dump: error: connection to server failed' >&2\nexit 1");
        let (r, _, _) = dump(&c, None).await;
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbRefused);
        assert!(e.message.contains("connection to server failed"), "{}", e.message);
        std::fs::remove_file(dir.join("pg_dump")).unwrap();
        let e = dump(&c, None).await.0.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbToolMissing);
        assert!(e.message.contains("pg_dump isn't installed on this computer"), "{}", e.message);
        assert!(tool_version(None, "pg_dump").await.is_err());
        // Stop: the child is killed and nothing is reported as a success.
        script("pg_dump", "sleep 30");
        let stop = AtomicBool::new(false);
        let mut out: Vec<u8> = Vec::new();
        let fut = run(Run { cfg: &c, ssh: None, tool: Tool::Dump, stdout: Some(&mut out), stdin_file: None, progress: &|_| {}, log: &|_| {}, stop: &stop });
        let (res, _) = tokio::join!(fut, async { tokio::time::sleep(std::time::Duration::from_millis(300)).await; stop.store(true, Ordering::SeqCst) });
        assert_eq!(res.unwrap_err().message, "Stopped.");
        std::env::remove_var("DALAN_TEST_TOOL_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_over_ssh_the_tool_runs_on_the_host_the_password_comes_in_on_stdin_and_a_missing_tool_is_named() {
        let h = harness(None).await;
        let ssh = sign_in_to_a(&h).await;
        // The host has no pg_dump: the run says so, naming the host.
        let (r, out, _) = dump(&cfg(), Some(&ssh)).await;
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbToolMissing, "{}", e.message);
        assert!(out.is_empty());
        assert_eq!(tool_version(Some(&ssh), "pg_dump").await.unwrap_err().code, ErrorCode::DbToolMissing);
        // The plumbing itself: the wrapper reads the first line of input into the environment, and the rest is the tool's input.
        let ch = ssh.handle.channel_open_session().await.unwrap();
        let (mut read, write) = ch.split();
        let script = wrapper("sh", &["-c".into(), "printf 'pw=%s;' \"$PGPASSWORD\"; cat".into()]);
        write.exec(true, format!("sh -c {}", shq(&script))).await.unwrap();
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
