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
