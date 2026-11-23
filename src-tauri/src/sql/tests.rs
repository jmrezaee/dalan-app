//! PostgreSQL browse tests (D2). The ignored ones need the Docker test servers: `scripts/test-servers.sh up`.
//! They seed schema `d2` in the `shop` database of the `pg` container and read it back through our own code.

use std::collections::HashSet;

use zeroize::Zeroizing;

use super::pg::{Login, PageRequest, PgConn, SortKey};
use super::values::CellKind;
use crate::dbs::spike::sign_in_to_a;
use crate::dbs::transport::{self, Target, TlsMode};
use crate::dbs::Engine;
use crate::error::ErrorCode;
use crate::ssh::tests::harness;

const SEED: &str = r#"
drop schema if exists d2 cascade;
create schema d2;
create type d2.status as enum ('open', 'paid', 'void');
create table d2.customers (id bigint primary key, name text not null, meta jsonb);
create table d2.orders (id bigint primary key, customer_id bigint not null references d2.customers(id),
  status d2.status not null, total numeric(12,2), placed_at timestamptz, note text);
create table d2.order_items (order_id bigint references d2.orders(id), line_no int, sku text, primary key (order_id, line_no));
create table d2.no_key (a int, b text);
create table d2.types (id int primary key, big bigint, dec numeric, f float8, b bytea, arr int[], u uuid, iv interval, j jsonb, long text);
create table d2.log (n int);
create table d2.e1 (id int primary key, name text, qty int);
create table d2.e2 (id int primary key, name text, qty int);
create table d2.e3 (id int primary key, name text, qty int);
create table d2.e4 (id int primary key, name text, qty int);
create table d2.e5 (id int primary key, name text, qty int);
create table d2.e6 (a text, b int, "select" text, j json, primary key (a, b));
insert into d2.e6 values ('x', 1, 's', '{"k": 1}'), ('x', 2, 't', null);
insert into d2.e1 select g, 'n' || g, g * 10 from generate_series(1, 5) g;
insert into d2.e2 select * from d2.e1;
insert into d2.e3 select * from d2.e1;
insert into d2.e4 select * from d2.e1;
insert into d2.e5 select * from d2.e1;
create function d2.bump() returns boolean language sql as 'insert into d2.log values (1) returning true';
create view d2.open_orders as select * from d2.orders where status = 'open';
insert into d2.customers select g, 'customer ' || g, jsonb_build_object('vip', g % 7 = 0) from generate_series(1, 50) g;
insert into d2.orders select g, 1 + g % 50, (array['open','paid','void'])[1 + g % 3]::d2.status, g * 1.25,
  timestamptz '2026-01-01 00:00:00+00' + g * interval '1 hour', case when g % 10 = 0 then null else 'note ' || g end
  from generate_series(1, 1200) g;
insert into d2.order_items select o, l, 'sku-' || o || '-' || l from generate_series(1, 30) o, generate_series(1, 4) l;
insert into d2.no_key values (1, 'a'), (2, 'b');
insert into d2.types values (1, 9223372036854775807, 12345678901234567890.123456789, 1.5, '\xdeadbeef', '{1,2,3}',
  '3f333df6-90a4-4fda-8dd3-9485d27cee36', '1 day 02:03:04', '{"a": [1, 2]}', repeat('x', 70000));
"#;

/// Seeds once per test run: the tests run side by side and read the same schema.
fn seed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(seed_now);
}

fn seed_now() {
    let mut child = std::process::Command::new("docker")
        .args([
            "exec",
            "-i",
            "dalan-test-pg",
            "psql",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-U",
            "app",
            "-d",
            "shop",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("docker");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(SEED.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success(), "seeding failed");
}

/// A way back to the test server on this computer, for the cancel request.
fn direct_reopen() -> super::pg::Reopen {
    std::sync::Arc::new(|| {
        Box::pin(async {
            let target = Target {
                address: "127.0.0.1".into(),
                port: 15432,
                tls: false,
                tls_server_name: None,
            };
            transport::direct_for(Engine::Postgres, &target).await
        })
    })
}

async fn connect_direct(mode: TlsMode) -> Result<PgConn, crate::error::AppError> {
    let target = Target {
        address: "127.0.0.1".into(),
        port: 15432,
        tls: false,
        tls_server_name: None,
    };
    let stream = transport::direct_for(Engine::Postgres, &target).await?;
    let login = Login {
        user: "app".into(),
        password: Some(Zeroizing::new("test-pg".into())),
        database: Some("shop".into()),
        tls: mode,
        server_name: "127.0.0.1".into(),
    };
    PgConn::connect(stream, login, direct_reopen(), |_| {})
        .await
        .map(|(c, _)| c)
}

fn req(table: &str) -> PageRequest {
    PageRequest {
        schema: "d2".into(),
        table: table.into(),
        filter: String::new(),
        sort: vec![],
        columns: vec![],
        cursor: None,
        page_size: 500,
    }
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_the_tree_the_columns_and_the_keys() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let schemas = c.objects(&[]).await.unwrap();
    assert!(schemas.iter().any(|o| o.name == "d2"));
    assert!(!schemas
        .iter()
        .any(|o| o.name.starts_with("pg_") || o.name == "information_schema"));
    let rels = c.objects(&["d2".into()]).await.unwrap();
    let names: Vec<_> = rels.iter().map(|o| (o.name.as_str(), o.kind)).collect();
    assert!(names.contains(&("orders", super::pg::ObjectKind::Table)));
    assert!(names.contains(&("open_orders", super::pg::ObjectKind::View)));

    let info = c.table_info("d2", "orders").await.unwrap();
    let id = info.columns.iter().find(|c| c.name == "id").unwrap();
    assert_eq!(
        (id.pk_position, id.kind, id.nullable),
        (Some(1), CellKind::Int, false)
    );
    assert_eq!(
        info.columns
            .iter()
            .find(|c| c.name == "status")
            .unwrap()
            .kind,
        CellKind::Enum
    );
    assert_eq!(
        info.columns
            .iter()
            .find(|c| c.name == "total")
            .unwrap()
            .kind,
        CellKind::Decimal
    );
    assert_eq!(info.references.len(), 1);
    assert_eq!(
        (
            info.references[0].table.as_str(),
            &info.references[0].columns[..]
        ),
        ("customers", &["customer_id".to_string()][..])
    );
    assert_eq!(info.referenced_by.len(), 1);
    assert_eq!(info.referenced_by[0].table, "order_items");
    let items = c.table_info("d2", "order_items").await.unwrap();
    let pk: Vec<_> = items
        .columns
        .iter()
        .filter_map(|c| c.pk_position.map(|p| (p, c.name.as_str())))
        .collect();
    assert_eq!(pk.len(), 2);
    assert!(c.table_info("d2", "nope").await.is_err());
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_keyset_paging_walks_every_row_once_and_offset_paging_follows_a_sort() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    // Single-column key: 1200 rows in pages of 500.
    let mut seen = Vec::new();
    let mut r = req("orders");
    loop {
        let page = c.table_page(&r).await.unwrap();
        assert!(page.keyset);
        seen.extend(
            page.rows
                .iter()
                .map(|row| row[0].v.clone().unwrap().parse::<i64>().unwrap()),
        );
        match page.next {
            Some(n) => r.cursor = Some(n),
            None => break,
        }
    }
    assert_eq!(seen.len(), 1200);
    assert_eq!(seen.iter().copied().collect::<HashSet<_>>().len(), 1200);
    assert!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "pages come in key order"
    );

    // Composite key: 120 rows in pages of 50, in (order_id, line_no) order.
    let mut r = req("order_items");
    r.page_size = 50;
    let mut rows = Vec::new();
    loop {
        let page = c.table_page(&r).await.unwrap();
        rows.extend(page.rows.iter().map(|x| {
            (
                x[0].v.clone().unwrap().parse::<i64>().unwrap(),
                x[1].v.clone().unwrap().parse::<i32>().unwrap(),
            )
        }));
        match page.next {
            Some(n) => r.cursor = Some(n),
            None => break,
        }
    }
    assert_eq!(rows.len(), 120);
    assert!(rows.windows(2).all(|w| w[0] < w[1]));

    // A sort uses offset paging, and a table with no key does too.
    let mut r = req("orders");
    r.page_size = 100;
    r.sort = vec![SortKey {
        column: "total".into(),
        descending: true,
    }];
    let first = c.table_page(&r).await.unwrap();
    assert!(!first.keyset);
    assert_eq!(first.rows[0][0].v.as_deref(), Some("1200"));
    r.cursor = first.next;
    let second = c.table_page(&r).await.unwrap();
    assert_eq!(second.rows[0][0].v.as_deref(), Some("1100"));
    let nk = c.table_page(&req("no_key")).await.unwrap();
    assert_eq!(
        (nk.rows.len(), nk.keyset, nk.next.is_none()),
        (2, false, true)
    );

    // A column subset and a filter.
    let mut r = req("orders");
    r.columns = vec!["id".into(), "status".into()];
    r.filter = "status = 'open' and id <= 30".into();
    let page = c.table_page(&r).await.unwrap();
    assert_eq!(page.columns.len(), 2);
    assert_eq!(page.rows.len(), 10);
    assert_eq!(
        c.count("d2", "orders", "status = 'open'").await.unwrap(),
        400
    );
    assert_eq!(c.count("d2", "orders", "").await.unwrap(), 1200);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_every_value_keeps_its_digits_and_big_ones_arrive_cut_until_asked_for() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let page = c.table_page(&req("types")).await.unwrap();
    let row = &page.rows[0];
    let by = |name: &str| &row[page.columns.iter().position(|c| c.name == name).unwrap()];
    assert_eq!(
        (by("big").kind, by("big").v.as_deref()),
        (CellKind::Int, Some("9223372036854775807"))
    );
    assert_eq!(
        (by("dec").kind, by("dec").v.as_deref()),
        (CellKind::Decimal, Some("12345678901234567890.123456789"))
    );
    assert_eq!(by("b").kind, CellKind::Bytes);
    assert_eq!(by("b").v.as_deref(), Some("\\xdeadbeef"));
    assert_eq!(
        (by("arr").kind, by("arr").v.as_deref()),
        (CellKind::Array, Some("{1,2,3}"))
    );
    assert_eq!(by("u").kind, CellKind::Uuid);
    assert_eq!(by("iv").kind, CellKind::Interval);
    assert_eq!(
        (by("j").kind, by("j").v.as_deref()),
        (CellKind::Json, Some("{\"a\": [1, 2]}"))
    );
    assert!(by("long").cut);
    let full = c
        .cell("d2", "types", &[("id".into(), "1".into())], "long")
        .await
        .unwrap();
    assert_eq!((full.cut, full.v.unwrap().len()), (false, 70000));
    // A NULL is a NULL, not an empty string.
    let orders = c.table_page(&req("orders")).await.unwrap();
    let note = orders
        .columns
        .iter()
        .position(|c| c.name == "note")
        .unwrap();
    assert_eq!(orders.rows[9][note].kind, CellKind::Null); // id 10
    assert!(
        c.cell("d2", "types", &[("long".into(), "x".into())], "id")
            .await
            .is_err(),
        "only key columns pick a row"
    );
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_filter_can_not_write_and_a_bad_one_is_refused_before_it_is_sent() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    // A function that inserts: the read-only transaction stops it, in the server's own words.
    let mut r = req("orders");
    r.filter = "d2.bump()".into();
    let err = c.table_page(&r).await.expect_err("must be refused");
    assert_eq!(err.code, ErrorCode::DbRefused);
    assert!(err.message.contains("read-only"), "{}", err.message);
    let n = c.count("d2", "log", "").await.unwrap();
    assert_eq!(n, 0, "nothing was written");
    // Two statements, or a trailing clause, never reach the server.
    for bad in ["1 = 1; drop table d2.orders", "1 = 1 order by id", "id ="] {
        r.filter = bad.into();
        assert_eq!(
            c.table_page(&r).await.err().unwrap().code,
            ErrorCode::Validation,
            "{bad}"
        );
    }
    // A column that isn't there, and a sort on one.
    let mut r = req("orders");
    r.columns = vec!["nope".into()];
    assert!(c.table_page(&r).await.is_err());
    let mut r = req("orders");
    r.sort = vec![SortKey {
        column: "x\"; drop table d2.orders; --".into(),
        descending: false,
    }];
    assert!(c.table_page(&r).await.is_err());
    assert!(
        c.table_page(&req("orders")).await.is_ok(),
        "the table is still there"
    );
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_tls_require_encrypts_verify_refuses_an_unknown_authority_and_a_wrong_password_says_so(
) {
    let c = connect_direct(TlsMode::Require).await.unwrap();
    let encrypted: bool = c
        .count(
            "pg_catalog",
            "pg_stat_ssl",
            "pid = pg_backend_pid() and ssl",
        )
        .await
        .unwrap()
        == 1;
    assert!(encrypted, "the server says this very connection uses TLS");
    // This computer doesn't trust the test CA.
    let err = connect_direct(TlsMode::Verify).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::DbTlsUntrusted, "{}", err.message);
    // Wrong password.
    let target = Target {
        address: "127.0.0.1".into(),
        port: 15432,
        tls: false,
        tls_server_name: None,
    };
    let stream = transport::direct_for(Engine::Postgres, &target)
        .await
        .unwrap();
    let login = Login {
        user: "app".into(),
        password: Some(Zeroizing::new("nope".into())),
        database: Some("shop".into()),
        tls: TlsMode::Off,
        server_name: String::new(),
    };
    let err = PgConn::connect(stream, login, direct_reopen(), |_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, ErrorCode::DbAuthFailed);
    // Nothing listening.
    let dead = Target {
        address: "127.0.0.1".into(),
        port: 1,
        tls: false,
        tls_server_name: None,
    };
    assert_eq!(
        transport::direct_for(Engine::Postgres, &dead)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::DbUnreachable
    );
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_through_a_host_the_same_pages_come_back() {
    seed();
    let h = harness(None).await;
    let ssh = sign_in_to_a(&h).await;
    let target = Target {
        address: "pg".into(),
        port: 5432,
        tls: false,
        tls_server_name: None,
    };
    let stream = transport::through_host_for(Engine::Postgres, &ssh, &target)
        .await
        .unwrap();
    let login = Login {
        user: "app".into(),
        password: Some(Zeroizing::new("test-pg".into())),
        database: Some("shop".into()),
        tls: TlsMode::Require,
        server_name: "pg".into(),
    };
    let (c, info) = PgConn::connect(stream, login, direct_reopen(), |_| {})
        .await
        .unwrap();
    assert!(
        info.version.starts_with("PostgreSQL 16"),
        "{}",
        info.version
    );
    assert_eq!(
        (info.database.as_str(), info.user.as_str()),
        ("shop", "app")
    );
    let page = c.table_page(&req("orders")).await.unwrap();
    assert_eq!(page.rows.len(), 500);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_request_dropped_half_way_leaves_the_connection_usable_and_a_partial_key_is_refused(
) {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    // A slow page that the caller gives up on (a tab closed, a stale request): the transaction is left open on the
    // connection until the next scope ends it.
    let mut slow = req("orders");
    slow.filter = "(select pg_sleep(2)) is not null".into();
    let gave_up =
        tokio::time::timeout(std::time::Duration::from_millis(150), c.table_page(&slow)).await;
    assert!(gave_up.is_err(), "the page should still have been running");
    assert!(
        c.table_page(&req("orders")).await.is_ok(),
        "the next request works"
    );
    assert_eq!(c.count("d2", "orders", "").await.unwrap(), 1200);
    // Composite key: both columns are needed to pick a row.
    let partial = c
        .cell(
            "d2",
            "order_items",
            &[("order_id".into(), "1".into())],
            "sku",
        )
        .await;
    assert!(partial.is_err());
    let whole = c
        .cell(
            "d2",
            "order_items",
            &[
                ("order_id".into(), "1".into()),
                ("line_no".into(), "2".into()),
            ],
            "sku",
        )
        .await
        .unwrap();
    assert_eq!(whole.v.as_deref(), Some("sku-1-2"));
}

// ---- D3: the editor's runs

use super::run::{pieces_for, FailKind, RunMode, RunOpts, SqlRunEvent};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// Runs a script and returns every event, in order.
async fn run(c: &PgConn, sql: &str, limit: u32, timeout_ms: u32) -> Vec<SqlRunEvent> {
    let pieces = pieces_for(sql, RunMode::All, None).unwrap();
    let out = Mutex::new(Vec::new());
    c.run_script(
        &pieces,
        &RunOpts { limit, timeout_ms },
        &AtomicBool::new(false),
        false,
        &|e| out.lock().unwrap().push(e),
    )
    .await
    .unwrap();
    out.into_inner().unwrap()
}

fn row_count(events: &[SqlRunEvent], index: u32) -> usize {
    events
        .iter()
        .map(|e| match e {
            SqlRunEvent::Rows { index: i, rows } if *i == index => rows.len(),
            _ => 0,
        })
        .sum()
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_script_streams_each_result_in_batches_and_stops_at_the_limit() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    // Two result sets; 1200 rows come in batches of 500, then the limit cuts the second one.
    let ev = run(
        &c,
        "select id, status::text, total from d2.orders order by id; select 1 as one, null::text as nothing",
        0,
        30_000,
    )
    .await;
    assert_eq!(row_count(&ev, 0), 1200);
    assert_eq!(row_count(&ev, 1), 1);
    let batches = ev
        .iter()
        .filter(|e| matches!(e, SqlRunEvent::Rows { index: 0, .. }))
        .count();
    assert_eq!(batches, 3, "500 + 500 + 200");
    assert!(matches!(ev.last(), Some(SqlRunEvent::Finished { .. })));
    // A limit: exactly that many rows, and `more` says there were others.
    let ev = run(&c, "select id from d2.orders order by id", 100, 30_000).await;
    assert_eq!(row_count(&ev, 0), 100);
    assert!(ev
        .iter()
        .any(|e| matches!(e, SqlRunEvent::Done { rows: 100, more: true, .. })));
    // At exactly the limit there is nothing more.
    let ev = run(&c, "select id from d2.customers order by id", 50, 30_000).await;
    assert!(ev
        .iter()
        .any(|e| matches!(e, SqlRunEvent::Done { rows: 50, more: false, .. })));
    // Values keep every digit; NULL is NULL; show and explain answer without a cursor.
    let ev = run(&c, "select big, dec from d2.types", 0, 30_000).await;
    let cell = ev
        .iter()
        .find_map(|e| match e {
            SqlRunEvent::Rows { rows, .. } => Some(rows[0].clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(cell[0].v.as_deref(), Some("9223372036854775807"));
    assert_eq!(cell[1].v.as_deref(), Some("12345678901234567890.123456789"));
    let ev = run(&c, "show server_version; explain select 1", 0, 30_000).await;
    assert_eq!(row_count(&ev, 0), 1);
    assert!(row_count(&ev, 1) >= 1);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_nothing_but_a_read_runs_and_a_refused_script_runs_nothing() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    for sql in [
        "insert into d2.log values (9)",
        "update d2.customers set name = 'x'",
        "delete from d2.log where n = 1",
        "create table d2.nope (a int)",
        "drop table d2.log",
        "truncate d2.log",
        "select d2.bump()", // a read in form, a write in effect: the read-only transaction stops it
        "begin",
        "set search_path = d2",
    ] {
        let ev = run(&c, sql, 0, 30_000).await;
        let stopped = ev.iter().any(|e| matches!(e, SqlRunEvent::Refused { .. }))
            || ev.iter().any(|e| matches!(e, SqlRunEvent::Failed { .. }));
        assert!(stopped, "{sql}");
        assert_eq!(row_count(&ev, 0), 0, "{sql}");
    }
    // One write in a script of reads: not even the reads run.
    let ev = run(&c, "select 1; insert into d2.log values (9); select 2", 0, 30_000).await;
    assert!(ev
        .iter()
        .any(|e| matches!(e, SqlRunEvent::Refused { index: 1, .. })));
    assert!(!ev.iter().any(|e| matches!(e, SqlRunEvent::Started { .. })));
    // Nothing was written by any of it.
    let ev = run(&c, "select count(*) from d2.log", 0, 30_000).await;
    let n = ev
        .iter()
        .find_map(|e| match e {
            SqlRunEvent::Rows { rows, .. } => rows[0][0].v.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(n, "0");
    // A bad statement fails with the server's own words and stops the script; the connection is still good.
    let ev = run(&c, "select 1; select * from d2.missing; select 3", 0, 30_000).await;
    assert!(ev.iter().any(|e| matches!(e, SqlRunEvent::Failed { index: 1, kind: FailKind::Error, message } if message.contains("missing"))));
    assert!(!ev.iter().any(|e| matches!(e, SqlRunEvent::Started { index: 2, .. })));
    assert_eq!(row_count(&run(&c, "select 1", 0, 30_000).await, 0), 1);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_long_statement_times_out_and_stop_cancels_it_from_a_second_connection() {
    seed();
    let c = std::sync::Arc::new(connect_direct(TlsMode::Off).await.unwrap());
    let ev = run(&c, "select pg_sleep(5)", 0, 1000).await;
    assert!(ev
        .iter()
        .any(|e| matches!(e, SqlRunEvent::Failed { kind: FailKind::Timeout, .. })), "{ev:?}");
    // Stop while it runs.
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let events = std::sync::Arc::new(Mutex::new(Vec::new()));
    let (c2, stop2, events2) = (c.clone(), stop.clone(), events.clone());
    let started = std::time::Instant::now();
    let job = tokio::spawn(async move {
        let pieces = pieces_for("select pg_sleep(30)", RunMode::All, None).unwrap();
        c2.run_script(
            &pieces,
            &RunOpts { limit: 0, timeout_ms: 60_000 },
            &stop2,
            false,
            &|e| events2.lock().unwrap().push(e),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    c.cancel_running().await.unwrap();
    job.await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "the sleep was not cut short");
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, SqlRunEvent::Failed { kind: FailKind::Cancelled, .. })));
    // The connection works afterwards.
    assert_eq!(row_count(&run(&c, "select 1", 0, 30_000).await, 0), 1);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_the_catalog_lists_tables_and_columns_for_completion() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let cat = c.catalog(false).await.unwrap();
    let orders = cat
        .tables
        .iter()
        .find(|t| t.schema == "d2" && t.name == "orders")
        .expect("d2.orders");
    assert_eq!(orders.columns[0], "id");
    assert!(orders.columns.contains(&"customer_id".to_string()));
    assert!(!cat.tables.iter().any(|t| t.schema == "pg_catalog"));
    // Cached until asked again.
    assert!(std::sync::Arc::ptr_eq(&cat, &c.catalog(false).await.unwrap()));
    assert!(!std::sync::Arc::ptr_eq(&cat, &c.catalog(true).await.unwrap()));
}

// ---- D4: writes, manual transactions and grid edits

use super::grid::{CellChange, GridEdits, KeyValue, RowEdit};

/// Like `run`, with the gate's permission to change things.
async fn run_w(c: &PgConn, sql: &str) -> Vec<SqlRunEvent> {
    let pieces = pieces_for(sql, RunMode::All, None).unwrap();
    let out = Mutex::new(Vec::new());
    c.run_script(&pieces, &RunOpts { limit: 0, timeout_ms: 30_000 }, &AtomicBool::new(false), true, &|e| out.lock().unwrap().push(e))
        .await
        .unwrap();
    out.into_inner().unwrap()
}

async fn scalar(c: &PgConn, sql: &str) -> String {
    let ev = run(c, sql, 0, 30_000).await;
    ev.iter()
        .find_map(|e| match e {
            SqlRunEvent::Rows { rows, .. } => rows[0][0].v.clone(),
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_confirmed_write_runs_in_auto_commit_and_a_preview_changes_nothing() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    // The preview asks the server and rolls back: the same count, and the rows are untouched.
    let pieces = pieces_for("update d2.e1 set qty = 0 where id <= 3; delete from d2.e1", RunMode::All, None).unwrap();
    let notes = c.preview(&pieces).await.unwrap();
    assert_eq!(notes[0].as_deref(), Some("This will change 3 rows."));
    assert_eq!(notes[1].as_deref(), Some("This will change 5 rows."));
    assert_eq!(scalar(&c, "select count(*) from d2.e1").await, "5");
    assert_eq!(scalar(&c, "select sum(qty) from d2.e1").await, "150");
    // Allowed to write: the statements commit one by one.
    let ev = run_w(&c, "update d2.e1 set qty = 0 where id <= 3; select count(*) from d2.e1; delete from d2.e1 where id = 5").await;
    assert!(ev.iter().any(|e| matches!(e, SqlRunEvent::Done { index: 0, affected: Some(a), .. } if *a == 3.0)));
    assert!(ev.iter().any(|e| matches!(e, SqlRunEvent::Done { index: 2, affected: Some(a), .. } if *a == 1.0)));
    assert_eq!(scalar(&c, "select count(*) from d2.e1").await, "4");
    assert_eq!(scalar(&c, "select sum(qty) from d2.e1").await, "40");
    // Without the gate's yes, a write is refused whatever the text says.
    let ev = run(&c, "delete from d2.e1", 0, 30_000).await;
    assert!(ev.iter().any(|e| matches!(e, SqlRunEvent::Refused { .. })));
    assert_eq!(scalar(&c, "select count(*) from d2.e1").await, "4");
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_manual_transaction_holds_its_changes_until_commit_or_rollback() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let other = connect_direct(TlsMode::Off).await.unwrap();
    c.txn_begin().await.unwrap();
    assert!(c.txn_begin().await.is_err(), "one at a time");
    run_w(&c, "update d2.e2 set qty = 99 where id = 1").await;
    let st = c.txn_state().await.unwrap();
    assert!(st.active && st.statements == 1 && !st.failed && st.locks >= 1, "{st:?}");
    // Another connection can't see it yet.
    assert_eq!(scalar(&other, "select qty from d2.e2 where id = 1").await, "10");
    // Reads in the transaction see it.
    assert_eq!(scalar(&c, "select qty from d2.e2 where id = 1").await, "99");
    c.txn_end(false).await.unwrap();
    assert!(!c.txn_state().await.unwrap().active);
    assert_eq!(scalar(&other, "select qty from d2.e2 where id = 1").await, "10");
    // Commit keeps it.
    c.txn_begin().await.unwrap();
    run_w(&c, "update d2.e2 set qty = 77 where id = 2").await;
    c.txn_end(true).await.unwrap();
    assert_eq!(scalar(&other, "select qty from d2.e2 where id = 2").await, "77");
    // A failing statement marks it: only rolling back is left.
    c.txn_begin().await.unwrap();
    run_w(&c, "select * from d2.missing").await;
    assert!(c.txn_state().await.unwrap().failed);
    c.txn_end(false).await.unwrap();
    assert_eq!(scalar(&c, "select 1").await, "1");
}

fn edit(id: &str, col: &str, old: Option<&str>, new: Option<&str>) -> RowEdit {
    RowEdit {
        key: vec![KeyValue { column: "id".into(), value: id.into() }],
        changes: vec![CellChange { column: col.into(), old: old.map(Into::into), new: new.map(Into::into) }],
        delete: false,
    }
}
fn edits(rows: Vec<RowEdit>) -> GridEdits {
    GridEdits { schema: "d2".into(), table: "e3".into(), rows }
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_grid_edits_commit_together_and_the_optimistic_check_stops_a_stale_row() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let stmts = c
        .grid_statements(&edits(vec![edit("1", "name", Some("n1"), Some("it's")), edit("2", "qty", Some("20"), None)]))
        .await
        .unwrap();
    assert!(stmts[0].shown.contains("'it''s'") && stmts[0].shown.contains("is not distinct from 'n1'"));
    assert_eq!(c.grid_commit(&stmts).await.unwrap(), 2);
    assert_eq!(scalar(&c, "select name from d2.e3 where id = 1").await, "it's");
    let ev = run(&c, "select qty is null from d2.e3 where id = 2", 0, 30_000).await;
    assert!(ev.iter().any(|e| matches!(e, SqlRunEvent::Rows { rows, .. } if rows[0][0].v.as_deref() == Some("t"))));
    // Someone else changed row 3 after it was loaded (old says 30, it is now 31): the whole commit is undone.
    run_w(&c, "update d2.e3 set qty = 31 where id = 3").await;
    let stmts = c
        .grid_statements(&edits(vec![edit("4", "qty", Some("40"), Some("41")), edit("3", "qty", Some("30"), Some("35"))]))
        .await
        .unwrap();
    let err = c.grid_commit(&stmts).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::SqlConflict);
    assert_eq!(scalar(&c, "select qty from d2.e3 where id = 4").await, "40", "the first statement was rolled back too");
    assert_eq!(scalar(&c, "select qty from d2.e3 where id = 3").await, "31");
    // A delete removes exactly one row; a second try finds nothing and is a conflict.
    let del = RowEdit { key: vec![KeyValue { column: "id".into(), value: "5".into() }], changes: vec![], delete: true };
    let stmts = c.grid_statements(&edits(vec![del.clone()])).await.unwrap();
    assert_eq!(c.grid_commit(&stmts).await.unwrap(), 1);
    assert_eq!(c.grid_commit(&stmts).await.unwrap_err().code, ErrorCode::SqlConflict);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_grid_edits_refuse_a_keyless_table_a_partial_key_and_a_changed_key() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let keyless = GridEdits { schema: "d2".into(), table: "no_key".into(), rows: vec![edit("1", "a", Some("1"), Some("2"))] };
    assert_eq!(c.grid_statements(&keyless).await.unwrap_err().code, ErrorCode::SqlNoKey);
    let partial = GridEdits {
        schema: "d2".into(),
        table: "order_items".into(),
        rows: vec![RowEdit { key: vec![KeyValue { column: "order_id".into(), value: "1".into() }], changes: vec![CellChange { column: "sku".into(), old: None, new: Some("x".into()) }], delete: false }],
    };
    assert_eq!(c.grid_statements(&partial).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(c.grid_statements(&edits(vec![edit("1", "id", Some("1"), Some("9"))])).await.unwrap_err().code, ErrorCode::Validation);
    assert_eq!(c.grid_statements(&edits(vec![edit("1", "nope", None, Some("x"))])).await.unwrap_err().code, ErrorCode::Validation);
    // The type name is never taken from the window: the SQL only names catalog types.
    let ok = c.grid_statements(&edits(vec![edit("1", "qty", Some("10"), Some("1); drop table d2.e3; --"))])).await.unwrap();
    assert!(!ok[0].sql.contains("drop"));
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_failed_transaction_is_rolled_back_not_committed_and_grid_handles_json_composite_keys_and_keyword_columns() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    c.txn_begin().await.unwrap();
    run_w(&c, "select * from d2.missing").await;
    let err = c.txn_end(true).await.unwrap_err();
    assert!(err.message.contains("rolled back"), "{}", err.message);
    assert!(!c.has_txn());
    assert_eq!(scalar(&c, "select 1").await, "1");
    // json has no equality operator; the key has two columns; one column is named like a keyword.
    let key = |b: &str| vec![KeyValue { column: "a".into(), value: "x".into() }, KeyValue { column: "b".into(), value: b.into() }];
    let g = |rows| GridEdits { schema: "d2".into(), table: "e6".into(), rows };
    let stmts = c
        .grid_statements(&g(vec![
            RowEdit { key: key("1"), changes: vec![CellChange { column: "j".into(), old: Some("{\"k\": 1}".into()), new: Some("{\"k\": 2}".into()) }, CellChange { column: "select".into(), old: Some("s".into()), new: Some("z".into()) }], delete: false },
            RowEdit { key: key("2"), changes: vec![CellChange { column: "j".into(), old: None, new: Some("[1]".into()) }], delete: false },
        ]))
        .await
        .unwrap();
    assert_eq!(c.grid_commit(&stmts).await.unwrap(), 2);
    assert_eq!(scalar(&c, "select j::text from d2.e6 where b = 1").await, "{\"k\": 2}");
    assert_eq!(scalar(&c, "select \"select\" from d2.e6 where b = 1").await, "z");
    // Not tried out: a statement that calls an effectful function is not previewed.
    let p = pieces_for("update d2.e4 set qty = pg_terminate_backend(0) where id = 1", RunMode::All, None).unwrap();
    assert!(c.preview(&p).await.unwrap()[0].as_deref().unwrap().starts_with("Not tried out"));
}

// ---- D5: structure, explain, diagram

use super::structure::StructureOp;

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_structure_lists_columns_keys_indexes_and_a_ddl() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let s = c.structure("d2", "orders").await.unwrap();
    assert_eq!(s.columns[0].name, "id");
    assert!(s.columns.iter().any(|c| c.name == "total" && c.type_name == "numeric(12,2)" && c.nullable));
    assert!(s.constraints.iter().any(|c| c.kind == "p" && c.columns == ["id"]));
    let fk = s.constraints.iter().find(|c| c.kind == "f").expect("a foreign key");
    assert!(fk.definition.contains("REFERENCES d2.customers(id)") && fk.columns == ["customer_id"]);
    assert!(s.indexes.iter().any(|i| i.primary && i.unique) && s.stats.table_bytes > 0);
    assert!(s.ddl.starts_with("create table \"d2\".\"orders\" (") && s.ddl.contains("constraint") && s.ddl.contains("not null"));
    assert_eq!(c.structure("d2", "nope").await.unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_a_structure_change_applies_in_one_transaction_and_a_failure_undoes_all_of_it() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let add = |n: &str| StructureOp::AddColumn { name: n.into(), type_name: "integer".into(), nullable: true, default: Some("0".into()) };
    let (stmts, _) = super::structure::statements("d2", "e5", &[add("extra"), StructureOp::CreateIndex { name: "e5_extra_idx".into(), columns: vec!["extra".into()], unique: false }], None).unwrap();
    c.structure_commit(&stmts).await.unwrap();
    let s = c.structure("d2", "e5").await.unwrap();
    assert!(s.columns.iter().any(|c| c.name == "extra" && c.default.as_deref() == Some("0")));
    assert!(s.indexes.iter().any(|i| i.name == "e5_extra_idx"));
    // The second statement fails (the column exists): the first one is undone too.
    let (stmts, _) = super::structure::statements("d2", "e5", &[add("second"), add("extra")], None).unwrap();
    let err = c.structure_commit(&stmts).await.unwrap_err();
    assert!(err.message.contains("already exists"), "{}", err.message);
    assert!(!c.structure("d2", "e5").await.unwrap().columns.iter().any(|c| c.name == "second"));
    // Rename, change type, drop.
    let ops = [
        StructureOp::RenameColumn { from: "extra".into(), to: "bonus".into() },
        StructureOp::ChangeType { column: "bonus".into(), type_name: "bigint".into() },
        StructureOp::DropIndex { name: "e5_extra_idx".into() },
        StructureOp::DropColumn { name: "bonus".into() },
    ];
    let (stmts, _) = super::structure::statements("d2", "e5", &ops, None).unwrap();
    c.structure_commit(&stmts).await.unwrap();
    assert!(!c.structure("d2", "e5").await.unwrap().columns.iter().any(|c| c.name == "bonus"));
    // A caught-up catalog: completion sees the change.
    assert!(!c.catalog(false).await.unwrap().tables.iter().any(|t| t.name == "e5" && t.columns.contains(&"bonus".to_string())));
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_explain_plans_and_analyze_times_a_read_without_changing_anything() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let plain = c.explain("select * from d2.orders where status = 'open'", false, 30_000).await.unwrap();
    assert!(!plain.analyzed && plain.root.est_rows > 0.0 && plain.root.actual_rows.is_none());
    let timed = c.explain("select o.id, c.name from d2.orders o join d2.customers c on c.id = o.customer_id where o.total > 100", true, 30_000).await.unwrap();
    assert!(timed.analyzed && timed.execution_ms.is_some() && timed.root.actual_rows.is_some() && timed.root.pct.is_some());
    let mut stack = vec![&timed.root];
    let mut names = Vec::new();
    while let Some(n) = stack.pop() {
        names.push(n.name.clone());
        stack.extend(n.children.iter());
    }
    assert!(names.iter().any(|n| n.contains("orders")) && names.iter().any(|n| n.contains("customers")), "{names:?}");
    // Even a function that writes can't, inside the read-only transaction.
    assert!(c.explain("select d2.bump()", true, 30_000).await.is_err());
    assert_eq!(scalar(&c, "select count(*) from d2.log").await, "0");
    assert_eq!(scalar(&c, "select 1").await, "1");
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_the_diagram_walks_foreign_keys_out_from_a_table() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    let d = c.diagram(&[("d2".into(), "orders".into())], 1).await.unwrap();
    let names: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"orders") && names.contains(&"customers") && names.contains(&"order_items"), "{names:?}");
    assert!(d.edges.iter().any(|e| e.from_table == "orders" && e.to_table == "customers" && e.from_columns == ["customer_id"] && e.to_columns == ["id"]));
    let orders = d.tables.iter().find(|t| t.name == "orders").unwrap();
    assert!(orders.columns.iter().any(|c| c.name == "id" && c.pk) && orders.columns.iter().any(|c| c.name == "customer_id" && c.fk));
    assert!(d.total_tables >= 3);
    assert_eq!(c.diagram(&[("d2".into(), "customers".into())], 0).await.unwrap().tables.len(), 1);
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_pg_explain_inside_a_manual_transaction_leaves_it_open_and_unchanged() {
    seed();
    let c = connect_direct(TlsMode::Off).await.unwrap();
    c.txn_begin().await.unwrap();
    run_w(&c, "update d2.e4 set qty = 11 where id = 1").await;
    let plan = c.explain("select * from d2.e4", true, 30_000).await.unwrap();
    assert!(plan.analyzed);
    // A function that writes can't, even here.
    assert!(c.explain("select d2.bump()", true, 30_000).await.is_err());
    // The transaction is still open with its change, and reads in it see that change.
    let st = c.txn_state().await.unwrap();
    assert!(st.active && !st.failed, "{st:?}");
    assert_eq!(scalar(&c, "select qty from d2.e4 where id = 1").await, "11");
    c.txn_end(false).await.unwrap();
    assert_eq!(scalar(&c, "select qty from d2.e4 where id = 1").await, "10");
}
