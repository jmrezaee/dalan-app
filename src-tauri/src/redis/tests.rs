//! Redis against the real server in Docker (`scripts/test-servers.sh up`; service `redis` in tests/servers/compose.yml).
//! Ignored unless asked for, like the SSH real-server tests: `cargo test -- --ignored`.
//! The direct route uses the published port (6390); the "through a host" route signs in to the `a` container over
//! SSH and reaches `redis:6379` from there, which is the path a production database takes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zeroize::Zeroizing;

use super::conn::{self, Config, Emit, RedisEvent, RedisStepState};
use super::store::{
    self, RedisDatabaseInput, RedisEnvironment, RedisKind, RedisRoute, RedisWriteMode,
};
use super::transport::Target;
use crate::error::ErrorCode;
use crate::hosts;
use crate::ssh::tests::{harness, host_input};
use crate::ssh::{PromptAnswer, PromptKind, Prompts, SessionEvent};

const DIRECT_PORT: u16 = 6390;
const PASSWORD: &str = "test-redis";

fn direct(password: Option<&str>, username: Option<&str>) -> Config {
    Config {
        name: "test".into(),
        route: RedisRoute::Direct,
        host_id: None,
        kind: RedisKind::Single,
        target: Target {
            address: "127.0.0.1".into(),
            port: DIRECT_PORT,
            tls: false,
            tls_server_name: None,
        },
        username: username.map(String::from),
        password: password.map(|p| Zeroizing::new(p.to_string())),
        db: 0,
    }
}

/// An emitter that keeps every event, and the list it writes to.
fn recorder() -> (Emit, Arc<Mutex<Vec<RedisEvent>>>) {
    let seen: Arc<Mutex<Vec<RedisEvent>>> = Arc::default();
    let sink = Arc::clone(&seen);
    (Arc::new(move |e| sink.lock().unwrap().push(e)), seen)
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_direct_signs_in_and_reads_the_servers_details() {
    let h = harness(None).await;
    let (emit, seen) = recorder();
    let c = conn::connect(
        &h.state.store(),
        &direct(Some(PASSWORD), None),
        &Arc::new(Prompts::default()),
        &emit,
    )
    .await
    .expect("the test Redis should accept its password");
    assert!(c.info.version.starts_with('7'), "{}", c.info.version);
    assert_eq!(c.info.mode, "standalone");
    assert!(!c.info.replica);
    assert!(c.info.resp3, "Redis 7 speaks RESP3");
    assert_eq!(c.info.db_count, 16);
    // Two steps for a direct database, both done, in order.
    let events = seen.lock().unwrap().clone();
    assert!(
        matches!(&events[0], RedisEvent::Plan { steps } if steps.len() == 2 && steps[0].label == "Reach Redis at 127.0.0.1:6390")
    );
    let done: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            RedisEvent::Step {
                index,
                state: RedisStepState::Done,
                ..
            } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(done, vec![0, 1]);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_a_wrong_or_missing_password_is_auth_failed() {
    let h = harness(None).await;
    let (emit, seen) = recorder();
    for cfg in [
        direct(Some("not-the-password"), None),
        direct(None, None),
        direct(Some("nope"), Some("app-ro")),
    ] {
        let err = match conn::connect(&h.state.store(), &cfg, &Arc::new(Prompts::default()), &emit)
            .await
        {
            Ok(_) => panic!("a wrong password must not sign in"),
            Err(e) => e,
        };
        assert_eq!(err.code, ErrorCode::RedisAuthFailed, "{}", err.message);
    }
    // The failed step is the sign-in (index 1), never "reach".
    assert!(seen.lock().unwrap().iter().any(|e| matches!(
        e,
        RedisEvent::Step {
            index: 1,
            state: RedisStepState::Failed,
            ..
        }
    )));
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_an_acl_user_signs_in() {
    let h = harness(None).await;
    let (emit, _) = recorder();
    let c = conn::connect(
        &h.state.store(),
        &direct(Some("ro-pass"), Some("app-ro")),
        &Arc::new(Prompts::default()),
        &emit,
    )
    .await
    .expect("the read-only user should sign in");
    assert!(c.info.version.starts_with('7'));
}

/// Makes the harness's host the `a` container (password sign-in), so "through a host" has a real SSH server.
async fn host_is_container_a(h: &crate::ssh::tests::Harness) {
    let a = crate::testservers::server("a");
    let mut i = host_input("a", a.port.expect("a has a port on this computer"), None);
    i.id = Some(h.host_id.clone());
    i.username = a.user.clone();
    i.password = a.password.clone();
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &i))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_through_a_host_asks_the_hosts_question_then_reaches_redis_from_there() {
    let h = harness(None).await;
    host_is_container_a(&h).await;
    let prompts = Arc::new(Prompts::default());
    // The host key is new: trust it when the SSH sign-in asks, the way a person would in the tab.
    let answering = Arc::clone(&prompts);
    let emit: Emit = Arc::new(move |e| {
        if let RedisEvent::Host {
            event: SessionEvent::Prompt { prompt_id, kind },
        } = e
        {
            assert!(
                matches!(kind, PromptKind::HostKeyNew { .. }),
                "only the new host key should be asked"
            );
            answering
                .answer(
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: true,
                    },
                )
                .unwrap();
        }
    });
    let cfg = Config {
        route: RedisRoute::Host,
        host_id: Some(h.host_id.clone()),
        // As seen from the host: the Docker network's name for the Redis container.
        target: Target {
            address: "redis".into(),
            port: 6379,
            tls: false,
            tls_server_name: None,
        },
        ..direct(Some(PASSWORD), None)
    };
    let c = tokio::time::timeout(
        Duration::from_secs(60),
        conn::connect(&h.state.store(), &cfg, &prompts, &emit),
    )
    .await
    .expect("connecting through the host took too long")
    .expect("Redis should be reachable from the host");
    assert!(c.info.version.starts_with('7'));

    // The same route to an address nothing listens on says which host it tried from.
    let nowhere = Config {
        route: RedisRoute::Host,
        host_id: Some(h.host_id.clone()),
        target: Target {
            address: "redis".into(),
            port: 1,
            tls: false,
            tls_server_name: None,
        },
        ..direct(Some(PASSWORD), None)
    };
    let err = match conn::connect(&h.state.store(), &nowhere, &prompts, &emit).await {
        Ok(_) => panic!("nothing listens on port 1"),
        Err(e) => e,
    };
    assert_eq!(err.code, ErrorCode::RedisUnreachable);
    assert!(
        err.message.contains("Can't reach Redis from"),
        "{}",
        err.message
    );
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_a_saved_database_opens_selects_and_closes() {
    let h = harness(None).await;
    let saved = h
        .state
        .store()
        .with_db_session(|c, s| {
            store::save(
                c,
                s,
                RedisDatabaseInput {
                    id: None,
                    name: "docker".into(),
                    environment: RedisEnvironment::None,
                    route: RedisRoute::Direct,
                    host_id: None,
                    address: "127.0.0.1".into(),
                    port: DIRECT_PORT,
                    kind: RedisKind::Single,
                    sentinel_master: None,
                    username: None,
                    password: Some(PASSWORD.into()),
                    clear_password: false,
                    tls: false,
                    tls_server_name: None,
                    write_mode: RedisWriteMode::Confirm,
                    default_db: 0,
                    key_separator: ":".into(),
                },
            )
        })
        .await
        .unwrap();
    let (emit, seen) = recorder();
    let id = h
        .state
        .redis
        .open(h.state.store(), saved.id.clone(), emit)
        .unwrap();
    let wait_for = |what: fn(&RedisEvent) -> bool| {
        let seen = Arc::clone(&seen);
        async move {
            for _ in 0..200 {
                if seen.lock().unwrap().iter().any(what) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("the event never arrived: {:?}", seen.lock().unwrap());
        }
    };
    wait_for(|e| matches!(e, RedisEvent::Ready { .. })).await;
    let db3 = h.state.redis.select(&id, 3).await.unwrap();
    assert_eq!(db3.index, 3);
    h.state.redis.close(&id);
    wait_for(|e| matches!(e, RedisEvent::Closed)).await;
    assert_eq!(h.state.redis.live_count(), 0);
    assert_eq!(
        h.state.redis.select(&id, 0).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    // Opening it stamped "last opened".
    let again = h
        .state
        .store()
        .with_db_session(move |c, s| store::get(c, &s.vault_id, &saved.id))
        .await
        .unwrap();
    assert!(again.last_opened_at.is_some());
}

// ---- R2: the browser's read side against the real server

use super::browse::{self, RedisBytes, RedisValuePage};

fn t(s: &str) -> RedisBytes {
    RedisBytes::Text { text: s.into() }
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_redis_scan_key_info_and_every_types_value_in_pages() {
    let h = harness(None).await;
    let (emit, _) = recorder();
    let c = conn::connect(
        &h.state.store(),
        &direct(Some(PASSWORD), None),
        &Arc::new(Prompts::default()),
        &emit,
    )
    .await
    .unwrap();
    let mut r = c.conn.clone();
    // Our own corner of the keyspace, so the test never touches anything else and can run twice.
    let p = format!("dalan-test:{}:", uuid::Uuid::now_v7());
    let k = |name: &str| format!("{p}{name}");
    let binary_key: Vec<u8> = [p.as_bytes(), &[0xff, 0x00, 0x41]].concat();
    let mut seed = redis::pipe();
    seed.cmd("SET")
        .arg(k("str"))
        .arg("Back at 14:30 UTC.")
        .arg("PX")
        .arg(600_000);
    seed.cmd("HSET")
        .arg(k("hash"))
        .arg("name")
        .arg("Sara Moradi")
        .arg("seats")
        .arg(12);
    seed.cmd("RPUSH")
        .arg(k("list"))
        .arg("sku-8841")
        .arg("sku-1207")
        .arg("sku-5530");
    seed.cmd("SADD")
        .arg(k("set"))
        .arg("billing")
        .arg("editor")
        .arg("owner");
    let mut zadd = redis::cmd("ZADD");
    zadd.arg(k("zset"));
    for i in 0..600 {
        zadd.arg(i).arg(format!("team:{i:04}"));
    }
    seed.add_command(zadd);
    for i in 0..3 {
        seed.cmd("XADD")
            .arg(k("stream"))
            .arg("*")
            .arg("order")
            .arg(5500 + i)
            .arg("status")
            .arg("paid");
    }
    seed.cmd("SET")
        .arg(binary_key.as_slice())
        .arg(&[0x00u8, 0x9f, 0x92, 0x96][..]);
    seed.cmd("SET").arg(k("big")).arg(vec![b'x'; 1_100_000]);
    seed.query_async::<()>(&mut r).await.unwrap();

    // SCAN in small pages until the cursor comes back to "0": every key of ours shows up once, with its type.
    let pattern = format!("{p}*");
    let mut cursor = String::new();
    let mut found = std::collections::BTreeMap::new();
    for _ in 0..10_000 {
        let page = browse::scan(&mut r, &cursor, &pattern, None, 10)
            .await
            .unwrap();
        for row in page.keys {
            found.insert(row.key.as_bytes().to_vec(), (row.key_type, row.ttl_ms));
        }
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(found.len(), 8, "{found:?}");
    let type_of = |name: &str| found[k(name).as_bytes()].0.clone();
    for (name, ty) in [
        ("str", "string"),
        ("hash", "hash"),
        ("list", "list"),
        ("set", "set"),
        ("zset", "zset"),
        ("stream", "stream"),
    ] {
        assert_eq!(type_of(name), ty);
    }
    assert!(
        found[k("str").as_bytes()]
            .1
            .is_some_and(|ms| ms > 0 && ms <= 600_000),
        "the string expires"
    );
    assert_eq!(
        found[k("hash").as_bytes()].1,
        None,
        "the hash has no expiry"
    );
    assert!(
        found.contains_key(&binary_key),
        "a binary key is listed by its bytes"
    );

    // The type filter is applied by the server.
    let mut hashes = Vec::new();
    let mut cursor = String::new();
    loop {
        let page = browse::scan(&mut r, &cursor, &pattern, Some("hash"), 500)
            .await
            .unwrap();
        hashes.extend(page.keys);
        cursor = page.cursor;
        if cursor == "0" {
            break;
        }
    }
    assert_eq!(hashes.len(), 1);
    assert_eq!(hashes[0].key, t(&k("hash")));

    // Key info: the size is in the type's own unit.
    assert_eq!(
        browse::key_info(&mut r, &t(&k("hash")))
            .await
            .unwrap()
            .length,
        2
    );
    assert_eq!(
        browse::key_info(&mut r, &t(&k("zset")))
            .await
            .unwrap()
            .length,
        600
    );
    assert_eq!(
        browse::key_info(&mut r, &t(&k("big")))
            .await
            .unwrap()
            .length,
        1_100_000
    );
    assert_eq!(
        browse::key_info(&mut r, &t(&k("gone")))
            .await
            .unwrap()
            .key_type,
        "none"
    );

    // Values, one type at a time.
    match browse::value_page(&mut r, &t(&k("str")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::String {
            value,
            total_bytes,
            truncated,
        } => {
            assert_eq!(value, t("Back at 14:30 UTC."));
            assert_eq!(total_bytes, 18);
            assert!(!truncated);
        }
        other => panic!("{other:?}"),
    }
    match browse::value_page(&mut r, &t(&k("hash")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::Hash { fields, next } => {
            assert_eq!(fields.len(), 2);
            assert!(fields
                .iter()
                .any(|f| f.field == t("seats") && f.value == t("12")));
            assert_eq!(next, None);
        }
        other => panic!("{other:?}"),
    }
    match browse::value_page(&mut r, &t(&k("list")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::List { start, items, next } => {
            assert_eq!(start, 0);
            assert_eq!(items, vec![t("sku-8841"), t("sku-1207"), t("sku-5530")]);
            assert_eq!(next, None);
        }
        other => panic!("{other:?}"),
    }
    match browse::value_page(&mut r, &t(&k("set")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::Set { members, .. } => assert_eq!(members.len(), 3),
        other => panic!("{other:?}"),
    }
    // The sorted set has 600 members: 500 highest first, then the last 100.
    let next = match browse::value_page(&mut r, &t(&k("zset")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::Zset { members, next } => {
            assert_eq!(members.len(), 500);
            assert_eq!(members[0].score, "599");
            assert_eq!(members[0].member, t("team:0599"));
            next.expect("there is a second page")
        }
        other => panic!("{other:?}"),
    };
    match browse::value_page(&mut r, &t(&k("zset")), &next, false)
        .await
        .unwrap()
    {
        RedisValuePage::Zset { members, next } => {
            assert_eq!(members.len(), 100);
            assert_eq!(members[99].score, "0");
            assert_eq!(next, None);
        }
        other => panic!("{other:?}"),
    }
    match browse::value_page(&mut r, &t(&k("stream")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::Stream { entries, next } => {
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].fields[0].value, t("5502"), "newest first");
            assert_eq!(next, None);
        }
        other => panic!("{other:?}"),
    }
    // A binary key and its binary value come through as bytes, untouched.
    match browse::value_page(
        &mut r,
        &RedisBytes::Binary {
            bytes: binary_key.clone(),
        },
        "",
        false,
    )
    .await
    .unwrap()
    {
        RedisValuePage::String { value, .. } => assert_eq!(
            value,
            RedisBytes::Binary {
                bytes: vec![0x00, 0x9f, 0x92, 0x96]
            }
        ),
        other => panic!("{other:?}"),
    }
    // Over 1 MB: the first 64 KB, flagged; "Load all" brings the rest.
    match browse::value_page(&mut r, &t(&k("big")), "", false)
        .await
        .unwrap()
    {
        RedisValuePage::String {
            value,
            total_bytes,
            truncated,
        } => {
            assert!(truncated);
            assert_eq!(total_bytes, 1_100_000);
            assert_eq!(value.as_bytes().len(), 64 * 1024);
        }
        other => panic!("{other:?}"),
    }
    match browse::value_page(&mut r, &t(&k("big")), "", true)
        .await
        .unwrap()
    {
        RedisValuePage::String {
            value, truncated, ..
        } => {
            assert!(!truncated);
            assert_eq!(value.as_bytes().len(), 1_100_000);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        browse::value_page(&mut r, &t(&k("gone")), "", false)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    // Tidy up our corner.
    let mut del = redis::cmd("DEL");
    for key in found.keys() {
        del.arg(key.as_slice());
    }
    del.query_async::<()>(&mut r).await.unwrap();
}
