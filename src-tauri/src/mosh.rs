//! Mosh (could-have M4): sign in over SSH, start `mosh-server` on the host, then run the local
//! `mosh-client` against the UDP port and key it printed, inside a terminal tab.
//!
//! Dalan doesn't implement the Mosh protocol: it needs `mosh` installed on this computer
//! (`mosh-client`) and on the server (`mosh-server`). The SSH connection is closed once the server
//! has started; after that only mosh's own UDP traffic remains, so jump hosts and proxies can't be
//! used (mosh can't go through them) and the host's address must be reachable directly.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::error::ErrorCode;
use crate::hosts;
use crate::pty::{self, Program};
use crate::ssh::client::{self, Params};
use crate::ssh::{Cmd, Prompts, SessionEvent, SessionIo, SessionState};
use crate::state::Store;

/// What runs on the server. `new` starts a session and prints `MOSH CONNECT <port> <key>`.
pub const SERVER_COMMAND: &str = "mosh-server new -s -c 256 -l LANG=en_US.UTF-8";

/// `MOSH CONNECT 60001 AbCdEf...` -> (60001, key). The key is 22 characters of base64.
pub fn parse_connect(output: &str) -> Option<(u16, String)> {
    output.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        (w.next()? == "MOSH" && w.next()? == "CONNECT").then_some(())?;
        let port: u16 = w.next()?.parse().ok().filter(|p| *p > 0)?;
        let key = w.next()?.to_string();
        let ok = (16..=64).contains(&key.len())
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='));
        ok.then_some((port, key))
    })
}

/// The local `mosh-client`. `DALAN_MOSH_CLIENT` overrides it (tests, odd installs). A GUI app's PATH is
/// thin on a Mac, so Homebrew and MacPorts folders are tried too.
pub fn find_client() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DALAN_MOSH_CLIENT")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    let name = if cfg!(windows) {
        "mosh-client.exe"
    } else {
        "mosh-client"
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/opt/local/bin",
            "/usr/bin",
        ]
        .map(PathBuf::from),
    );
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

fn fail(io: &SessionIo, code: ErrorCode, message: String) {
    (io.event)(SessionEvent::Error { code, message });
}

/// One Mosh session: bootstrap over SSH, then the local client in a PTY.
pub async fn run(
    store: Store,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    p: Params,
    cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let hid = p.host_id.clone();
    let info = match store
        .with_db_session(move |c, s| hosts::connect_info(c, s, &hid))
        .await
    {
        Ok(i) => i,
        Err(e) => return fail(&io, e.code, e.message),
    };
    if !info.jump_host_ids.is_empty() || info.proxy.is_some() {
        return fail(&io, ErrorCode::Validation, format!("{} uses a jump host or proxy, which Mosh can't go through. Mosh talks to the server directly over UDP.", info.label));
    }
    let Some(client_path) = find_client() else {
        return fail(&io, ErrorCode::NotFound, "Mosh isn't installed on this computer. Install it (for example `brew install mosh`), then try again.".into());
    };
    let Some(conn) =
        client::connect_authenticated(&store, &io, &prompts, &p.host_id, None, false).await
    else {
        return;
    };
    let output =
        client::exec_output(&conn.handle, SERVER_COMMAND, 8192, Duration::from_secs(15)).await;
    let _ = conn
        .handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    let text = output
        .map(|o| String::from_utf8_lossy(&o).into_owned())
        .unwrap_or_default();
    let Some((port, key)) = parse_connect(&text) else {
        let why = if text.contains("not found") || text.contains("No such file") {
            format!(
                "mosh-server isn't installed on {}. Install mosh there, then try again.",
                conn.label
            )
        } else {
            format!("{} didn't start mosh-server.", conn.label)
        };
        return fail(&io, ErrorCode::Internal, why);
    };
    // mosh-client wants an IP address, not a name.
    let ip = match tokio::net::lookup_host((info.address.as_str(), port))
        .await
        .ok()
        .and_then(|mut a| a.next())
    {
        Some(a) => a.ip(),
        None => {
            return fail(
                &io,
                ErrorCode::HostUnreachable,
                format!("Can't find the address of {}.", info.address),
            )
        }
    };
    let program = Program {
        path: client_path.display().to_string(),
        args: vec![ip.to_string(), port.to_string()],
        env: vec![
            ("MOSH_KEY".into(), key),
            ("TERM".into(), "xterm-256color".into()),
        ],
        close_on_exit: false,
    };
    pty::run_program(Arc::clone(&io), program, p.cols, p.rows, cmds).await;
    let _ = SessionState::Closed; // (states come from the terminal runner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_line_is_found_among_other_output() {
        let out = "\r\nMOSH CONNECT 60001 4NeCCgvZFe2RnPgrcU1PQw\r\n\r\nmosh-server (mosh 1.4.0) [build mosh 1.4.0]\r\n";
        assert_eq!(
            parse_connect(out),
            Some((60001, "4NeCCgvZFe2RnPgrcU1PQw".into()))
        );
        assert_eq!(parse_connect("bash: mosh-server: command not found"), None);
        assert_eq!(parse_connect("MOSH CONNECT 0 4NeCCgvZFe2RnPgrcU1PQw"), None); // port 0
        assert_eq!(parse_connect("MOSH CONNECT 60001 short"), None); // not a key
        assert_eq!(parse_connect("MOSH CONNECT x 4NeCCgvZFe2RnPgrcU1PQw"), None);
    }
}
