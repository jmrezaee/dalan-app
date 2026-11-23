//! The gate for SQL (spec section 3, "The gate for SQL"): a script is split into statements, and each one is put in a
//! class. D3 runs only `Read`; the write, schema and dangerous classes are refused until the confirmation dialogs
//! arrive (D4). A statement that can't be understood is `Dangerous`, never `Read`.
//!
//! Splitting uses sqlparser's tokenizer, so a `;` inside a string, a dollar-quoted body or a comment is not a split.

use serde::Serialize;
use specta::Type;
use sqlparser::ast::{Expr, Query, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};

/// Ordered by how much care a statement needs: `Read < Write < Schema < Dangerous`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Read,
    Write,
    Schema,
    Dangerous,
}

/// One statement of a script, with where it sits in the text (in characters, for the editor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    /// First and one-past-last character of `text` in the script.
    pub start: usize,
    pub end: usize,
}

/// Splits a script at its `;`. Comments and blank space between statements are dropped. If the text can't be
/// tokenised (an open quote), it comes back as one piece and the classifier will refuse it.
pub fn split(sql: &str) -> Vec<Piece> {
    let chars: Vec<char> = sql.chars().collect();
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, sql).tokenize_with_location() else {
        return whole(&chars);
    };
    // Line and column (1-based) to a character index.
    let mut line_start = vec![0usize];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_start.push(i + 1);
        }
    }
    let at = |l: sqlparser::tokenizer::Location| -> usize {
        let base = line_start.get((l.line as usize).saturating_sub(1)).copied();
        (base.unwrap_or(chars.len()) + (l.column as usize).saturating_sub(1)).min(chars.len())
    };
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut has_content = false;
    let push = |start: usize, end: usize, out: &mut Vec<Piece>| {
        let slice = &chars[start..end];
        let lead = slice.iter().take_while(|c| c.is_whitespace()).count();
        let trail = slice.iter().rev().take_while(|c| c.is_whitespace()).count();
        if lead + trail < slice.len() {
            out.push(Piece {
                text: slice[lead..slice.len() - trail].iter().collect(),
                start: start + lead,
                end: end - trail,
            });
        }
    };
    for t in &tokens {
        match &t.token {
            Token::SemiColon => {
                if has_content {
                    push(start, at(t.span.start), &mut out);
                }
                start = at(t.span.end);
                has_content = false;
            }
            Token::Whitespace(
                Whitespace::Space
                | Whitespace::Newline
                | Whitespace::Tab
                | Whitespace::SingleLineComment { .. }
                | Whitespace::MultiLineComment(_),
            )
            | Token::EOF => {}
            _ => {
                if !has_content {
                    // The statement starts here: comments before it belong to nothing.
                    start = at(t.span.start);
                }
                has_content = true;
            }
        }
    }
    if has_content {
        push(start, chars.len(), &mut out);
    }
    out
}

fn whole(chars: &[char]) -> Vec<Piece> {
    let text: String = chars.iter().collect();
    if text.trim().is_empty() {
        return vec![];
    }
    vec![Piece {
        text: text.trim().to_string(),
        start: 0,
        end: chars.len(),
    }]
}

/// Every name in the statement, quoted or not, upper-cased: `"pg_terminate_backend"(pid)` is the same call.
fn names(sql: &str) -> Vec<String> {
    let dialect = PostgreSqlDialect {};
    Tokenizer::new(&dialect, sql)
        .tokenize()
        .map(|ts| {
            ts.into_iter()
                .filter_map(|t| match t {
                    Token::Word(w) => Some(w.value.to_ascii_uppercase()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Functions that act outside the transaction (they survive the rollback, or touch other sessions, the server's
/// files or other servers). A read-only transaction does not stop them, so naming one makes a statement dangerous.
fn effectful(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "PG_TERMINATE_", "PG_CANCEL_", "PG_RELOAD_", "PG_ROTATE_", "PG_ADVISORY_", "PG_TRY_ADVISORY_", "PG_READ_",
        "PG_LS_", "PG_STAT_FILE", "PG_SWITCH_", "PG_CREATE_", "PG_DROP_", "PG_REPLICATION_", "PG_PROMOTE", "PG_BACKUP_",
        "PG_START_BACKUP", "PG_STOP_BACKUP", "DBLINK", "LO_", "LOWRITE", "LOREAD", "NEXTVAL", "SETVAL", "ADMINPACK",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// `true` when the statement names a function that acts outside the transaction (see `effectful`). Such a statement
/// is never tried out before the confirmation: a rollback would not undo what it does.
pub fn calls_effectful(sql: &str) -> bool {
    names(sql).iter().any(|x| effectful(x))
}

/// `begin`, `commit`, `rollback`, `savepoint` and friends. The editor has its own Manual mode with Commit and Roll
/// back; a raw one typed in a script would end or start a transaction behind the transaction bar's back.
pub fn is_txn_control(sql: &str) -> bool {
    matches!(
        words(sql).first().map(String::as_str),
        Some("BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE")
    )
}

/// The statement the cursor is in (or the last one before it, when the cursor sits in the blank space after a `;`).
pub fn at_cursor(pieces: &[Piece], cursor: usize) -> Option<&Piece> {
    pieces
        .iter()
        .find(|p| cursor >= p.start && cursor <= p.end)
        .or_else(|| pieces.iter().rev().find(|p| p.end <= cursor))
        .or_else(|| pieces.first())
}

/// The words of a statement, upper-cased, comments skipped. Quoted names are not words.
fn words(sql: &str) -> Vec<String> {
    let dialect = PostgreSqlDialect {};
    Tokenizer::new(&dialect, sql)
        .tokenize()
        .map(|ts| {
            ts.into_iter()
                .filter_map(|t| match t {
                    Token::Word(w) if w.quote_style.is_none() => Some(w.value.to_ascii_uppercase()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn trivially_true(e: &Option<Expr>) -> bool {
    match e {
        None => true,
        Some(e) => {
            let s: String = e
                .to_string()
                .to_ascii_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
                .collect();
            matches!(s.as_str(), "true" | "1=1" | "'1'='1'" | "1<>0" | "2>1")
        }
    }
}

fn statement_class(s: &Statement) -> Class {
    match s {
        Statement::Query(q) => query_class(q),
        Statement::Update(u) => {
            if trivially_true(&u.selection) {
                Class::Dangerous
            } else {
                Class::Write
            }
        }
        Statement::Delete(d) => {
            if trivially_true(&d.selection) {
                Class::Dangerous
            } else {
                Class::Write
            }
        }
        Statement::Insert(_) | Statement::Merge { .. } => Class::Write,
        // Anything else inside a data-modifying CTE is not a plain read.
        _ => Class::Dangerous,
    }
}

fn set_expr_class(e: &SetExpr) -> Class {
    match e {
        SetExpr::Select(s) => {
            if s.into.is_some() {
                Class::Write
            } else {
                Class::Read
            }
        }
        SetExpr::Query(q) => query_class(q),
        SetExpr::SetOperation { left, right, .. } => set_expr_class(left).max(set_expr_class(right)),
        SetExpr::Values(_) | SetExpr::Table(_) => Class::Read,
        SetExpr::Insert(s) | SetExpr::Update(s) | SetExpr::Delete(s) | SetExpr::Merge(s) => {
            statement_class(s)
        }
    }
}

fn query_class(q: &Query) -> Class {
    let mut c = set_expr_class(&q.body);
    if !q.locks.is_empty() {
        c = c.max(Class::Write); // FOR UPDATE / FOR SHARE take row locks
    }
    if let Some(w) = &q.with {
        for cte in &w.cte_tables {
            c = c.max(query_class(&cte.query));
        }
    }
    c
}

/// Puts one statement (as `split` returns it) in a class.
pub fn classify(sql: &str) -> Class {
    let w = words(sql);
    let Some(first) = w.first().map(String::as_str) else {
        // Opens with a parenthesis or something else that isn't a word: only a parsed query is fine.
        return parsed_class(sql);
    };
    // Killing other sessions is dangerous wherever the call hides.
    if names(sql).iter().any(|x| effectful(x)) {
        return Class::Dangerous;
    }
    let has = |x: &str| w.iter().any(|y| y == x);
    match first {
        "SHOW" => Class::Read,
        "SELECT" | "WITH" | "VALUES" | "UPDATE" | "DELETE" => parsed_class(sql),
        // `table x` is `select * from x`; the parser doesn't know it.
        "TABLE" => parsed_class(&format!("select * from {}", sql.trim().get(5..).unwrap_or("").trim())),
        "EXPLAIN" => {
            if has("ANALYZE") || has("ANALYSE") {
                // EXPLAIN ANALYZE runs the statement: it is as risky as the statement.
                explain_inner(sql).max(Class::Write)
            } else {
                Class::Read
            }
        }
        // Arbitrary code or a shell command: as risky as anything, so the name must be typed.
        "DO" | "EXECUTE" => Class::Dangerous,
        "COPY" if has("PROGRAM") => Class::Dangerous,
        "INSERT" | "MERGE" | "COPY" | "CALL" | "LOCK" | "SET" | "RESET" | "BEGIN"
        | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE"
        | "PREPARE" | "DEALLOCATE" | "DISCARD" | "LISTEN" | "UNLISTEN" | "NOTIFY"
        | "DECLARE" | "FETCH" | "MOVE" | "CLOSE" | "IMPORT" | "SECURITY" => Class::Write,
        "VACUUM" => {
            if has("FULL") {
                Class::Dangerous
            } else {
                Class::Schema
            }
        }
        "CLUSTER" | "DROP" | "TRUNCATE" => Class::Dangerous,
        "ALTER" => {
            let second = w.get(1).map(String::as_str);
            // `alter system`, a role's password, and a column's type change (`alter column c type ...`,
            // `set data type`, which rewrites the data) are dangerous; `alter type` itself is a schema change.
            if second == Some("SYSTEM")
                || (matches!(second, Some("ROLE" | "USER")) && has("PASSWORD"))
                || (second != Some("TYPE") && has("TYPE"))
            {
                Class::Dangerous
            } else {
                Class::Schema
            }
        }
        "CREATE" => {
            if matches!(w.get(1).map(String::as_str), Some("ROLE" | "USER")) && has("PASSWORD") {
                Class::Dangerous
            } else {
                Class::Schema
            }
        }
        "COMMENT" | "GRANT" | "REVOKE" | "REFRESH" | "ANALYZE" | "ANALYSE" | "REINDEX" => {
            Class::Schema
        }
        _ => Class::Dangerous,
    }
}

/// The class of what an `EXPLAIN ANALYZE` would run. Unreadable inner statements are `Dangerous`.
fn explain_inner(sql: &str) -> Class {
    // Parse the whole statement: sqlparser nests the explained statement.
    let dialect = PostgreSqlDialect {};
    match Parser::parse_sql(&dialect, sql) {
        Ok(s) if s.len() == 1 => match &s[0] {
            Statement::Explain { statement, .. } => statement_class(statement),
            _ => Class::Dangerous,
        },
        _ => Class::Dangerous,
    }
}

fn parsed_class(sql: &str) -> Class {
    let dialect = PostgreSqlDialect {};
    match Parser::parse_sql(&dialect, sql) {
        Ok(s) if s.len() == 1 => statement_class(&s[0]),
        _ => Class::Dangerous,
    }
}

/// `true` when `sql` is a statement D3 may run: a read that goes through a cursor (a plain query).
pub fn is_query(sql: &str) -> bool {
    let dialect = PostgreSqlDialect {};
    matches!(Parser::parse_sql(&dialect, sql), Ok(s) if s.len() == 1 && matches!(s[0], Statement::Query(_)))
}

/// Formats each statement on its own lines. A statement that has comments, or that sqlparser can't read, is kept as
/// typed: formatting never changes what a statement means, and never throws a comment away.
pub fn format(sql: &str) -> String {
    let dialect = PostgreSqlDialect {};
    let pieces = split(sql);
    let has_comment = |t: &str| {
        Tokenizer::new(&dialect, t)
            .tokenize()
            .map(|ts| {
                ts.iter().any(|t| {
                    matches!(
                        t,
                        Token::Whitespace(
                            Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_)
                        )
                    )
                })
            })
            .unwrap_or(true)
    };
    let out: Vec<String> = pieces
        .iter()
        .map(|p| {
            if has_comment(&p.text) {
                return format!("{};", p.text);
            }
            match Parser::parse_sql(&dialect, &p.text) {
                Ok(s) if s.len() == 1 => format!("{:#};", s[0]),
                _ => format!("{};", p.text),
            }
        })
        .collect();
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Class::*;

    /// The unit table of the gate: every statement the spec names, in its class.
    #[test]
    fn statements_are_classified() {
        let table: &[(&str, Class)] = &[
            // read
            ("select 1", Read),
            ("select * from t where id = 1", Read),
            ("with a as (select 1) select * from a", Read),
            ("(select 1) union (select 2)", Read),
            ("values (1), (2)", Read),
            ("table orders", Read),
            ("show search_path", Read),
            ("explain select 1", Read),
            ("explain (format json) select * from t", Read),
            ("explain analyze select 1", Write),
            ("select now()", Read),
            // write
            ("insert into t values (1)", Write),
            ("update t set a = 1 where id = 2", Write),
            ("delete from t where id = 2", Write),
            ("merge into t using s on t.id = s.id when matched then delete", Write),
            ("copy t from stdin", Write),
            ("call p()", Write),
            ("do $$ begin null; end $$", Dangerous),
            ("execute p", Dangerous),
            ("copy t to program 'sh'", Dangerous),
            ("select * from t for update", Write),
            ("select * into backup from t", Write),
            ("with d as (delete from t where id = 1 returning *) select * from d", Write),
            ("explain analyze delete from t where id = 1", Write),
            ("lock table t", Write),
            ("set search_path = x", Write),
            ("begin", Write),
            ("commit", Write),
            ("rollback", Write),
            ("savepoint s", Write),
            // schema
            ("create table t (a int)", Schema),
            ("alter table t add column b int", Schema),
            ("comment on table t is 'x'", Schema),
            ("grant select on t to u", Schema),
            ("revoke select on t from u", Schema),
            ("refresh materialized view v", Schema),
            ("vacuum t", Schema),
            ("analyze t", Schema),
            ("reindex table t", Schema),
            // dangerous
            ("drop table t", Dangerous),
            ("truncate t", Dangerous),
            ("update t set a = 1", Dangerous),
            ("delete from t", Dangerous),
            ("delete from t where true", Dangerous),
            ("delete from t where 1 = 1", Dangerous),
            ("alter table t alter column a type bigint", Dangerous),
            ("alter table t alter column a set data type text", Dangerous),
            ("vacuum full t", Dangerous),
            ("cluster t", Dangerous),
            ("alter system set work_mem = '1GB'", Dangerous),
            ("create role r password 'x'", Dangerous),
            ("alter role r with password 'x'", Dangerous),
            ("select pg_terminate_backend(123)", Dangerous),
            ("select pg_cancel_backend(pid) from pg_stat_activity", Dangerous),
            ("select \"pg_terminate_backend\"(1)", Dangerous),
            ("select pg_catalog.\"PG_CANCEL_BACKEND\"(1)", Dangerous),
            ("select /* x */ pg_terminate_backend /* y */ (1)", Dangerous),
            ("select pg_advisory_lock(1)", Dangerous),
            ("select pg_reload_conf()", Dangerous),
            ("select * from dblink('host=x', 'select 1') as t(a int)", Dangerous),
            ("select pg_read_file('/etc/passwd')", Dangerous),
            ("select nextval('s')", Dangerous),
            ("select pg_sleep(1)", Read),
            ("(select 1)", Read),
            ("-- note\nselect 1", Read),
            ("show all", Read),
            ("selec 1", Dangerous),
            ("select * from", Dangerous),
            ("frobnicate everything", Dangerous),
        ];
        for (sql, want) in table {
            assert_eq!(classify(sql), *want, "{sql}");
        }
    }

    #[test]
    fn a_script_splits_at_real_semicolons_only() {
        let sql = "select 'a;b';\n-- one; two\nselect $$x;y$$ ; /* ; */ select 3;;  ";
        let p = split(sql);
        let texts: Vec<&str> = p.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, vec!["select 'a;b'", "select $$x;y$$", "select 3"]);
        let chars: Vec<char> = sql.chars().collect();
        for piece in &p {
            let slice: String = chars[piece.start..piece.end].iter().collect();
            assert_eq!(slice, piece.text);
        }
        assert!(split("   -- nothing\n ").is_empty());
        // An open quote can't be split: one piece, and it is refused.
        let open = split("select 'abc; drop table t");
        assert_eq!(open.len(), 1);
        assert_eq!(classify(&open[0].text), Dangerous);
    }

    #[test]
    fn transaction_verbs_and_effectful_calls_are_recognised() {
        for t in ["begin", "COMMIT", "rollback to savepoint a", "savepoint a", "start transaction", "end", "release a"] {
            assert!(is_txn_control(t), "{t}");
        }
        assert!(!is_txn_control("select 1") && !is_txn_control("update t set begin = 1 where id = 2"));
        assert!(calls_effectful("update t set a = pg_terminate_backend(1) where id = 1"));
        assert!(calls_effectful("insert into t select \"dblink_exec\"('x', 'y')"));
        assert!(!calls_effectful("update t set a = 1 where id = 2"));
    }

    #[test]
    fn the_cursor_picks_its_statement() {
        let p = split("select 1;\nselect 2;\n");
        assert_eq!(at_cursor(&p, 3).unwrap().text, "select 1");
        assert_eq!(at_cursor(&p, 12).unwrap().text, "select 2");
        // After the last semicolon: the statement before.
        assert_eq!(at_cursor(&p, 20).unwrap().text, "select 2");
        assert!(at_cursor(&[], 0).is_none());
    }

    #[test]
    fn only_a_plain_query_goes_through_a_cursor() {
        assert!(is_query("select 1"));
        assert!(is_query("with a as (select 1) select * from a"));
        assert!(!is_query("show all"));
        assert!(!is_query("explain select 1"));
    }

    #[test]
    fn format_keeps_meaning_and_comments() {
        let f = format("select a,b from t where x=1; select 2");
        assert!(f.contains("SELECT") && f.matches(';').count() == 2);
        let kept = format("select 1 -- why\n");
        assert!(kept.contains("-- why"));
        assert!(format("selec nonsense").starts_with("selec nonsense"));
    }
}
