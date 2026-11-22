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
    PgConn::connect(stream, login, |_| {}).await.map(|(c, _)| c)
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
    let err = PgConn::connect(stream, login, |_| {}).await.err().unwrap();
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
    let (c, info) = PgConn::connect(stream, login, |_| {}).await.unwrap();
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
