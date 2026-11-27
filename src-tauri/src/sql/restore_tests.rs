//! D12 against the Docker test servers (`scripts/test-servers.sh up`): the real dumps in tests/fixtures/dumps are
//! restored by the real tools. This computer has no PostgreSQL or MySQL client, so the tools are reached through
//! stand-ins that run the server container's own (`docker exec`); MongoDB's mongorestore runs here when installed.
//! Through a host (sshd `a`), stand-in tools on the host check the upload folder and Stop.

use super::*;
use crate::dbs::transport::{self, Target, TlsMode};
use crate::redis::store::{RedisRoute, RedisWriteMode};
use zeroize::Zeroizing;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/dumps")
        .join(name)
}

fn docker(args: &[&str]) -> String {
    let o = std::process::Command::new("docker")
        .args(args)
        .output()
        .expect("docker");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// A pretend tool on the host `a`, on its PATH.
fn put_tool(name: &str, body: &str) {
    use std::io::Write;
    let mut c = std::process::Command::new("docker")
        .args([
            "exec",
            "-i",
            "dalan-test-ssh",
            "sh",
            "-c",
            &format!("cat > /usr/local/bin/{name} && chmod 755 /usr/local/bin/{name}"),
        ])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("docker");
    c.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(c.wait().unwrap().success());
}

fn psql(db: &str, sql: &str) -> String {
    docker(&[
        "exec",
        "dalan-test-pg",
        "psql",
        "-U",
        "app",
        "-d",
        db,
        "-tAc",
        sql,
    ])
}

/// Stand-in tools in a folder of their own, found through DALAN_TEST_TOOL_DIR.
fn stand_ins() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dalan-restore-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = |name: &str, body: &str| {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/bash\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    script(
        "psql",
        "exec docker exec -i -e PGPASSWORD dalan-test-pg psql \"$@\"",
    );
    // A directory dump is copied into the container first.
    script(
        "pg_restore",
        "out=(); prev=\nfor a in \"$@\"; do\n  case \"$prev\" in -Fd|-Fc|-L) n=/tmp/dalan-$RANDOM; docker exec dalan-test-pg rm -rf $n; docker cp \"$a\" dalan-test-pg:$n >/dev/null; a=$n;; esac\n  out+=(\"$a\"); prev=\"$a\"\ndone\nexec docker exec -i -e PGPASSWORD dalan-test-pg pg_restore \"${out[@]}\"",
    );
    for (name, c) in [
        ("mysql", "dalan-test-mysql"),
        ("mariadb", "dalan-test-mariadb"),
    ] {
        script(
            name,
            &format!("case \"$1\" in --defaults-file=*) cfg=\"${{1#--defaults-file=}}\"; shift; docker cp \"$cfg\" {c}:/tmp/dalan.cnf >/dev/null; exec docker exec -i {c} {name} --defaults-file=/tmp/dalan.cnf \"$@\";; esac\nexec docker exec -i {c} {name} \"$@\""),
        );
    }
    if let Ok(p) = which("mongorestore") {
        let _ = std::os::unix::fs::symlink(p, dir.join("mongorestore"));
    }
    std::env::set_var("DALAN_TEST_TOOL_DIR", &dir);
    dir
}

fn which(name: &str) -> Result<std::path::PathBuf, ()> {
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
        .iter()
        .map(|d| std::path::Path::new(d).join(name))
        .find(|p| p.is_file())
        .ok_or(())
}

fn cfg(engine: Engine, port: u16, db: &str, user: &str, pw: &str) -> SqlConfig {
    SqlConfig {
        name: "t".into(),
        engine,
        route: RedisRoute::Direct,
        host_id: None,
        address: "127.0.0.1".into(),
        port,
        database: Some(db.into()),
        user: user.into(),
        password: Some(Zeroizing::new(pw.into())),
        tls: TlsMode::Off,
        server_name: None,
        auth_source: Some("admin".into()),
    }
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

async fn pg(db: &str) -> crate::sql::pg::PgConn {
    let login = crate::sql::pg::Login {
        user: "app".into(),
        password: Some(Zeroizing::new("test-pg".into())),
        database: Some(db.into()),
        tls: TlsMode::Off,
        server_name: "127.0.0.1".into(),
    };
    crate::sql::pg::PgConn::connect(
        (reopen(Engine::Postgres, 15432))().await.unwrap(),
        login,
        reopen(Engine::Postgres, 15432),
        |_| {},
    )
    .await
    .unwrap()
    .0
}

async fn my(port: u16, root_pw: &str) -> crate::sql::mydb::MyDb {
    let login = crate::sql::mysql::MyLogin {
        user: "root".into(),
        password: Some(Zeroizing::new(root_pw.into())),
        database: None,
        tls: TlsMode::Off,
        server_name: "127.0.0.1".into(),
    };
    let warm = crate::sql::mysql::MyLogin {
        tls: TlsMode::Require,
        ..login.clone()
    };
    if let Ok((c, _)) = crate::sql::mysql::MyConn::connect(
        (reopen(Engine::Mysql, port))().await.unwrap(),
        warm,
        reopen(Engine::Mysql, port),
    )
    .await
    {
        c.close().await;
    }
    crate::sql::mydb::MyDb::new(
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

fn opts(new_db: Option<&str>) -> RestoreOptions {
    RestoreOptions {
        new_database: new_db.map(str::to_string),
        source_database: None,
        existing: Existing::Stop,
        content: Content::All,
        skip_owner_acl: true,
        include_roles: false,
        single_transaction: true,
        objects: None,
        rename_schema: None,
        jobs: 0,
        disable_triggers: false,
    }
}

/// Plans and runs a restore the way `restore_commit` does (without the window and the gate's typed name).
async fn restore(
    conn: Conn<'_>,
    tool_cfg: &SqlConfig,
    ssh: Option<Arc<Connected>>,
    file: &std::path::Path,
    o: RestoreOptions,
    stop: Option<Arc<AtomicBool>>,
) -> AppResult<(RestorePlan, Option<RestoreResult>)> {
    let dump = Arc::new(dumpfile::pick(file, "t".into()).await?);
    let (plan, job) = plan(PlanInput {
        conn: match &conn {
            Conn::Pg(c) => Conn::Pg(c),
            Conn::My(c) => Conn::My(c),
            Conn::Mongo(c) => Conn::Mongo(c),
        },
        cfg: tool_cfg,
        ssh: ssh.as_deref(),
        dump: &dump,
        dump_id: "t".into(),
        opts: o,
        saved_name: "test".into(),
        environment: "none".into(),
        mode: RedisWriteMode::Confirm,
    })
    .await?;
    if plan.blocked.is_some() {
        return Ok((plan, None));
    }
    if job.new_database {
        create_database(&conn, &job.target_db, encoding(&job)).await?;
    }
    let mut c = clone_cfg(tool_cfg);
    c.database = Some(job.target_db.clone());
    let (outcome, message, rep) = run(RunInput {
        job: &job,
        cfg: &c,
        ssh,
        dump,
        stop: stop.unwrap_or_default(),
        progress: &|_| {},
    })
    .await?;
    let removed = after(&conn, &job, &outcome).await;
    Ok((plan, Some(result(&job, outcome, message, rep, removed))))
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_restore_real_dumps_into_new_and_existing_databases_on_every_engine() {
    let _tools = stand_ins();
    for db in ["d12_custom", "d12_tar", "d12_dir", "d12_plain", "d12_all"] {
        psql("shop", &format!("drop database if exists {db}"));
    }
    let shop = pg("shop").await;
    let tool = cfg(Engine::Postgres, 5432, "shop", "app", "test-pg");
    // Every PostgreSQL format into a new database; owners are skipped, so the missing role app_owner is no error.
    for (file, db) in [
        ("pg16.custom", "d12_custom"),
        ("pg13.tar", "d12_tar"),
        ("pg16.dir", "d12_dir"),
        ("pg16.sql.gz", "d12_plain"),
        ("pg16-all.sql", "d12_all"),
    ] {
        let o = RestoreOptions {
            source_database: (db == "d12_all").then(|| "shop".to_string()),
            ..opts(Some(db))
        };
        let (plan, r) = restore(Conn::Pg(&shop), &tool, None, &fixture(file), o, None)
            .await
            .unwrap_or_else(|e| panic!("{file}: {}", e.message));
        let r = r.unwrap();
        assert_eq!(
            (r.outcome.as_str(), r.message.as_deref()),
            ("done", None),
            "{file}: {:?}",
            r.warnings
        );
        assert_eq!(plan.level, "confirm", "{file}");
        assert_eq!(
            psql(db, "select count(*) from public.orders"),
            "4",
            "{file}"
        );
        assert_eq!(
            psql(db, "select note from public.orders where id = 2"),
            "line\nbreak",
            "{file}"
        );
        assert_eq!(psql(db, "select public.order_count(1)"), "2", "{file}");
        assert!(
            !r.warnings.iter().any(|w| w.text.contains("app_owner")),
            "{file}: {:?}",
            r.warnings
        );
    }
    // The same plain dump again into the existing database: clashes block "stop if anything exists".
    let plain = pg("d12_plain").await;
    let into_plain = cfg(Engine::Postgres, 5432, "d12_plain", "app", "test-pg");
    let (plan, r) = restore(
        Conn::Pg(&plain),
        &into_plain,
        None,
        &fixture("pg16.sql"),
        opts(None),
        None,
    )
    .await
    .unwrap();
    assert!(
        r.is_none() && plan.clashes.contains(&"public.orders".to_string()),
        "{plan:?}"
    );
    assert_eq!(plan.level, "typed");
    // Drop and replace: the rows are the dump's, not twice as many.
    let (plan, r) = restore(
        Conn::Pg(&plain),
        &into_plain,
        None,
        &fixture("pg16.sql"),
        RestoreOptions {
            existing: Existing::Replace,
            ..opts(None)
        },
        None,
    )
    .await
    .unwrap();
    assert!(plan.replaces.contains(&"public.orders".to_string()));
    assert_eq!(r.unwrap().outcome, "done");
    assert_eq!(psql("d12_plain", "select count(*) from public.orders"), "4");
    // Add rows: the keys clash, the one transaction fails, nothing changed.
    let (_, r) = restore(
        Conn::Pg(&plain),
        &into_plain,
        None,
        &fixture("pg16.custom"),
        RestoreOptions {
            existing: Existing::Append,
            ..opts(None)
        },
        None,
    )
    .await
    .unwrap();
    let r = r.unwrap();
    assert_eq!(r.outcome, "failed");
    assert!(
        r.nothing_changed && r.message.as_deref().unwrap_or("").contains("duplicate key"),
        "{r:?}"
    );
    assert_eq!(psql("d12_plain", "select count(*) from public.orders"), "4");
    // A dump from pg_dump 17 and the server's pg_restore 16: refused before anything runs.
    let e = restore(
        Conn::Pg(&shop),
        &tool,
        None,
        &fixture("pg17.custom"),
        opts(Some("d12_new")),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::DbDumpTooNew, "{}", e.message);
    // A failure into a new database removes it again.
    let bad = std::env::temp_dir().join(format!("dalan-bad-{}.sql", std::process::id()));
    std::fs::write(&bad, "-- PostgreSQL database dump\nCREATE TABLE t (a int);\nINSERT INTO t VALUES ('not a number');\n").unwrap();
    let (_, r) = restore(
        Conn::Pg(&shop),
        &tool,
        None,
        &bad,
        opts(Some("d12_bad")),
        None,
    )
    .await
    .unwrap();
    let r = r.unwrap();
    assert_eq!(r.outcome, "failed");
    assert!(r.removed_new_database && r.nothing_changed, "{r:?}");
    assert_eq!(
        psql(
            "shop",
            "select count(*) from pg_database where datname = 'd12_bad'"
        ),
        "0"
    );
    // A hostile file never reaches planning.
    std::fs::write(&bad, "-- PostgreSQL database dump\n\\! touch /tmp/pwned\n").unwrap();
    assert_eq!(
        restore(
            Conn::Pg(&shop),
            &tool,
            None,
            &bad,
            opts(Some("d12_x")),
            None
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::DbDumpUnsafe
    );
    let _ = std::fs::remove_file(&bad);
    // Part 2: only some tables (an archive through -L, a plain dump through the stream filter), a schema renamed in
    // the stream, and parallel jobs from a gzipped custom archive (unpacked into a file first).
    for db in ["d12_some", "d12_some_plain", "d12_rename", "d12_jobs"] {
        psql("shop", &format!("drop database if exists {db}"));
    }
    let only = |db: &str| RestoreOptions {
        objects: Some(vec!["public.customers".into()]),
        ..opts(Some(db))
    };
    for (file, db) in [("pg16.custom", "d12_some"), ("pg16.sql", "d12_some_plain")] {
        let (plan, r) = restore(Conn::Pg(&shop), &tool, None, &fixture(file), only(db), None)
            .await
            .unwrap_or_else(|e| panic!("{file}: {}", e.message));
        let r = r.unwrap();
        assert_eq!(r.outcome, "done", "{file}: {r:?}");
        assert_eq!(plan.rows, 3.0, "{file}");
        assert_eq!(
            psql(db, "select count(*) from public.customers"),
            "3",
            "{file}"
        );
        assert_eq!(
            psql(db, "select count(*) from pg_class where relname = 'orders'"),
            "0",
            "{file}"
        );
    }
    let (_, r) = restore(
        Conn::Pg(&shop),
        &tool,
        None,
        &fixture("pg16.sql"),
        RestoreOptions {
            rename_schema: Some(RenameSchema {
                from: "public".into(),
                to: "shop2".into(),
            }),
            ..opts(Some("d12_rename"))
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap().outcome, "done");
    assert_eq!(psql("d12_rename", "select count(*) from shop2.orders"), "4");
    let (_, r) = restore(
        Conn::Pg(&shop),
        &tool,
        None,
        &fixture("pg16.custom.gz"),
        RestoreOptions {
            jobs: 2,
            single_transaction: false,
            ..opts(Some("d12_jobs"))
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap().outcome, "done");
    assert_eq!(psql("d12_jobs", "select count(*) from public.orders"), "4");
    assert_eq!(
        restore(
            Conn::Pg(&shop),
            &tool,
            None,
            &fixture("pg16.custom"),
            RestoreOptions {
                jobs: 2,
                ..opts(Some("d12_x"))
            },
            None
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Validation
    );
    // A MySQL dump can't go into PostgreSQL.
    assert_eq!(
        restore(
            Conn::Pg(&shop),
            &tool,
            None,
            &fixture("mysql84.sql"),
            opts(Some("d12_x")),
            None
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Validation
    );

    // MySQL and MariaDB, each with its own client; DEFINERs go, so the trigger belongs to the restoring user.
    for (port, root, file, engine, container, client) in [
        (
            13306,
            "test-mysql",
            "mysql84.sql.gz",
            Engine::Mysql,
            "dalan-test-mysql",
            "mysql",
        ),
        (
            13307,
            "test-mariadb",
            "mariadb11.sql.xz",
            Engine::Mariadb,
            "dalan-test-mariadb",
            "mariadb",
        ),
    ] {
        let m = my(port, root).await;
        let _ = m.conn.query("drop database if exists d12_my", 0).await;
        let tool = SqlConfig {
            database: None,
            ..cfg(engine, 3306, "", "root", root)
        };
        let (plan, r) = restore(
            Conn::My(&m),
            &tool,
            None,
            &fixture(file),
            opts(Some("d12_my")),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{file}: {}", e.message));
        assert_eq!(plan.tool, client, "{file}");
        let r = r.unwrap();
        assert_eq!(r.outcome, "done", "{file}: {r:?}");
        let n = docker(&["exec", container, client, "-uroot", &format!("-p{root}"), "-N", "-e", "select count(*) from d12_my.orders; select count(*) from information_schema.triggers where trigger_schema = 'd12_my'"]);
        assert_eq!(
            n.split_whitespace().collect::<Vec<_>>(),
            vec!["4", "1"],
            "{file}"
        );
        let _ = m.conn.query("drop database if exists d12_my", 0).await;
    }

    // MongoDB: the archive (gzipped) and the folder, from the dump's database `shop` into a new one.
    if which("mongorestore").is_ok() {
        let login = crate::mongo::conn::MongoLogin {
            user: "app".into(),
            password: Some(Zeroizing::new("test-mongo".into())),
            auth_db: "admin".into(),
            tls: TlsMode::Off,
            server_name: "127.0.0.1".into(),
        };
        let mg = crate::mongo::conn::MongoConn::connect(
            (reopen(Engine::Mongodb, 17017))().await.unwrap(),
            login,
            reopen(Engine::Mongodb, 17017),
        )
        .await
        .unwrap()
        .0;
        for (file, db) in [
            ("mongo7.archive.gz", "d12_mg_archive"),
            ("mongo7.dir", "d12_mg_dir"),
        ] {
            let _ = mg.command(db, bson::doc! { "dropDatabase": 1 }).await;
            let tool = cfg(Engine::Mongodb, 17017, "shop", "app", "test-mongo");
            let (_, r) = restore(
                Conn::Mongo(&mg),
                &tool,
                None,
                &fixture(file),
                opts(Some(db)),
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("{file}: {}", e.message));
            let r = r.unwrap();
            assert_eq!(r.outcome, "done", "{file}: {r:?}");
            let n = mg
                .command(db, bson::doc! { "count": "customers" })
                .await
                .unwrap();
            assert_eq!(n.get_i32("n").unwrap_or(-1), 3, "{file}");
            let _ = mg.command(db, bson::doc! { "dropDatabase": 1 }).await;
        }
    }
    std::env::remove_var("DALAN_TEST_TOOL_DIR");
}

/// Stop ends the tool; with one transaction nothing changed, and the new database goes again.
#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_restore_stop_kills_the_tool_here_and_on_a_host_and_the_upload_folder_is_private_and_removed(
) {
    // Here: a pretend psql that never ends.
    let dir = std::env::temp_dir().join(format!("dalan-restore-slow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("psql");
    std::fs::write(&p, "#!/bin/sh\ncase \"$*\" in *--version*) echo 'psql (PostgreSQL) 16.4'; exit 0;; esac\necho $$ > \"$0.pid\"\nexec sleep 30\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("DALAN_TEST_TOOL_DIR", &dir);
    psql("shop", "drop database if exists d12_stop");
    let shop = pg("shop").await;
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        s2.store(true, Ordering::SeqCst);
    });
    let tool = cfg(Engine::Postgres, 5432, "shop", "app", "test-pg");
    let (_, r) = restore(
        Conn::Pg(&shop),
        &tool,
        None,
        &fixture("pg16.sql"),
        opts(Some("d12_stop")),
        Some(stop),
    )
    .await
    .unwrap();
    let r = r.unwrap();
    assert_eq!(r.outcome, "stopped");
    assert!(r.nothing_changed && r.removed_new_database, "{r:?}");
    let pid = std::fs::read_to_string(dir.join("psql.pid")).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "the tool still runs after Stop");
    std::env::remove_var("DALAN_TEST_TOOL_DIR");
    let _ = std::fs::remove_dir_all(&dir);

    // On a host: pretend tools that report what they see, and one that never ends.
    let h = crate::ssh::tests::harness(None).await;
    let ssh = Arc::new(crate::dbs::spike::sign_in_to_a(&h).await);
    put_tool("pg_restore", "#!/bin/sh\ncase \"$1\" in --version) echo 'pg_restore (PostgreSQL) 16.4'; exit 0;; esac\nfor a; do last=$a; done\necho \"pg_restore: warning: dir=$last mode=$(stat -c %a $last) files=$(ls $last | tr '\\n' ',')\" >&2\n");
    put_tool("psql", "#!/bin/sh\ncase \"$1\" in --version) echo 'psql (PostgreSQL) 16.4'; exit 0;; esac\nexec sleep 31\n");
    let (plan, r) = restore(
        Conn::Pg(&shop),
        &tool,
        Some(Arc::clone(&ssh)),
        &fixture("pg16.dir"),
        opts(Some("d12_up")),
        None,
    )
    .await
    .unwrap();
    assert!(plan.upload, "{plan:?}");
    let r = r.unwrap();
    assert_eq!(r.outcome, "done", "{r:?}");
    let seen = r
        .warnings
        .iter()
        .map(|w| w.text.clone())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        seen.contains("mode=700") && seen.contains("toc.dat") && seen.contains("3436.dat.gz"),
        "{seen}"
    );
    let up = seen
        .split("dir=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .unwrap()
        .to_string();
    let (_, gone) = backup::exec_collect(&ssh, &format!("test -e {up} && echo there || echo gone"))
        .await
        .unwrap();
    assert_eq!(
        gone.trim(),
        "gone",
        "the upload folder was left on the host"
    );
    psql("shop", "drop database if exists d12_up");
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        s2.store(true, Ordering::SeqCst);
    });
    let (_, r) = restore(
        Conn::Pg(&shop),
        &tool,
        Some(Arc::clone(&ssh)),
        &fixture("pg16.sql"),
        opts(Some("d12_up")),
        Some(stop),
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap().outcome, "stopped");
    let (_, left) = backup::exec_collect(&ssh, "ps -eo args | grep -c '^sleep 31' || true")
        .await
        .unwrap();
    assert_eq!(
        left.trim(),
        "0",
        "the tool still runs on the host after Stop"
    );
    docker(&[
        "exec",
        "dalan-test-ssh",
        "rm",
        "-f",
        "/usr/local/bin/pg_restore",
        "/usr/local/bin/psql",
    ]);
}

/// D16: a dump's catalog through a scratch database, which is gone again afterwards (PostgreSQL and MySQL).
#[tokio::test]
#[ignore = "needs the Docker test servers"]
async fn l3_a_dump_is_read_through_a_scratch_database_that_goes_again() {
    let _tools = stand_ins();
    let shop = pg("shop").await;
    let server = |env: &'static str| crate::sql::restore::Scratch {
        name: "test",
        environment: env,
        mode: RedisWriteMode::Confirm,
    };
    let dump = Arc::new(
        dumpfile::pick(&fixture("pg16.sql"), "t".into())
            .await
            .unwrap(),
    );
    let err = crate::sql::schemacat::from_dump(
        Conn::Pg(&shop),
        cfg(Engine::Postgres, 5432, "shop", "app", "test-pg"),
        None,
        Arc::clone(&dump),
        "t",
        server("production"),
        None,
        async |_| unreachable!(),
    )
    .await
    .unwrap_err();
    assert!(err.message.contains("is production"), "{}", err.message);
    let cat = crate::sql::schemacat::from_dump(
        Conn::Pg(&shop),
        cfg(Engine::Postgres, 5432, "shop", "app", "test-pg"),
        None,
        dump,
        "t",
        server("staging"),
        Some("public"),
        async |db| Ok(crate::sql::RawConn::Pg(Box::new(pg(&db).await))),
    )
    .await
    .unwrap();
    assert_eq!(cat.schema, "public");
    assert!(
        cat.tables
            .iter()
            .any(|t| t.name == "orders" && t.constraints.iter().any(|c| c.kind == "f")),
        "{:?}",
        cat.tables.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    assert!(cat
        .objects
        .iter()
        .any(|o| o.name.starts_with("order_count(")));
    assert_eq!(
        psql(
            "shop",
            "select count(*) from pg_database where datname like 'dalan_compare_%'"
        ),
        "0",
        "the scratch database stayed"
    );

    let mysql = my(13306, "test-mysql").await;
    let dump = Arc::new(
        dumpfile::pick(&fixture("mysql84.sql"), "m".into())
            .await
            .unwrap(),
    );
    let raw = || async {
        let login = crate::sql::mysql::MyLogin {
            user: "root".into(),
            password: Some(Zeroizing::new("test-mysql".into())),
            database: None,
            tls: TlsMode::Require,
            server_name: "127.0.0.1".into(),
        };
        crate::sql::mysql::MyConn::connect(
            (reopen(Engine::Mysql, 13306))().await.unwrap(),
            login,
            reopen(Engine::Mysql, 13306),
        )
        .await
        .unwrap()
        .0
    };
    let cat = crate::sql::schemacat::from_dump(
        Conn::My(&mysql),
        cfg(Engine::Mysql, 3306, "shop", "root", "test-mysql"),
        None,
        dump,
        "m",
        server("none"),
        None,
        async |_| Ok(crate::sql::RawConn::My(raw().await)),
    )
    .await
    .unwrap();
    assert!(
        cat.tables.iter().any(|t| t.name == "orders"),
        "{:?}",
        cat.tables.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    let left = raw().await.query("select count(*) from information_schema.schemata where schema_name like 'dalan_compare_%'", 0).await.unwrap().rows;
    assert_eq!(
        left[0][0].as_deref(),
        Some("0"),
        "the scratch database stayed"
    );
}

/// D17: a backup restored into a scratch database `dalan_check_…`, counted against the source, and dropped again.
#[tokio::test]
#[ignore = "needs the Docker test servers"]
async fn l3_a_backup_check_restores_into_scratch_counts_rows_and_drops_it() {
    let _tools = stand_ins();
    psql("shop", "drop database if exists d17_src");
    // The source: what the fixture holds, with one more order than the backup has.
    let shop = pg("shop").await;
    let dump = Arc::new(
        dumpfile::pick(&fixture("pg16.custom"), "t".into())
            .await
            .unwrap(),
    );
    let (_, r) = restore(
        Conn::Pg(&shop),
        &cfg(Engine::Postgres, 5432, "shop", "app", "test-pg"),
        None,
        &fixture("pg16.custom"),
        opts(Some("d17_src")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap().outcome, "done");
    psql(
        "d17_src",
        "insert into public.orders (customer_id) values (1)",
    );
    let src = crate::sql::RawConn::Pg(Box::new(pg("d17_src").await));
    let rows = with_scratch(
        Conn::Pg(&shop),
        cfg(Engine::Postgres, 5432, "shop", "app", "test-pg"),
        None,
        dump,
        "t",
        Scratch {
            name: "test",
            environment: "staging",
            mode: RedisWriteMode::Confirm,
        },
        "dalan_check",
        crate::dbs::dumpscan::Content::All,
        async |scratch| {
            let restored = crate::sql::verify::counts(
                &crate::sql::RawConn::Pg(Box::new(pg(&scratch).await)),
                &scratch,
            )
            .await?;
            let source = crate::sql::verify::counts(&src, "d17_src").await?;
            Ok(crate::sql::verify::compare(restored, &source))
        },
    )
    .await
    .unwrap();
    let orders = rows.iter().find(|r| r.name == "public.orders").unwrap();
    assert_eq!(
        (orders.restored, orders.source, orders.same),
        (4.0, Some(5.0), false)
    );
    assert!(
        rows.iter()
            .filter(|r| r.name != "public.orders")
            .all(|r| r.same),
        "{rows:?}"
    );
    assert_eq!(
        psql(
            "shop",
            "select count(*) from pg_database where datname like 'dalan_check_%'"
        ),
        "0"
    );
    psql("shop", "drop database d17_src");
}
