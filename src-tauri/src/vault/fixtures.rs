//! Migration fixtures (test-plan 9.5): `tests/fixtures/db/v0001.db` .. `v0005.db` are real database files as an
//! app at that schema version would have left them, with a vault, a secret, a group, a host and history in each.
//! `every_fixture_upgrades_and_keeps_its_data` opens each one with the current code.
//!
//! A NEW MIGRATION MUST ADD ITS FIXTURE: add the version's changes to `fill`, then run
//!   cargo test -- --ignored write_the_fixture_for_the_newest_version
//! (it writes only the newest file, so older fixtures stay exactly as they shipped).

use std::path::PathBuf;

use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::kdf::KdfParams;
use super::{crypto, derive_key, make_key_check, random_salt, secrets, unlock};

const PASSWORD: &str = "fixture-password";

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/db")
}

fn file(version: usize) -> PathBuf {
    dir().join(format!("v{version:04}.db"))
}

/// How many migrations the app has now (each one needs a fixture).
fn latest() -> usize {
    crate::db::migration_count()
}

/// The schema as it was at `version`: the first `version` migrations only.
fn migrations_up_to(version: usize) -> Migrations<'static> {
    let all = crate::db::migration_sources();
    Migrations::new(all[..version].iter().map(|s| M::up(s)).collect())
}

/// Builds the file for one version: only SQL that version's schema understands.
fn fill(version: usize, conn: &mut Connection) {
    crate::db::configure(conn).unwrap();
    migrations_up_to(version).to_latest(conn).unwrap();

    let params_kdf = KdfParams::cheap();
    let salt = random_salt();
    let key = derive_key(PASSWORD.as_bytes(), &salt, &params_kdf).unwrap();
    // v5 adds the wrapped data key; before that the vault row has none.
    conn.execute(
        "INSERT INTO vaults (id, name, kind, kdf_salt, kdf_params, key_check, created_at, updated_at) VALUES ('v1', 'Personal', 'local', ?1, ?2, ?3, 1, 1)",
        params![salt.to_vec(), serde_json::to_string(&params_kdf).unwrap(), make_key_check(&key).unwrap()],
    )
    .unwrap();
    if version >= 5 {
        let data_key = super::random_data_key();
        conn.execute(
            "UPDATE vaults SET wrapped_data_key = ?1 WHERE id = 'v1'",
            params![super::wrap_data_key(&key, &data_key).unwrap()],
        )
        .unwrap();
    }
    let (nonce, ct) = crypto::seal(&key, b"sec1", b"hunter2").unwrap();
    conn.execute(
        "INSERT INTO secrets (id, vault_id, kind, nonce, ciphertext, created_at, updated_at) VALUES ('sec1', 'v1', 'password', ?1, ?2, 1, 1)",
        params![nonce, ct],
    )
    .unwrap();

    // A group with a host in it, and a host with its own profile and the saved password.
    conn.execute_batch(
        "INSERT INTO connection_profiles (id, vault_id, username, ssh_port, created_at, updated_at) VALUES ('pg', 'v1', 'deploy', 2200, 1, 1);
         INSERT INTO groups (id, vault_id, profile_id, label, created_at, updated_at) VALUES ('g1', 'v1', 'pg', 'Production', 1, 1);
         INSERT INTO connection_profiles (id, vault_id, username, password_secret_id, created_at, updated_at) VALUES ('p1', 'v1', 'root', 'sec1', 1, 1);
         INSERT INTO hosts (id, vault_id, group_id, profile_id, label, address, created_at, updated_at) VALUES ('h1', 'v1', 'g1', 'p1', 'web 1', '10.0.0.1', 1, 1);
         INSERT INTO command_history (id, host_id, command, ran_at) VALUES ('c1', 'h1', 'uptime', 1);
         INSERT INTO connection_history (id, vault_id, host_id, connected_at) VALUES ('ch1', 'v1', 'h1', 1);",
    )
    .unwrap();

    // The protocol column arrived in v2, serial in v3, mosh in v4: one host of each kind that version knows.
    if version >= 2 {
        conn.execute_batch(
            "INSERT INTO connection_profiles (id, vault_id, created_at, updated_at) VALUES ('p2', 'v1', 1, 1);
             INSERT INTO hosts (id, vault_id, profile_id, label, address, protocol, created_at, updated_at) VALUES ('h2', 'v1', 'p2', 'old router', '10.0.0.2', 'telnet', 1, 1);",
        )
        .unwrap();
    }
    if version >= 3 {
        conn.execute_batch(
            "INSERT INTO connection_profiles (id, vault_id, created_at, updated_at) VALUES ('p3', 'v1', 1, 1);
             INSERT INTO hosts (id, vault_id, profile_id, label, address, protocol, created_at, updated_at) VALUES ('h3', 'v1', 'p3', 'console', '/dev/ttyUSB0', 'serial', 1, 1);
             INSERT INTO serial_settings (host_id, baud) VALUES ('h3', 115200);",
        )
        .unwrap();
    }
    if version >= 4 {
        conn.execute_batch(
            "INSERT INTO connection_profiles (id, vault_id, created_at, updated_at) VALUES ('p4', 'v1', 1, 1);
             INSERT INTO hosts (id, vault_id, profile_id, label, address, protocol, created_at, updated_at) VALUES ('h4', 'v1', 'p4', 'roaming', '10.0.0.4', 'mosh', 1, 1);",
        )
        .unwrap();
    }
}

/// Run by hand when a migration is added (see the top of this file). Writes only the files that are missing, so
/// an older version's file is never rewritten.
#[test]
#[ignore = "writes fixture files; run by hand when adding a migration"]
fn write_the_fixture_for_the_newest_version() {
    std::fs::create_dir_all(dir()).unwrap();
    for version in 1..=latest() {
        let path = file(version);
        if path.exists() {
            continue;
        }
        let mut conn = Connection::open(&path).unwrap();
        // One plain file, no -wal sidecar to commit.
        conn.pragma_update(None, "journal_mode", "DELETE").ok();
        fill(version, &mut conn);
    }
}

#[test]
fn every_migration_has_its_fixture() {
    for v in 1..=latest() {
        assert!(
            file(v).exists(),
            "migration {v} has no fixture: add it to fill() in vault/fixtures.rs and run the ignored test there"
        );
    }
}

#[test]
fn every_fixture_upgrades_and_keeps_its_data() {
    for v in 1..=latest() {
        let tmp =
            std::env::temp_dir().join(format!("dalan-fixture-{}-v{v}.db", std::process::id()));
        std::fs::copy(file(v), &tmp).unwrap();
        // The app's own open(): WAL, pragmas, and every newer migration.
        let conn = crate::db::open(&tmp).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version as usize,
            latest(),
            "v{v} did not reach the latest version"
        );
        let broken = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count();
        assert_eq!(broken, 0, "v{v} has broken foreign keys after upgrading");

        // The password still opens it, and the saved secret still decrypts.
        let s = unlock(&conn, PASSWORD).unwrap_or_else(|e| panic!("v{v}: {e:?}"));
        assert_eq!(
            secrets::get(&conn, &s, "sec1").unwrap().as_slice(),
            b"hunter2",
            "v{v}"
        );
        if v < 5 {
            assert_eq!(
                *s.data_key, *s.key,
                "v{v}: an old vault adopts its key as the data key"
            );
        }

        // The app's own commands read it.
        let tree = crate::hosts::tree(&conn, "v1").unwrap();
        let labels: Vec<_> = tree.hosts.iter().map(|h| h.label.as_str()).collect();
        let expect = 1 + usize::from(v >= 2) + usize::from(v >= 3) + usize::from(v >= 4);
        assert_eq!(labels.len(), expect, "v{v}: {labels:?}");
        assert!(labels.contains(&"web 1"), "v{v}");
        assert_eq!(tree.groups.len(), 1, "v{v}");
        let web = tree.hosts.iter().find(|h| h.label == "web 1").unwrap();
        assert_eq!(
            web.protocol.as_str(),
            "ssh",
            "v{v}: hosts from before the protocol column are SSH"
        );
        let history = crate::history::list(&conn, &s, Some("h1"), None, 10).unwrap();
        assert_eq!(history.len(), 1, "v{v}");
        assert_eq!(history[0].command, "uptime", "v{v}");
        drop(conn);
        std::fs::remove_file(&tmp).ok();
    }
}
