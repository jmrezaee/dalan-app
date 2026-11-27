//! The plain-SQL dump scanner (D12, S48, F31; replica/databases/restore-copy.md §2 "Safety"). One pass over a
//! `pg_dump -Fp` / `pg_dumpall` / `mysqldump` / `mariadb-dump` file, streaming, in constant memory (a statement's
//! first 64 KB at most). It does three jobs:
//!
//! 1. **Preview:** the objects, the rows of each table (exact for COPY blocks, counted tuples for INSERTs), the
//!    databases the file switches between, owners, grants, extensions and DEFINER clauses.
//! 2. **Safety:** `psql` and `mysql` have commands that reach the host (`\!`, `\o |cmd`, `system`, `source`, …). Every
//!    client command is read before anything runs; only the few a dump needs pass. Statements that reach outside the
//!    database (`COPY … PROGRAM`, server files, `LOAD DATA`, `INTO OUTFILE`, untrusted languages, `ALTER SYSTEM`, …)
//!    refuse the file the same way, naming the line.
//! 3. **Rewrite while sending:** the same scanner sits between the file and the tool's input, so what was checked is
//!    what the tool gets. It drops what would land outside the chosen target (`CREATE DATABASE`, `\connect`, `USE`,
//!    other databases' sections), owners and grants when asked, roles unless asked, and applies the "existing
//!    objects" and "schema only / data only" choices.
//!
//! The lexer follows the *clients'* rules (where `psql`/`mysql` think a string, comment or command starts), works on
//! bytes (every syntax character is ASCII, so UTF-8 text passes through untouched) and holds back a small carry at the
//! end of each chunk so no token is split between two chunks.

use std::collections::BTreeMap;

/// Which client reads the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `psql`
    Pg,
    /// `mysql` / `mariadb`
    My,
}

/// What to do with objects that already exist at the target (§2 "Existing objects").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Existing {
    /// Clashes are found before anything runs (the plan lists them); nothing is dropped.
    Stop,
    /// Drop and replace what the dump contains.
    Replace,
    /// Data only, into the tables that are there.
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    All,
    Schema,
    Data,
}

/// How the stream is changed on its way to the tool. `None` in [`Scanner::new`] means preview only.
#[derive(Debug, Clone)]
pub struct Rewrite {
    /// Keep only this database's section of a multi-database file (`\connect x` / `USE x`). `None`: every section.
    pub section: Option<String>,
    pub skip_owner_acl: bool,
    pub include_roles: bool,
    pub existing: Existing,
    pub content: Content,
    /// MySQL: drop `DEFINER=user@host` so the restoring user owns views, routines and triggers.
    pub strip_definer: bool,
    /// MariaDB's `/*M!999999\- enable the sandbox mode */` first line: kept for the MariaDB client (it turns the
    /// sandbox on), dropped for Oracle's `mysql`, which stops at it.
    pub keep_sandbox_line: bool,
}

/// Why a file is refused: the line and what it does, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub line: u64,
    pub what: String,
}

impl Refusal {
    pub fn message(&self) -> String {
        format!(
            "Line {} {}. Dalan won't restore this file.",
            group(self.line),
            self.what
        )
    }
}

/// 18204 → "18,204".
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One object a dump creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obj {
    pub kind: String,
    /// The database section it is in (multi-database files).
    pub database: Option<String>,
    /// `schema.name` as the dump spells it, quotes removed, unquoted names in lower case.
    pub name: String,
}

/// What a scan found, filled as it goes.
#[derive(Debug, Default, Clone)]
pub struct PlainInfo {
    pub objects: Vec<Obj>,
    /// Objects past the cap are counted, not kept.
    pub objects_more: u64,
    /// Rows per table, and whether the count is exact (COPY lines) or counted from INSERT tuples.
    pub rows: BTreeMap<String, (u64, bool)>,
    /// Databases the file names (`CREATE DATABASE`, `\connect`, `USE`), in order of first appearance.
    pub databases: Vec<String>,
    pub owners: Vec<String>,
    pub grants: u64,
    pub extensions: Vec<String>,
    pub definers: u64,
    pub roles: u64,
    pub superuser_roles: u64,
    pub gtid: u64,
    pub statements: u64,
    pub made_by: Option<String>,
    pub server_version: Option<String>,
    pub source_db: Option<String>,
    pub encoding: Option<String>,
    pub mariadb_sandbox: bool,
    pub cluster: bool,
    pub lines: u64,
}

const MAX_OBJECTS: usize = 5000;
/// Bytes of a statement held for deciding what to do with it. Longer statements (big INSERTs, function bodies) are
/// decided from their start and then streamed.
const HEAD_CAP: usize = 64 * 1024;
/// Tokens of a statement kept for the rules (every rule looks at the start of a statement).
const HEAD_TOKENS: usize = 48;
/// Bytes held back at the end of a chunk so a lookahead never runs off it (a dollar tag, `/*M!999999`, a delimiter).
const CARRY: usize = 160;
/// No dump has a client command line this long.
const MAX_COMMAND_LINE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokKind {
    Word,
    QIdent,
    Str,
    Punct(u8),
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TokKind,
    /// Words: upper case. Quoted identifiers: the name without quotes. Strings: the text (first 256 bytes).
    norm: String,
    /// Identifiers as written, for names put back into SQL.
    raw: String,
}

/// Where the lexer is. `Copy`, so matching on it never clones (the dollar tag lives in `Scanner::tag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lex {
    Normal,
    LineComment,
    Block(u32),
    /// `escapes`: a backslash escapes the next byte (MySQL, PG `E''`, or `standard_conforming_strings = off`).
    Str {
        quote: u8,
        escapes: bool,
    },
    QIdent(u8),
    Dollar,
    /// PG: the data lines after `COPY … FROM stdin;`, up to `\.`.
    CopyData,
    /// A client command line being collected to its end.
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Undecided,
    Keep,
    Drop,
}

/// What a statement is, for the "schema only / data only" filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Setting,
    Data,
    Schema,
}

#[derive(Default)]
struct Stmt {
    toks: Vec<Tok>,
    ntoks: u64,
    has_code: bool,
    /// Where the statement's code starts in `Scanner::unit` (comments before it come first).
    code_at: usize,
    first_line: u64,
    depth: i32,
    after_values: bool,
    values_rows: u64,
    prev_word: String,
    copy_dir_next: bool,
    lang_next: bool,
    is_copy_stdin: bool,
}

pub struct Scanner {
    d: Dialect,
    client_checks: bool,
    rw: Option<Rewrite>,
    pub info: PlainInfo,
    lex: Lex,
    tag: Vec<u8>,
    line: u64,
    at_line_start: bool,
    carry: Vec<u8>,
    /// Bytes of the current unit (a statement and the comments before it) not yet written or dropped.
    unit: Vec<u8>,
    decision: Decision,
    stmt: Stmt,
    word: Vec<u8>,
    qword: Vec<u8>,
    str_buf: Vec<u8>,
    delimiter: Vec<u8>,
    in_versioned: bool,
    /// `standard_conforming_strings` as psql sees it; starts at the server's value.
    scs: bool,
    scs_default: bool,
    section: Option<String>,
    command: Vec<u8>,
    /// The first bytes of the current COPY data line (enough to see `\.`).
    copy_head: Vec<u8>,
    copy_keep: bool,
    copy_table: Option<String>,
    copy_pending: bool,
    /// The last two bytes seen in code, for `E'` and word boundaries.
    prev: [u8; 2],
    started: bool,
    header: Vec<u8>,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn unquote_ident(raw: &[u8]) -> String {
    if raw.len() >= 2 && matches!(raw[0], b'"' | b'`') && raw[raw.len() - 1] == raw[0] {
        let q = raw[0] as char;
        return String::from_utf8_lossy(&raw[1..raw.len() - 1])
            .replace(&format!("{q}{q}"), &q.to_string());
    }
    String::from_utf8_lossy(raw).into_owned()
}

impl Scanner {
    /// `server_scs`: the target's `standard_conforming_strings` (on for every server since 9.1 unless configured).
    pub fn new(d: Dialect, rw: Option<Rewrite>, server_scs: bool) -> Scanner {
        Scanner {
            d,
            client_checks: true,
            rw,
            info: PlainInfo::default(),
            lex: Lex::Normal,
            tag: Vec::new(),
            line: 1,
            at_line_start: true,
            carry: Vec::new(),
            unit: Vec::new(),
            decision: Decision::Undecided,
            stmt: Stmt {
                first_line: 1,
                ..Stmt::default()
            },
            word: Vec::new(),
            qword: Vec::new(),
            str_buf: Vec::new(),
            delimiter: b";".to_vec(),
            in_versioned: false,
            scs: server_scs,
            scs_default: server_scs,
            section: None,
            command: Vec::new(),
            copy_head: Vec::new(),
            copy_keep: false,
            copy_table: None,
            copy_pending: false,
            prev: *b"\n\n",
            started: false,
            header: Vec::new(),
        }
    }

    /// For SQL a server runs directly (a pg archive's TOC entries): there is no client to send commands to.
    pub fn without_client_checks(mut self) -> Scanner {
        self.client_checks = false;
        self
    }

    /// Feeds the next chunk. Bytes for the tool are appended to `out` (rewrite mode; a preview leaves it empty).
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), Refusal> {
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(chunk);
        let end = buf.len().saturating_sub(CARRY);
        let used = self.run(&buf, end, out)?;
        self.carry = buf[used..].to_vec();
        Ok(())
    }

    /// The end of the file: the rest is scanned and the last statement decided.
    pub fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), Refusal> {
        let buf = std::mem::take(&mut self.carry);
        self.run(&buf, buf.len(), out)?;
        self.flush_word()?;
        if self.lex == Lex::Command {
            self.end_command(out)?;
        }
        if self.stmt.has_code {
            self.end_statement(out)?;
        } else {
            self.emit_between(out);
        }
        self.info.lines = self.line;
        let h = String::from_utf8_lossy(&self.header).into_owned();
        header_details(&mut self.info, &h);
        Ok(())
    }

    fn refuse(&self, line: u64, what: impl Into<String>) -> Refusal {
        Refusal {
            line,
            what: what.into(),
        }
    }

    /// Lexes `buf[..end]` (looking ahead up to `buf.len()`) and returns how far it got.
    fn run(&mut self, buf: &[u8], end: usize, out: &mut Vec<u8>) -> Result<usize, Refusal> {
        let mut i = 0;
        if !self.started && end > 0 {
            self.started = true;
            if self.d == Dialect::My && buf.starts_with(b"/*M!999999\\- ") {
                i = self.sandbox_line(buf, out)?;
            }
        }
        while i < end {
            if self.line <= 40 && self.header.len() < 8192 {
                self.header.push(buf[i]);
            }
            let b = buf[i];
            let n1 = buf.get(i + 1).copied();
            match self.lex {
                Lex::Command => {
                    self.command.push(b);
                    i += 1;
                    if b == b'\n' {
                        self.newline();
                        self.end_command(out)?;
                    } else if self.command.len() > MAX_COMMAND_LINE {
                        return Err(
                            self.refuse(self.line, "has a client command line that never ends")
                        );
                    }
                    continue;
                }
                Lex::CopyData => {
                    // Data lines pass as they are (not lexed); `\.` alone on a line ends the block.
                    let stop = buf[i..end]
                        .iter()
                        .position(|&c| c == b'\n')
                        .map_or(end, |p| i + p + 1);
                    let seg = &buf[i..stop];
                    if self.copy_head.len() < 4 {
                        let take = (4 - self.copy_head.len()).min(seg.len());
                        self.copy_head.extend_from_slice(&seg[..take]);
                    }
                    if self.copy_keep {
                        out.extend_from_slice(seg);
                    }
                    i = stop;
                    if seg.last() == Some(&b'\n') {
                        let h = std::mem::take(&mut self.copy_head);
                        let is_end = matches!(h.as_slice(), b"\\.\n" | b"\\.\r\n" | b"\\.\r");
                        self.line += 1;
                        if is_end {
                            self.lex = Lex::Normal;
                            self.at_line_start = true;
                        } else if let Some(t) = &self.copy_table {
                            if let Some(r) = self.info.rows.get_mut(t) {
                                r.0 += 1;
                            }
                        }
                    }
                    continue;
                }
                _ => {}
            }
            self.unit.push(b);
            match self.lex {
                Lex::LineComment => {
                    if b == b'\n' {
                        self.lex = Lex::Normal;
                        self.newline();
                    }
                    i += 1;
                }
                Lex::Block(depth) => {
                    if b == b'*' && n1 == Some(b'/') {
                        self.unit.push(b'/');
                        i += 2;
                        self.lex = if depth > 1 {
                            Lex::Block(depth - 1)
                        } else {
                            Lex::Normal
                        };
                        continue;
                    }
                    // PG block comments nest.
                    if self.d == Dialect::Pg && b == b'/' && n1 == Some(b'*') {
                        self.unit.push(b'*');
                        i += 2;
                        self.lex = Lex::Block(depth + 1);
                        continue;
                    }
                    if b == b'\n' {
                        self.newline();
                    }
                    i += 1;
                }
                Lex::Str { quote, escapes } => {
                    if b == b'\n' {
                        self.newline();
                    }
                    if escapes && b == b'\\' {
                        if let Some(n) = n1 {
                            self.unit.push(n);
                            self.str_push(b);
                            self.str_push(n);
                            if n == b'\n' {
                                self.newline();
                            }
                        }
                        i += 2;
                        continue;
                    }
                    if b == quote {
                        if n1 == Some(quote) {
                            self.unit.push(quote);
                            self.str_push(quote);
                            i += 2;
                            continue;
                        }
                        self.lex = Lex::Normal;
                        let s = String::from_utf8_lossy(&self.str_buf).into_owned();
                        self.str_buf.clear();
                        self.token(TokKind::Str, s, String::new())?;
                        i += 1;
                        continue;
                    }
                    self.str_push(b);
                    i += 1;
                }
                Lex::QIdent(q) => {
                    if b == b'\n' {
                        self.newline();
                    }
                    self.qword.push(b);
                    if b == q {
                        if n1 == Some(q) {
                            self.unit.push(q);
                            self.qword.push(q);
                            i += 2;
                            continue;
                        }
                        self.lex = Lex::Normal;
                        let raw = std::mem::take(&mut self.qword);
                        let norm = unquote_ident(&raw);
                        self.token(
                            TokKind::QIdent,
                            norm,
                            String::from_utf8_lossy(&raw).into_owned(),
                        )?;
                    }
                    i += 1;
                }
                Lex::Dollar => {
                    if b == b'\n' {
                        self.newline();
                    }
                    if b == b'$' && buf[i..].starts_with(&self.tag) {
                        self.unit.extend_from_slice(&self.tag[1..]);
                        i += self.tag.len();
                        self.lex = Lex::Normal;
                        self.token(TokKind::Str, String::new(), String::new())?;
                        continue;
                    }
                    i += 1;
                }
                Lex::Normal => {
                    i = self.normal(buf, i, out)?;
                    self.prev = [self.prev[1], b];
                }
                Lex::CopyData | Lex::Command => unreachable!("handled above"),
            }
            // A long statement is decided from its start, then streamed.
            if self.unit.len() > HEAD_CAP && self.stmt.has_code {
                if self.decision == Decision::Undecided {
                    self.decide(out);
                }
                if self.decision == Decision::Keep {
                    out.extend_from_slice(&self.unit);
                }
                self.unit.clear();
                self.stmt.code_at = 0;
            }
        }
        Ok(i)
    }

    /// MariaDB's sandbox line, the only client command a dump may carry before its first statement.
    fn sandbox_line(&mut self, buf: &[u8], out: &mut Vec<u8>) -> Result<usize, Refusal> {
        let nl = buf
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |p| p + 1);
        let line = &buf[..nl];
        if nl > 80 || !line.trim_ascii_end().ends_with(b"*/") {
            return Err(self.refuse(
                1,
                "has a client command (`\\-`) that isn't MariaDB's sandbox line",
            ));
        }
        self.info.mariadb_sandbox = true;
        if self.rw.as_ref().is_some_and(|r| r.keep_sandbox_line) {
            out.extend_from_slice(line);
        }
        self.line += 1;
        Ok(nl)
    }

    fn str_push(&mut self, b: u8) {
        if self.str_buf.len() < 256 {
            self.str_buf.push(b);
        }
    }

    fn newline(&mut self) {
        self.line += 1;
        self.at_line_start = true;
    }

    fn mark_code(&mut self) {
        if !self.stmt.has_code {
            self.stmt.has_code = true;
            self.stmt.code_at = self.unit.len().saturating_sub(1);
        }
    }

    /// One byte in code (`self.unit` already holds it). Returns where to go on.
    fn normal(&mut self, buf: &[u8], i: usize, out: &mut Vec<u8>) -> Result<usize, Refusal> {
        let b = buf[i];
        let n1 = buf.get(i + 1).copied();
        let line_start = self.at_line_start;
        if !b.is_ascii_whitespace() {
            self.at_line_start = false;
        }
        // MySQL: the statement delimiter in force (`;`, or `;;` / `$$` after DELIMITER) ends a statement.
        if self.d == Dialect::My && !self.in_versioned && buf[i..].starts_with(&self.delimiter) {
            self.flush_word()?;
            let rest = self.delimiter.len();
            self.unit.extend_from_slice(&buf[i + 1..i + rest]);
            if self.stmt.has_code {
                self.end_statement(out)?;
            }
            return Ok(i + rest);
        }
        if is_ident(b) && !(b == b'$' && self.word.is_empty() && self.d == Dialect::Pg) {
            // MySQL long-form client commands: the first word of a line, before a statement has any code.
            if self.d == Dialect::My
                && line_start
                && self.word.is_empty()
                && !self.stmt.has_code
                && !self.in_versioned
            {
                let w: String = buf[i..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic() || **c == b'_')
                    .take(32)
                    .map(|c| c.to_ascii_lowercase() as char)
                    .collect();
                let ends_word = buf.get(i + w.len()).is_none_or(|c| !is_ident(*c));
                if ends_word && MY_COMMANDS.contains(&w.as_str()) {
                    self.unit.pop();
                    self.emit_between(out);
                    self.command.clear();
                    self.command.push(b);
                    self.lex = Lex::Command;
                    return Ok(i + 1);
                }
            }
            self.mark_code();
            self.word.push(b);
            return Ok(i + 1);
        }
        self.flush_word()?;
        match b {
            b'\n' => {
                self.newline();
                if self.copy_pending {
                    // The COPY statement's line is over: its data starts on the next one.
                    self.copy_pending = false;
                    self.lex = Lex::CopyData;
                    self.copy_head.clear();
                    if self.copy_keep {
                        out.extend_from_slice(&self.unit);
                    }
                    self.unit.clear();
                }
                Ok(i + 1)
            }
            b'-' if n1 == Some(b'-')
                && (self.d == Dialect::Pg
                    || buf
                        .get(i + 2)
                        .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_control())) =>
            {
                self.lex = Lex::LineComment;
                Ok(i + 1)
            }
            b'#' if self.d == Dialect::My => {
                self.lex = Lex::LineComment;
                Ok(i + 1)
            }
            b'/' if n1 == Some(b'*') => {
                // MySQL: `/*!12345 … */`, `/*M!…*/` and `/*+ … */` are code the server runs.
                let rest = &buf[i + 2..];
                if self.d == Dialect::My
                    && (rest.first() == Some(&b'!')
                        || rest.starts_with(b"M!")
                        || rest.first() == Some(&b'+'))
                {
                    let mut j = i + 2;
                    if buf.get(j) == Some(&b'M') {
                        j += 1;
                    }
                    j += 1;
                    while buf.get(j).is_some_and(u8::is_ascii_digit) {
                        j += 1;
                    }
                    self.mark_code();
                    self.unit.extend_from_slice(&buf[i + 1..j]);
                    self.in_versioned = true;
                    return Ok(j);
                }
                self.unit.push(b'*');
                self.lex = Lex::Block(1);
                Ok(i + 2)
            }
            b'*' if n1 == Some(b'/') && self.in_versioned => {
                self.unit.push(b'/');
                self.in_versioned = false;
                Ok(i + 2)
            }
            b'\'' | b'"' if self.d == Dialect::My => {
                self.mark_code();
                self.lex = Lex::Str {
                    quote: b,
                    escapes: true,
                };
                Ok(i + 1)
            }
            b'`' if self.d == Dialect::My => {
                self.mark_code();
                self.qword.clear();
                self.qword.push(b);
                self.lex = Lex::QIdent(b'`');
                Ok(i + 1)
            }
            b'\'' => {
                self.mark_code();
                // `E'…'` takes backslash escapes; so does every string while standard_conforming_strings is off.
                let e = matches!(self.prev[1], b'E' | b'e') && !is_ident(self.prev[0]);
                self.lex = Lex::Str {
                    quote: b'\'',
                    escapes: e || !self.scs,
                };
                Ok(i + 1)
            }
            b'"' => {
                self.mark_code();
                self.qword.clear();
                self.qword.push(b);
                self.lex = Lex::QIdent(b'"');
                Ok(i + 1)
            }
            b'$' => {
                // PG: `$tag$` or `$$` opens a dollar-quoted string (not `$1`, and not inside a word).
                let mut j = i + 1;
                while buf
                    .get(j)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 0x80)
                {
                    j += 1;
                }
                self.mark_code();
                let opens = buf.get(j) == Some(&b'$')
                    && buf.get(i + 1).is_none_or(|c| !c.is_ascii_digit())
                    && !is_ident(self.prev[1]);
                if opens {
                    self.tag = buf[i..=j].to_vec();
                    self.unit.extend_from_slice(&buf[i + 1..=j]);
                    self.lex = Lex::Dollar;
                    return Ok(j + 1);
                }
                Ok(i + 1)
            }
            b'\\' if self.client_checks => {
                self.unit.pop();
                self.backslash(buf, i, out)
            }
            b';' if self.d == Dialect::Pg => {
                if self.stmt.has_code {
                    self.end_statement(out)?;
                }
                Ok(i + 1)
            }
            _ => {
                if !b.is_ascii_whitespace() {
                    self.mark_code();
                    self.token(TokKind::Punct(b), String::new(), String::new())?;
                }
                Ok(i + 1)
            }
        }
    }

    /// A backslash in code is a client command. PG: to the end of the line; MySQL: `\` and one character.
    fn backslash(&mut self, buf: &[u8], i: usize, out: &mut Vec<u8>) -> Result<usize, Refusal> {
        let line = self.line;
        if self.d == Dialect::My {
            let c = buf.get(i + 1).copied().unwrap_or(b' ');
            return Err(self.refuse(
                line,
                format!("has a client command (`\\{}`){}", c as char, my_effect(c)),
            ));
        }
        if self.stmt.has_code {
            let name: String = buf[i + 1..]
                .iter()
                .take_while(|c| !c.is_ascii_whitespace())
                .take(16)
                .map(|&c| c as char)
                .collect();
            return Err(self.refuse(
                line,
                format!(
                    "has a client command (`\\{name}`) inside a statement{}",
                    pg_effect(&name)
                ),
            ));
        }
        self.emit_between(out);
        self.command.clear();
        self.command.push(b'\\');
        self.lex = Lex::Command;
        Ok(i + 1)
    }

    /// A whole client command line was read: the allowed ones pass (or are dropped); anything else refuses the file.
    fn end_command(&mut self, out: &mut Vec<u8>) -> Result<(), Refusal> {
        self.lex = Lex::Normal;
        let line_no = if self.command.ends_with(b"\n") {
            self.line - 1
        } else {
            self.line
        };
        let raw = std::mem::take(&mut self.command);
        let text = String::from_utf8_lossy(&raw).into_owned();
        let t = text.trim();
        if self.d == Dialect::Pg {
            let body = &t[1..];
            let name: String = body.chars().take_while(|c| !c.is_whitespace()).collect();
            let rest = &body[name.len()..];
            let args = rest.trim();
            if rest.contains('\\') {
                return Err(self.refuse(line_no, "has two client commands on one line"));
            }
            match name.as_str() {
                "connect" | "c" => {
                    // Dropped: everything lands in the chosen target.
                    self.switch_section(connect_target(args));
                    Ok(())
                }
                // pg_dump 17.6+ fences its output with these. Dalan's scanner does that job, and older psql versions
                // stop at them, so they are not sent.
                "restrict" | "unrestrict" if args.chars().all(|c| c.is_ascii_alphanumeric()) => {
                    Ok(())
                }
                // Dalan sets ON_ERROR_STOP itself; the file doesn't get to undo it.
                "set" if args.split_whitespace().next() == Some("ON_ERROR_STOP") => Ok(()),
                "encoding"
                    if !args.is_empty()
                        && args
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
                {
                    self.info.encoding.get_or_insert_with(|| args.to_string());
                    if self.rw.is_some() && self.keep_now() {
                        out.extend_from_slice(&raw);
                    }
                    Ok(())
                }
                _ => Err(self.refuse(
                    line_no,
                    format!("has a client command (`\\{name}`){}", pg_effect(&name)),
                )),
            }
        } else {
            let name = t
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            match name.as_str() {
                "delimiter" => {
                    let d = t.split_whitespace().nth(1).unwrap_or("");
                    if d.is_empty() || d.contains('\\') || d.len() > 8 {
                        return Err(
                            self.refuse(line_no, "sets a statement delimiter Dalan can't follow")
                        );
                    }
                    self.delimiter = d.as_bytes().to_vec();
                    if self.rw.is_some() {
                        out.extend_from_slice(&raw);
                    }
                    Ok(())
                }
                "use" => {
                    let delim = String::from_utf8_lossy(&self.delimiter).into_owned();
                    let db = t[3..]
                        .trim()
                        .trim_end_matches(delim.as_str())
                        .trim_end_matches(';')
                        .trim();
                    self.switch_section(Some(unquote_ident(db.as_bytes())));
                    Ok(())
                }
                _ => Err(self.refuse(
                    line_no,
                    format!("has a client command (`{name}`){}", my_named_effect(&name)),
                )),
            }
        }
    }

    fn switch_section(&mut self, db: Option<String>) {
        if let Some(db) = db.filter(|d| !d.is_empty()) {
            if !self.info.databases.contains(&db) {
                self.info.databases.push(db.clone());
            }
            self.section = Some(db);
        }
    }

    /// The current section is one being restored.
    fn keep_now(&self) -> bool {
        match self.rw.as_ref().and_then(|r| r.section.as_ref()) {
            None => true,
            Some(want) => self.section.as_ref().is_none_or(|s| s == want),
        }
    }

    fn flush_word(&mut self) -> Result<(), Refusal> {
        if !self.word.is_empty() {
            let raw = std::mem::take(&mut self.word);
            let s = String::from_utf8_lossy(&raw).into_owned();
            self.token(TokKind::Word, s.to_ascii_uppercase(), s)?;
        }
        Ok(())
    }

    /// Every token passes here: the rules that can fire anywhere in a statement, row counting, and the head kept
    /// for the decision.
    fn token(&mut self, kind: TokKind, norm: String, raw: String) -> Result<(), Refusal> {
        let line = self.line;
        let d = self.d;
        let st = &mut self.stmt;
        if st.ntoks == 0 {
            st.first_line = line;
        }
        st.ntoks += 1;
        let first = st.toks.first().map(|t| t.norm.clone()).unwrap_or_default();
        let name = match kind {
            TokKind::Word => norm.clone(),
            TokKind::QIdent => norm.to_ascii_uppercase(),
            _ => String::new(),
        };
        // Functions that reach the server's files or programs, wherever they are called.
        if matches!(
            name.as_str(),
            "LO_IMPORT"
                | "LO_EXPORT"
                | "PG_READ_FILE"
                | "PG_READ_BINARY_FILE"
                | "PG_LS_DIR"
                | "PG_STAT_FILE"
                | "PG_FILE_WRITE"
                | "LOAD_FILE"
                | "SYS_EXEC"
                | "SYS_EVAL"
                | "SONAME"
        ) {
            let shown = if raw.is_empty() {
                norm.to_ascii_lowercase()
            } else {
                raw.clone()
            };
            return Err(self.refuse(
                line,
                format!("calls {shown} on the server, which reaches outside the database"),
            ));
        }
        if name == "NO_BACKSLASH_ESCAPES"
            || (kind == TokKind::Str
                && d == Dialect::My
                && norm.to_ascii_lowercase().contains("no_backslash_escapes"))
        {
            return Err(self.refuse(line, "turns on NO_BACKSLASH_ESCAPES, which changes how the client reads the rest of the file"));
        }
        if kind == TokKind::Str && norm.eq_ignore_ascii_case("standard_conforming_strings") {
            return Err(self.refuse(
                line,
                "changes how strings are read in a way Dalan can't follow",
            ));
        }
        let st = &mut self.stmt;
        // COPY: only `FROM stdin` / `TO stdout`.
        if st.copy_dir_next {
            st.copy_dir_next = false;
            match (&kind, name.as_str()) {
                (TokKind::Word, "STDIN") => st.is_copy_stdin = true,
                (TokKind::Word, "STDOUT") => {}
                (TokKind::Word, "PROGRAM") => {
                    return Err(self.refuse(
                        line,
                        "runs a program on the database server (`COPY … PROGRAM`)",
                    ))
                }
                _ => {
                    return Err(self.refuse(
                        line,
                        "reads or writes a file on the database server (`COPY` with a file name)",
                    ))
                }
            }
        }
        let st = &mut self.stmt;
        if first == "COPY"
            && st.depth == 0
            && kind == TokKind::Word
            && (norm == "FROM" || norm == "TO")
        {
            st.copy_dir_next = true;
        }
        if st.prev_word == "INTO" && matches!(name.as_str(), "OUTFILE" | "DUMPFILE") {
            return Err(self.refuse(
                line,
                "writes a file on the database server (`INTO OUTFILE`)",
            ));
        }
        if st.lang_next {
            st.lang_next = false;
            let lang = name.to_ascii_lowercase();
            if !matches!(lang.as_str(), "plpgsql" | "sql") {
                return Err(self.refuse(
                    line,
                    format!("runs a code block in {lang}, which can run programs on the server"),
                ));
            }
        }
        let st = &mut self.stmt;
        if first == "DO" && kind == TokKind::Word && norm == "LANGUAGE" {
            st.lang_next = true;
        }
        // INSERT … VALUES (…), (…): one row per top-level tuple.
        match kind {
            TokKind::Punct(b'(') => {
                if st.depth == 0 && st.after_values {
                    st.values_rows += 1;
                }
                st.depth += 1;
            }
            TokKind::Punct(b')') => st.depth -= 1,
            TokKind::Word if norm == "VALUES" || norm == "VALUE" => st.after_values = true,
            _ => {}
        }
        st.prev_word = if kind == TokKind::Word {
            norm.clone()
        } else {
            String::new()
        };
        if st.toks.len() < HEAD_TOKENS {
            st.toks.push(Tok { kind, norm, raw });
            if st.toks.len() == 3 || st.toks.len() == 8 {
                self.check_head(false)?;
            }
        }
        Ok(())
    }

    /// Rules read from a statement's opening words. `whole`: the statement has ended.
    fn check_head(&self, whole: bool) -> Result<(), Refusal> {
        let toks = &self.stmt.toks;
        let w: Vec<&str> = toks
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .take(6)
            .map(|t| t.norm.as_str())
            .collect();
        let line = self.stmt.first_line;
        if w.starts_with(&["LOAD", "DATA"]) || w.starts_with(&["LOAD", "XML"]) {
            return Err(self.refuse(line, "loads a file into the database (`LOAD DATA`)"));
        }
        if w.starts_with(&["ALTER", "SYSTEM"]) {
            return Err(self.refuse(line, "changes the server's configuration (`ALTER SYSTEM`)"));
        }
        if w.starts_with(&["INSTALL", "PLUGIN"]) || w.starts_with(&["INSTALL", "COMPONENT"]) {
            return Err(self.refuse(line, "installs a server plugin"));
        }
        // CREATE [OR REPLACE] [TRUSTED] [PROCEDURAL] LANGUAGE x: only plpgsql (old dumps create it).
        if toks.first().is_some_and(|t| t.norm == "CREATE") {
            if let Some(p) = toks.iter().position(|t| t.norm == "LANGUAGE") {
                let lead_ok = toks[1..p].iter().all(|t| {
                    matches!(t.norm.as_str(), "OR" | "REPLACE" | "TRUSTED" | "PROCEDURAL")
                });
                if lead_ok {
                    match toks.get(p + 1) {
                        Some(t) if t.norm.eq_ignore_ascii_case("plpgsql") => {}
                        None if !whole => {}
                        _ => return Err(self.refuse(
                            line,
                            "creates a procedural language, which can run programs on the server",
                        )),
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes (or drops) what sits between statements: comments and blank lines.
    fn emit_between(&mut self, out: &mut Vec<u8>) {
        if self.rw.is_some() && self.keep_now() {
            out.extend_from_slice(&self.unit);
        }
        self.unit.clear();
    }

    /// The statement is complete.
    fn end_statement(&mut self, out: &mut Vec<u8>) -> Result<(), Refusal> {
        self.flush_word()?;
        self.check_head(true)?;
        if self.decision == Decision::Undecided {
            self.decide(out);
        }
        if self.decision == Decision::Keep {
            out.extend_from_slice(&self.unit);
        }
        self.unit.clear();
        let st = std::mem::take(&mut self.stmt);
        self.after_statement(&st);
        self.decision = Decision::Undecided;
        self.stmt = Stmt {
            first_line: self.line,
            ..Stmt::default()
        };
        Ok(())
    }

    /// Bookkeeping that needs the whole statement: COPY blocks, INSERT rows, settings that change how psql reads.
    fn after_statement(&mut self, st: &Stmt) {
        self.info.statements += 1;
        let first = st.toks.first().map(|t| t.norm.as_str()).unwrap_or("");
        if first == "COPY" && st.is_copy_stdin {
            self.copy_pending = true;
            self.copy_keep = self.decision == Decision::Keep;
            let name = name_after(&st.toks, 1);
            if let Some(n) = &name {
                self.info.rows.entry(n.clone()).or_insert((0, true));
            }
            self.copy_table = name;
        }
        if matches!(first, "INSERT" | "REPLACE") && st.values_rows > 0 {
            let at = st
                .toks
                .iter()
                .position(|t| t.norm == "INTO")
                .map_or(1, |p| p + 1);
            if let Some(n) = name_after(&st.toks, at) {
                self.info.rows.entry(n).or_insert((0, false)).0 += st.values_rows;
            }
        }
        if self.d == Dialect::Pg {
            let names: Vec<String> = st
                .toks
                .iter()
                .map(|t| t.norm.to_ascii_uppercase())
                .collect();
            let at = names
                .iter()
                .position(|n| n == "STANDARD_CONFORMING_STRINGS");
            if first == "SET" && at.is_some_and(|p| p <= 2) {
                let v = st.toks[at.unwrap_or(0) + 1..]
                    .iter()
                    .find(|t| !matches!(t.kind, TokKind::Punct(_)) && t.norm != "TO")
                    .map(|t| t.norm.to_ascii_lowercase());
                self.scs = !matches!(v.as_deref(), Some("off" | "false" | "0"));
            }
            if first == "RESET"
                && names
                    .get(1)
                    .is_some_and(|n| n == "ALL" || n == "STANDARD_CONFORMING_STRINGS")
            {
                self.scs = self.scs_default;
            }
        }
    }

    /// Keep, drop or rewrite the statement in `self.unit`, from its opening tokens.
    fn decide(&mut self, out: &mut Vec<u8>) {
        let toks = std::mem::take(&mut self.stmt.toks);
        let w: Vec<&str> = toks
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .take(12)
            .map(|t| t.norm.as_str())
            .collect();
        let first = w.first().copied().unwrap_or("");
        let second = w.get(1).copied().unwrap_or("");
        let my = self.d == Dialect::My;
        let has = |x: &str| toks.iter().any(|t| t.norm == x);
        let pair = |a: &str, b: &str| w.windows(2).any(|p| p[0] == a && p[1] == b);
        // ---- What it is
        let is_db = (matches!(first, "CREATE" | "ALTER" | "DROP")
            && (second == "DATABASE" || (my && second == "SCHEMA")))
            || (first == "COMMENT" && pair("ON", "DATABASE"));
        let is_role = (matches!(first, "CREATE" | "ALTER" | "DROP")
            && matches!(second, "ROLE" | "USER" | "TABLESPACE" | "GROUP"))
            || (matches!(first, "GRANT" | "REVOKE") && !w.contains(&"ON"))
            || (first == "SET" && second == "PASSWORD")
            || (first == "FLUSH" && second == "PRIVILEGES");
        let superuser = is_role && has("SUPERUSER");
        let is_owner = first == "ALTER" && pair("OWNER", "TO");
        let is_acl = (matches!(first, "GRANT" | "REVOKE") && w.contains(&"ON"))
            || (first == "ALTER" && second == "DEFAULT" && w.get(2) == Some(&"PRIVILEGES"))
            || (first == "SECURITY" && second == "LABEL")
            || (first == "COMMENT" && pair("ON", "EXTENSION"));
        let is_gtid = my && first == "SET" && (has("GTID_PURGED") || has("SQL_LOG_BIN"));
        let setval = first == "SELECT" && has("SETVAL");
        let class = if matches!(first, "INSERT" | "REPLACE" | "COPY" | "LOCK" | "UNLOCK")
            || (first == "SELECT"
                && (setval || has("LOWRITE") || toks.iter().any(|t| t.norm.starts_with("LO_"))))
            || (first == "ALTER" && has("KEYS") && (has("DISABLE") || has("ENABLE")))
        {
            Class::Data
        } else if matches!(first, "SET" | "SELECT" | "BEGIN" | "COMMIT" | "START" | "") {
            Class::Setting
        } else {
            Class::Schema
        };
        // ---- Preview
        if is_db && first == "CREATE" {
            if let Some(n) = name_after_kind(&toks, &["DATABASE", "SCHEMA"]) {
                if !self.info.databases.contains(&n) {
                    self.info.databases.push(n);
                }
            }
        }
        if is_role {
            self.info.roles += 1;
            if superuser {
                self.info.superuser_roles += 1;
            }
        }
        if is_owner {
            if let Some(o) = toks
                .iter()
                .position(|t| t.norm == "OWNER")
                .and_then(|p| toks.get(p + 2))
            {
                let o = if o.kind == TokKind::Word {
                    o.raw.to_ascii_lowercase()
                } else {
                    o.norm.clone()
                };
                if !self.info.owners.contains(&o) && self.info.owners.len() < 50 {
                    self.info.owners.push(o);
                }
            }
        }
        if matches!(first, "GRANT" | "REVOKE") && w.contains(&"ON") {
            self.info.grants += 1;
        }
        if is_gtid {
            self.info.gtid += 1;
        }
        if toks
            .windows(2)
            .any(|p| p[0].norm == "DEFINER" && p[1].kind == TokKind::Punct(b'='))
        {
            self.info.definers += 1;
        }
        let created = (first == "CREATE" && !is_db && !is_role)
            .then(|| object_of(&toks))
            .flatten();
        if let Some((kind, name)) = &created {
            if kind == "extension" && !self.info.extensions.contains(name) {
                self.info.extensions.push(name.clone());
            }
            if self.info.objects.len() < MAX_OBJECTS {
                self.info.objects.push(Obj {
                    kind: kind.clone(),
                    database: self.section.clone(),
                    name: name.clone(),
                });
            } else {
                self.info.objects_more += 1;
            }
        }
        // ---- The decision
        let Some(rw) = self.rw.clone() else {
            self.decision = Decision::Drop;
            self.stmt.toks = toks;
            return;
        };
        let drop = !self.keep_now()
            || is_db
            || (is_role && (!rw.include_roles || superuser))
            || ((is_owner || is_acl) && rw.skip_owner_acl)
            || is_gtid
            || (rw.content == Content::Schema && class == Class::Data)
            || ((rw.content == Content::Data || rw.existing == Existing::Append)
                && class == Class::Schema)
            || (rw.existing == Existing::Append && setval);
        if drop {
            self.decision = Decision::Drop;
            self.unit.clear();
        } else {
            self.decision = Decision::Keep;
            if rw.strip_definer && my {
                let at = self.stmt.code_at.min(self.unit.len());
                let mut u = self.unit[..at].to_vec();
                u.extend(strip_definer(&self.unit[at..]));
                self.unit = u;
            }
            if rw.existing == Existing::Replace {
                if let Some((kind, _)) = &created {
                    self.replace_rewrite(kind, &toks, out);
                }
            }
        }
        self.stmt.toks = toks;
    }

    /// "Drop and replace": a `DROP … IF EXISTS` before each created relation, and `OR REPLACE` / `IF NOT EXISTS`
    /// where the engine has them.
    fn replace_rewrite(&mut self, kind: &str, toks: &[Tok], out: &mut Vec<u8>) {
        let my = self.d == Dialect::My;
        let drop_kind = match (kind, my) {
            ("table", _) => Some("TABLE"),
            ("view", _) => Some("VIEW"),
            ("materialized view", false) => Some("MATERIALIZED VIEW"),
            ("sequence", false) => Some("SEQUENCE"),
            ("type", false) => Some("TYPE"),
            ("domain", false) => Some("DOMAIN"),
            ("procedure", true) => Some("PROCEDURE"),
            ("function", true) => Some("FUNCTION"),
            ("trigger", true) => Some("TRIGGER"),
            ("event", true) => Some("EVENT"),
            _ => None,
        };
        if let (Some(k), Some(name)) = (drop_kind, raw_name_after_kind(toks)) {
            let cascade = if my { "" } else { " CASCADE" };
            let end = String::from_utf8_lossy(&self.delimiter).into_owned();
            // Written before the statement's own unit, so it runs first.
            out.extend_from_slice(format!("DROP {k} IF EXISTS {name}{cascade}{end}\n").as_bytes());
        }
        if !my {
            let at = self.stmt.code_at.min(self.unit.len());
            let insert = match kind {
                "function" | "procedure" | "aggregate" if !has_or_replace(toks) => {
                    find_ci(&self.unit[at..], b"CREATE").map(|p| (at + p + 6, &b" OR REPLACE"[..]))
                }
                "schema" if !toks.iter().any(|t| t.norm == "EXISTS") => {
                    find_ci(&self.unit[at..], b"SCHEMA")
                        .map(|p| (at + p + 6, &b" IF NOT EXISTS"[..]))
                }
                _ => None,
            };
            if let Some((p, text)) = insert {
                self.unit.splice(p..p, text.iter().copied());
            }
        }
    }
}

fn has_or_replace(toks: &[Tok]) -> bool {
    toks.get(1).is_some_and(|t| t.norm == "OR") && toks.get(2).is_some_and(|t| t.norm == "REPLACE")
}

/// Where `word` (any case, whole word, outside quotes) first appears in `unit`.
fn find_ci(unit: &[u8], word: &[u8]) -> Option<usize> {
    let mut q: Option<u8> = None;
    for i in 0..unit.len() {
        let b = unit[i];
        match q {
            Some(c) => {
                if b == c {
                    q = None;
                }
            }
            None if matches!(b, b'\'' | b'"' | b'`') => q = Some(b),
            None => {
                if unit[i..].len() >= word.len()
                    && unit[i..i + word.len()].eq_ignore_ascii_case(word)
                    && (i == 0 || !is_ident(unit[i - 1]))
                    && unit.get(i + word.len()).is_none_or(|c| !is_ident(*c))
                {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// The kinds a CREATE statement can make, as shown in the preview.
const KINDS: &[&str] = &[
    "TABLE",
    "VIEW",
    "FUNCTION",
    "PROCEDURE",
    "TRIGGER",
    "EVENT",
    "SEQUENCE",
    "TYPE",
    "DOMAIN",
    "SCHEMA",
    "EXTENSION",
    "INDEX",
    "AGGREGATE",
];

/// `CREATE … KIND [IF NOT EXISTS] name` → ("kind", "schema.name").
fn object_of(toks: &[Tok]) -> Option<(String, String)> {
    let at = toks
        .iter()
        .position(|t| t.kind == TokKind::Word && KINDS.contains(&t.norm.as_str()))?;
    let mut kind = toks[at].norm.to_ascii_lowercase();
    if kind == "view" && at > 0 && toks[at - 1].norm == "MATERIALIZED" {
        kind = "materialized view".into();
    }
    if kind == "index" {
        return Some((kind, name_after(toks, at + 1).unwrap_or_default()));
    }
    Some((kind, name_after(toks, at + 1)?))
}

fn name_after_kind(toks: &[Tok], kinds: &[&str]) -> Option<String> {
    let at = toks.iter().position(|t| kinds.contains(&t.norm.as_str()))?;
    name_after(toks, at + 1)
}

/// A (possibly qualified) name at `at`, after `IF NOT EXISTS` / `ONLY`; quotes removed, unquoted parts lower case.
fn name_after(toks: &[Tok], mut at: usize) -> Option<String> {
    while toks.get(at).is_some_and(|t| {
        t.kind == TokKind::Word
            && matches!(
                t.norm.as_str(),
                "IF" | "NOT" | "EXISTS" | "ONLY" | "CONCURRENTLY"
            )
    }) {
        at += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    loop {
        let t = toks.get(at)?;
        match t.kind {
            TokKind::Word => parts.push(t.raw.to_ascii_lowercase()),
            TokKind::QIdent => parts.push(t.norm.clone()),
            _ => break,
        }
        if toks.get(at + 1).map(|t| &t.kind) == Some(&TokKind::Punct(b'.')) {
            at += 2;
        } else {
            break;
        }
    }
    (!parts.is_empty()).then(|| parts.join("."))
}

/// A CREATE statement's name as written (quotes kept), for a DROP put in front of it.
fn raw_name_after_kind(toks: &[Tok]) -> Option<String> {
    let mut at = toks
        .iter()
        .position(|t| t.kind == TokKind::Word && KINDS.contains(&t.norm.as_str()))?
        + 1;
    while toks.get(at).is_some_and(|t| {
        t.kind == TokKind::Word && matches!(t.norm.as_str(), "IF" | "NOT" | "EXISTS")
    }) {
        at += 1;
    }
    let mut s = String::new();
    loop {
        let t = toks.get(at)?;
        match t.kind {
            TokKind::Word | TokKind::QIdent => s.push_str(&t.raw),
            _ => break,
        }
        if toks.get(at + 1).map(|t| &t.kind) == Some(&TokKind::Punct(b'.')) {
            s.push('.');
            at += 2;
        } else {
            break;
        }
    }
    (!s.is_empty()).then_some(s)
}

/// `\connect -reuse-previous=on "dbname='shop'"` or `\connect shop` → "shop".
fn connect_target(args: &str) -> Option<String> {
    let a = args
        .split_whitespace()
        .find(|a| !a.starts_with('-'))?
        .trim_matches('"');
    if let Some(rest) = a.strip_prefix("dbname=") {
        return Some(rest.trim_matches('\'').replace("''", "'"));
    }
    Some(a.trim_matches('\'').to_string())
}

/// MySQL's long-form client commands (`mysql --help`'s list). Only `delimiter` and `use` pass.
const MY_COMMANDS: &[&str] = &[
    "clear",
    "connect",
    "delimiter",
    "edit",
    "ego",
    "exit",
    "go",
    "help",
    "nopager",
    "notee",
    "nowarning",
    "pager",
    "print",
    "prompt",
    "quit",
    "rehash",
    "source",
    "status",
    "system",
    "tee",
    "use",
    "charset",
    "warnings",
    "resetconnection",
    "query_attributes",
    "ssl_session_data_print",
];

fn pg_effect(name: &str) -> &'static str {
    match name {
        "!" | "setenv" | "gexec" => ", which runs a program on the host",
        "o" | "out" | "g" | "w" | "write" | "copy" | "gx" => {
            ", which can write a file or run a program on the host"
        }
        "i" | "include" | "ir" | "include_relative" | "e" | "edit" | "ef" | "ev" => {
            ", which reads a file on the host"
        }
        _ => "",
    }
}

fn my_effect(c: u8) -> &'static str {
    match c {
        b'!' => ", which runs a program on the host",
        b'.' => ", which reads a file on the host",
        b'T' | b'P' => ", which writes a file or runs a program on the host",
        _ => "",
    }
}

fn my_named_effect(name: &str) -> &'static str {
    match name {
        "system" => ", which runs a program on the host",
        "source" => ", which reads a file on the host",
        "tee" | "pager" => ", which writes a file or runs a program on the host",
        "connect" => ", which connects somewhere else",
        _ => "",
    }
}

/// Removes `DEFINER=user@host` (outside quotes) so the restoring user owns the object.
pub fn strip_definer(unit: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unit.len());
    let mut i = 0;
    let mut q: Option<u8> = None;
    // One user or host part: quoted, or a bare word.
    let spec = |j: usize| -> usize {
        match unit.get(j) {
            Some(&qc @ (b'`' | b'\'' | b'"')) => {
                let mut k = j + 1;
                while k < unit.len() {
                    if unit[k] == qc {
                        if unit.get(k + 1) == Some(&qc) {
                            k += 2;
                            continue;
                        }
                        return k + 1;
                    }
                    k += 1;
                }
                k
            }
            _ => {
                let mut k = j;
                while unit.get(k).is_some_and(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'.' | b'-' | b'%')
                }) {
                    k += 1;
                }
                // CURRENT_USER()
                if unit.get(k) == Some(&b'(') && unit.get(k + 1) == Some(&b')') {
                    k += 2;
                }
                k
            }
        }
    };
    while i < unit.len() {
        let b = unit[i];
        if let Some(c) = q {
            out.push(b);
            if b == b'\\' && c != b'`' {
                if let Some(n) = unit.get(i + 1) {
                    out.push(*n);
                }
                i += 2;
                continue;
            }
            if b == c {
                q = None;
            }
            i += 1;
            continue;
        }
        if matches!(b, b'\'' | b'"' | b'`') {
            q = Some(b);
            out.push(b);
            i += 1;
            continue;
        }
        if unit.len() - i >= 7
            && unit[i..i + 7].eq_ignore_ascii_case(b"DEFINER")
            && (i == 0 || !is_ident(unit[i - 1]))
        {
            let mut j = i + 7;
            while unit.get(j) == Some(&b' ') {
                j += 1;
            }
            if unit.get(j) == Some(&b'=') {
                j += 1;
                while unit.get(j) == Some(&b' ') {
                    j += 1;
                }
                let mut k = spec(j);
                if unit.get(k) == Some(&b'@') {
                    k = spec(k + 1);
                }
                if k > j {
                    i = k;
                    continue;
                }
            }
        }
        out.push(b);
        i += 1;
    }
    out
}

/// The source details from the first lines' comments.
pub fn header_details(info: &mut PlainInfo, header: &str) {
    for l in header.lines().take(40) {
        let l = l.trim_start_matches("--").trim();
        let after = |p: &str| l.strip_prefix(p).map(|s| s.trim().to_string());
        if let Some(v) = after("Dumped by pg_dump version") {
            info.made_by = Some(format!("pg_dump {v}"));
        } else if let Some(v) = after("Dumped from database version") {
            info.server_version = Some(format!("PostgreSQL {v}"));
        } else if l.starts_with("PostgreSQL database cluster dump") {
            info.cluster = true;
            info.made_by.get_or_insert_with(|| "pg_dumpall".to_string());
        } else if l.starts_with("MySQL dump") || l.starts_with("MariaDB dump") {
            info.made_by = Some(l.split_whitespace().collect::<Vec<_>>().join(" "));
        } else if let Some(v) = after("Server version") {
            info.server_version = Some(v);
        } else if let Some((_, db)) = l.split_once("Database:") {
            let db = db.trim().trim_matches('`');
            if !db.is_empty() {
                info.source_db = Some(db.to_string());
            }
        }
    }
}

impl Scanner {
    /// What is being sent now: the table whose rows are streaming, or the last object created.
    pub fn current(&self) -> Option<String> {
        if self.lex == Lex::CopyData {
            return self.copy_table.clone();
        }
        self.info
            .objects
            .last()
            .map(|o| format!("{} {}", o.kind, o.name))
    }
}

/// Scans a whole text at once (tests and pg archive TOC entries).
pub fn scan_all(
    d: Dialect,
    text: &[u8],
    rw: Option<Rewrite>,
    client_checks: bool,
) -> Result<(PlainInfo, Vec<u8>), Refusal> {
    let mut s = Scanner::new(d, rw, true);
    s.client_checks = client_checks;
    let mut out = Vec::new();
    s.feed(text, &mut out)?;
    s.finish(&mut out)?;
    Ok((s.info, out))
}

#[cfg(test)]
#[path = "dumpscan_tests.rs"]
mod tests;
