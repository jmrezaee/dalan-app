//! D13 against the Docker test servers (`scripts/test-servers.sh up`): real copies between real servers, through
//! Dalan's own connections. The sources are made from the D12 fixtures (tests/fixtures/dumps).

use super::*;
use crate::dbs::transport::{self, Target, TlsMode};
use std::sync::Arc;
use zeroize::Zeroizing;

fn docker_in(container: &str, args: &[&str], input: &[u8]) -> String {
    use std::io::Write;
    let mut c = std::process::Command::new("docker")
        .arg("exec")
        .arg("-i")
        .arg(container)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("docker");
    c.stdin.take().unwrap().write_all(input).unwrap();
    String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout)
        .trim()
        .to_string()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/dumps")
            .join(name),
    )
    .unwrap()
}

fn psql(db: &str, sql: &str) -> String {
    docker_in(
        "dalan-test-pg",
        &["psql", "-U", "app", "-d", db, "-tAc", sql],
        b"",
    )
}

fn reopen(engine: Engine, port: u16) -> crate::sql::pg::Reopen {
    Arc::new(move || {
        Box::pin(async move {
            transport::direct_for(
                engine,
                &Target {
                    address: "127.0.0.1".into(),
                    port,
                    tls: false,
                    tls_server_name: None,
                },
            )
            .await
        })
    })
}

async fn pg(db: &str) -> RawConn {
    let login = crate::sql::pg::Login {
        user: "app".into(),
        password: Some(Zeroizing::new("test-pg".into())),
        database: Some(db.into()),
        tls: TlsMode::Off,
        server_name: "127.0.0.1".into(),
    };
    RawConn::Pg(Box::new(
        PgConn::connect(
            (reopen(Engine::Postgres, 15432))().await.unwrap(),
            login,
            reopen(Engine::Postgres, 15432),
            |_| {},
        )
        .await
        .unwrap()
        .0,
    ))
}

async fn my(port: u16, pw: &str, db: Option<&str>) -> RawConn {
    let login = crate::sql::mysql::MyLogin {
        user: "root".into(),
        password: Some(Zeroizing::new(pw.into())),
        database: db.map(str::to_string),
        tls: TlsMode::Require,
        server_name: "127.0.0.1".into(),
    };
    RawConn::My(
        crate::sql::mysql::MyConn::connect(
            (reopen(Engine::Mysql, port))().await.unwrap(),
            login,
            reopen(Engine::Mysql, port),
        )
        .await
        .unwrap()
        .0,
    )
}

async fn mongo() -> RawConn {
    let login = crate::mongo::conn::MongoLogin {
        user: "app".into(),
        password: Some(Zeroizing::new("test-mongo".into())),
        auth_db: "admin".into(),
        tls: TlsMode::Off,
        server_name: "127.0.0.1".into(),
    };
    RawConn::Mongo(
        MongoConn::connect(
            (reopen(Engine::Mongodb, 17017))().await.unwrap(),
            login,
            reopen(Engine::Mongodb, 17017),
        )
        .await
        .unwrap()
        .0,
    )
}

fn obj(schema: &str, name: &str, kind: &str) -> CopyObject {
    CopyObject {
        schema: schema.into(),
        name: name.into(),
        kind: kind.into(),
        filter: None,
        limit: None,
        rename: None,
        types: None,
        ddl: None,
    }
}

fn job(engine: Engine, target_db: &str, objects: Vec<CopyObject>, mode: CopyMode) -> CopyJob {
    CopyJob {
        req: CopyRequest {
            target_conn_id: String::new(),
            new_database: None,
            objects,
            mode,
            content: CopyContent::All,
            stop_on_error: true,
            routines: false,
        },
        source_engine: engine,
        target_db: target_db.into(),
        new_database: false,
        typed_name: String::new(),
        plans: vec![],
    }
}

async fn go(j: &CopyJob, src: RawConn, dst: RawConn, stop: &AtomicBool) -> CopyReport {
    run(Run {
        job: j,
        src,
        dst,
        stop,
        progress: &|_| {},
    })
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_copy_same_engine_postgres_mysql_to_mariadb_and_mongo_with_keys_views_and_row_checks() {
    // PostgreSQL: the D12 fixture as the source, copied into another database with keys, sequences and the view.
    for db in ["d13_src", "d13_dst"] {
        psql("shop", &format!("drop database if exists {db}"));
        psql("shop", &format!("create database {db}"));
    }
    docker_in(
        "dalan-test-pg",
        &["psql", "-q", "-U", "app", "-d", "d13_src"],
        &fixture("pg16.sql"),
    );
    let objs = vec![
        obj("public", "customers", "table"),
        obj("public", "orders", "table"),
        obj("public", "big_orders", "view"),
    ];
    let r = go(
        &job(Engine::Postgres, "d13_dst", objs.clone(), CopyMode::Create),
        pg("d13_src").await,
        pg("d13_dst").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let orders = r
        .tables
        .iter()
        .find(|t| t.source == "public.orders")
        .unwrap();
    assert_eq!(
        (orders.rows, orders.source_rows, orders.target_rows),
        (4.0, Some(4.0), Some(4.0))
    );
    assert_eq!(
        psql("d13_dst", "select note from public.orders where id = 2"),
        "line\nbreak"
    );
    assert_eq!(
        psql("d13_dst", "select count(*) from public.big_orders"),
        "2"
    );
    assert_eq!(psql("d13_dst", "select count(*) from pg_constraint where conrelid = 'public.orders'::regclass and contype in ('p','f')"), "2");
    // Identity moved on: the next insert doesn't collide.
    assert!(psql(
        "d13_dst",
        "insert into public.orders (customer_id, total) values (1, 1) returning id"
    )
    .starts_with("5"));
    // Again with "create": refused per table (it exists), nothing changed; "drop and recreate" works; upsert by key.
    let r = go(
        &job(
            Engine::Postgres,
            "d13_dst",
            vec![obj("public", "customers", "table")],
            CopyMode::Create,
        ),
        pg("d13_src").await,
        pg("d13_dst").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "failed");
    psql(
        "d13_src",
        "update public.customers set name = 'Ada L' where id = 1",
    );
    let r = go(
        &job(
            Engine::Postgres,
            "d13_dst",
            vec![obj("public", "customers", "table")],
            CopyMode::Upsert,
        ),
        pg("d13_src").await,
        pg("d13_dst").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert_eq!(
        psql("d13_dst", "select name from public.customers where id = 1"),
        "Ada L"
    );
    assert_eq!(
        psql("d13_dst", "select count(*) from public.customers"),
        "3"
    );
    // A filter and a rename: a sample into another table.
    let mut sample = obj("public", "orders", "table");
    sample.filter = Some("total > 100".into());
    sample.rename = Some("big".into());
    let r = go(
        &job(Engine::Postgres, "d13_dst", vec![sample], CopyMode::Create),
        pg("d13_src").await,
        pg("d13_dst").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert_eq!(psql("d13_dst", "select count(*) from public.big"), "2");
    // Stop before anything: every table stopped, nothing written.
    let r = go(
        &job(
            Engine::Postgres,
            "d13_dst",
            vec![obj("public", "orders", "table")],
            CopyMode::Recreate,
        ),
        pg("d13_src").await,
        pg("d13_dst").await,
        &AtomicBool::new(true),
    )
    .await;
    assert_eq!(r.outcome, "stopped");
    assert_eq!(psql("d13_dst", "select count(*) from public.orders"), "5");

    // MySQL 8.4 → MariaDB 11, through Dalan (the servers never meet).
    docker_in(
        "dalan-test-mysql",
        &[
            "mysql",
            "-uroot",
            "-ptest-mysql",
            "-e",
            "drop database if exists d13; create database d13",
        ],
        b"",
    );
    docker_in(
        "dalan-test-mysql",
        &["mysql", "-uroot", "-ptest-mysql", "d13"],
        &fixture("mysql84.sql"),
    );
    docker_in(
        "dalan-test-mariadb",
        &[
            "mariadb",
            "-uroot",
            "-ptest-mariadb",
            "-e",
            "drop database if exists d13c; create database d13c",
        ],
        b"",
    );
    let objs = vec![
        obj("d13", "customers", "table"),
        obj("d13", "orders", "table"),
        obj("d13", "big_orders", "view"),
    ];
    let r = go(
        &job(Engine::Mysql, "d13c", objs, CopyMode::Create),
        my(13306, "test-mysql", Some("d13")).await,
        my(13307, "test-mariadb", Some("d13c")).await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    let n = docker_in("dalan-test-mariadb", &["mariadb", "-uroot", "-ptest-mariadb", "-N", "-e", "select count(*) from d13c.orders; select note from d13c.orders where id = 2; select count(*) from d13c.big_orders"], b"");
    assert_eq!(
        n.lines().collect::<Vec<_>>(),
        vec!["4", "quote's", "2"],
        "{n}"
    );

    // MongoDB: one database into another, exact types and indexes.
    let RawConn::Mongo(m) = mongo().await else {
        unreachable!()
    };
    for db in ["d13m", "d13m_copy"] {
        let _ = m.command(db, doc! { "dropDatabase": 1 }).await;
    }
    m.command("d13m", doc! { "insert": "items", "documents": [{ "_id": 1, "n": 5_i64, "d": bson::Decimal128::from_bytes([0; 16]) }, { "_id": 2, "n": 7_i64 }] }).await.unwrap();
    m.command(
        "d13m",
        doc! { "createIndexes": "items", "indexes": [{ "key": { "n": 1 }, "name": "n_1" }] },
    )
    .await
    .unwrap();
    let r = go(
        &job(
            Engine::Mongodb,
            "d13m_copy",
            vec![obj("d13m", "items", "collection")],
            CopyMode::Create,
        ),
        mongo().await,
        mongo().await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert_eq!(
        (r.tables[0].source_rows, r.tables[0].target_rows),
        (Some(2.0), Some(2.0))
    );
    let got = m
        .command(
            "d13m_copy",
            doc! { "find": "items", "filter": { "_id": 1 } },
        )
        .await
        .unwrap();
    let d = &crate::mongo::docs::names(got.get_document("cursor").unwrap(), "firstBatch")[0];
    assert!(matches!(d.get("n"), Some(Bson::Int64(5))), "{d:?}");
    let idx = m
        .command("d13m_copy", doc! { "listIndexes": "items" })
        .await
        .unwrap();
    assert_eq!(
        crate::mongo::docs::names(idx.get_document("cursor").unwrap(), "firstBatch").len(),
        2
    );
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_copy_across_engines_types_keys_related_and_routines() {
    // The sources: the D12 fixtures.
    for db in ["d13x_src", "d13x_from_my", "d13x_routines"] {
        psql("shop", &format!("drop database if exists {db}"));
        psql("shop", &format!("create database {db}"));
    }
    docker_in(
        "dalan-test-pg",
        &["psql", "-q", "-U", "app", "-d", "d13x_src"],
        &fixture("pg16.sql"),
    );
    psql("d13x_src", "create table public.kinds (id serial primary key, flag boolean, at timestamptz, tags text[], blob bytea, span interval, j jsonb, price numeric)");
    psql("d13x_src", "insert into public.kinds (flag, at, tags, blob, span, j, price) values (true, '2026-01-02 03:04:05+02', '{a,b}', '\\xdeadbeef', '1 day 2 hours', '{\"k\": [1, 2]}', 12.345), (null, null, null, null, null, null, null)");
    docker_in("dalan-test-mysql", &["mysql", "-uroot", "-ptest-mysql", "-e", "drop database if exists d13x; create database d13x; drop database if exists d13x_back; create database d13x_back"], b"");
    docker_in(
        "dalan-test-mysql",
        &["mysql", "-uroot", "-ptest-mysql", "d13x_back"],
        &fixture("mysql84.sql"),
    );

    // "Select related": orders references customers.
    let rel = related(&pg("d13x_src").await, &[obj("public", "orders", "table")])
        .await
        .unwrap();
    assert_eq!(
        rel.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
        vec!["customers"]
    );

    // PostgreSQL → MySQL: types mapped, keys and the foreign key after the data, the view listed with its DDL.
    let objs = vec![
        obj("public", "customers", "table"),
        obj("public", "orders", "table"),
        obj("public", "kinds", "table"),
        obj("public", "big_orders", "view"),
    ];
    let r = go(
        &job(Engine::Postgres, "d13x", objs, CopyMode::Create),
        pg("d13x_src").await,
        my(13306, "test-mysql", Some("d13x")).await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert!(
        r.tables
            .iter()
            .filter(|t| t.status == "done")
            .all(|t| t.source_rows == t.target_rows),
        "{r:?}"
    );
    assert_eq!(r.not_copied.len(), 1);
    assert!(r.not_copied[0].ddl.contains("orders"), "{:?}", r.not_copied);
    assert!(
        r.warnings.iter().any(|w| w.contains("time zone")),
        "{:?}",
        r.warnings
    );
    let got = docker_in("dalan-test-mysql", &["mysql", "-uroot", "-ptest-mysql", "-N", "-e", "select flag, at, tags, hex(`blob`), span, j, note from d13x.kinds, d13x.orders where kinds.id = 1 and orders.id = 2; select count(*) from information_schema.referential_constraints where constraint_schema = 'd13x'; select mood from d13x.customers where id = 1"], b"");
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(
        lines[0].split('\t').collect::<Vec<_>>(),
        vec![
            "1",
            "2026-01-02 01:04:05.000000",
            "[\"a\", \"b\"]",
            "DEADBEEF",
            "P1DT2H",
            "{\"k\": [1, 2]}",
            "line\\nbreak"
        ],
        "{got}"
    );
    assert_eq!(lines[1], "1", "the foreign key arrived: {got}");
    assert_eq!(lines[2], "happy", "{got}");
    // A column type chosen in the review, and edited DDL that isn't one CREATE TABLE refused.
    let mut o = obj("public", "customers", "table");
    o.rename = Some("c2".into());
    o.types = Some(vec![CopyColumnType {
        name: "name".into(),
        target_type: "varchar(20)".into(),
    }]);
    let r = go(
        &job(Engine::Postgres, "d13x", vec![o.clone()], CopyMode::Create),
        pg("d13x_src").await,
        my(13306, "test-mysql", Some("d13x")).await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert_eq!(docker_in("dalan-test-mysql", &["mysql", "-uroot", "-ptest-mysql", "-N", "-e", "select column_type from information_schema.columns where table_schema = 'd13x' and table_name = 'c2' and column_name = 'name'"], b""), "varchar(20)");
    o.rename = Some("c3".into());
    o.ddl = Some("create table c3 (id int); drop table customers".into());
    let r = go(
        &job(Engine::Postgres, "d13x", vec![o], CopyMode::Create),
        pg("d13x_src").await,
        my(13306, "test-mysql", Some("d13x")).await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "failed");
    assert_eq!(
        docker_in(
            "dalan-test-mysql",
            &[
                "mysql",
                "-uroot",
                "-ptest-mysql",
                "-N",
                "-e",
                "select count(*) from d13x.customers"
            ],
            b""
        ),
        "3"
    );

    // MySQL → PostgreSQL: auto_increment becomes identity (moved on), tinyint(1)… the rows and the key arrive.
    let objs = vec![
        obj("d13x_back", "customers", "table"),
        obj("d13x_back", "orders", "table"),
    ];
    let r = go(
        &job(Engine::Mysql, "d13x_from_my", objs, CopyMode::Create),
        my(13306, "test-mysql", Some("d13x_back")).await,
        pg("d13x_from_my").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert_eq!(
        psql(
            "d13x_from_my",
            "select note from public.orders where id = 2"
        ),
        "quote's"
    );
    assert_eq!(psql("d13x_from_my", "select count(*) from pg_constraint where conrelid = 'public.orders'::regclass and contype = 'f'"), "1");
    assert!(psql(
        "d13x_from_my",
        "insert into public.customers (name) values ('Di') returning id"
    )
    .starts_with('4'));

    // Same engine with functions and triggers asked for.
    let mut j = job(
        Engine::Postgres,
        "d13x_routines",
        vec![
            obj("public", "customers", "table"),
            obj("public", "orders", "table"),
        ],
        CopyMode::Create,
    );
    j.req.routines = true;
    let r = go(
        &j,
        pg("d13x_src").await,
        pg("d13x_routines").await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(psql("d13x_routines", "select public.order_count(1)"), "2");
    assert!(
        psql(
            "d13x_routines",
            "insert into public.orders (customer_id, total) values (1, 1) returning note"
        )
        .starts_with("n/a"),
        "the trigger runs"
    );
    // MySQL → MariaDB with the procedure and the trigger.
    docker_in(
        "dalan-test-mariadb",
        &[
            "mariadb",
            "-uroot",
            "-ptest-mariadb",
            "-e",
            "drop database if exists d13r; create database d13r",
        ],
        b"",
    );
    let mut j = job(
        Engine::Mysql,
        "d13r",
        vec![
            obj("d13x_back", "customers", "table"),
            obj("d13x_back", "orders", "table"),
        ],
        CopyMode::Create,
    );
    j.req.routines = true;
    let r = go(
        &j,
        my(13306, "test-mysql", Some("d13x_back")).await,
        my(13307, "test-mariadb", Some("d13r")).await,
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(r.outcome, "done", "{r:?}");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let got = docker_in("dalan-test-mariadb", &["mariadb", "-uroot", "-ptest-mariadb", "-N", "-e", "select count(*) from information_schema.routines where routine_schema = 'd13r'; select count(*) from information_schema.triggers where trigger_schema = 'd13r'"], b"");
    assert_eq!(
        got.split_whitespace().collect::<Vec<_>>(),
        vec!["1", "1"],
        "{got}"
    );
}
