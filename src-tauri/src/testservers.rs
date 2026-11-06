//! The Docker test servers (`scripts/test-servers.sh up`) as the real-server tests see them. The script writes
//! `.test-servers/servers.json`; these helpers read it on every call, so a `rotate` that changes a fingerprint is seen.
//! Only compiled for tests. Nothing here touches a machine that is not a throwaway container.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

/// Every field the script writes; tests read the ones they need.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Server {
    pub name: String,
    pub host: String,
    /// The port on this computer (`None` for a server only reachable inside the Docker network).
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    /// Path of a private key the server accepts, relative to the repository root.
    pub key: Option<String>,
    pub key_with_passphrase: Option<String>,
    /// The name and port other containers use (what a jump host dials).
    pub net_host: String,
    pub net_port: u16,
    /// ed25519 host key fingerprint (`SHA256:...`).
    pub fingerprint: Option<String>,
}

/// The repository root, one level above `src-tauri`.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn all() -> HashMap<String, Server> {
    let path = root().join(".test-servers/servers.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing. Start the servers first: scripts/test-servers.sh up",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("servers.json is not valid")
}

pub fn server(name: &str) -> Server {
    all()
        .remove(name)
        .unwrap_or_else(|| panic!("no test server named {name} in servers.json"))
}

/// The local port of a server.
pub fn port(name: &str) -> u16 {
    server(name)
        .port
        .unwrap_or_else(|| panic!("{name} has no port on this computer"))
}

/// Runs `scripts/test-servers.sh` and returns what it printed.
pub fn script(args: &[&str]) -> String {
    let out = Command::new(root().join("scripts/test-servers.sh"))
        .args(args)
        .current_dir(root())
        .output()
        .expect("couldn't run scripts/test-servers.sh");
    assert!(
        out.status.success(),
        "test-servers.sh {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The current "Verification code" of the `kbd` server.
pub fn otp() -> String {
    script(&["otp"])
}
