//! The MongoDB shell (S47; F30): `db.orders.find({...}).sort({...}).limit(5)` and the familiar helpers, parsed in
//! Rust. There is no JavaScript engine: one command per line, arguments are relaxed JSON (unquoted keys, single
//! quotes, trailing commas, comments) plus the shell's value helpers. Each command becomes a database command and a
//! class for the gate, so the shell has no way around it.

use bson::{doc, Bson, Document};
use serde::Serialize;
use specta::Type;

use super::conn::MongoConn;
use super::docs::{names, relaxed};
use crate::error::{AppError, AppResult};
use crate::sql::classify::Class;

pub const NOT_JS: &str = "The shell runs single commands, not JavaScript. Use the Aggregations view or a saved pipeline for multi-step work.";

fn err(msg: impl Into<String>) -> AppError {
    AppError::validation(msg)
}

// ---- Values: relaxed JSON and the shell's helpers

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match self.s[self.i + 2..].windows(2).position(|w| w == b"*/") {
                    Some(p) => self.i += p + 4,
                    None => self.i = self.s.len(),
                }
            } else {
                break;
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: u8) -> AppResult<()> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.bad(&format!("expected {}", c as char)))
        }
    }
    fn bad(&self, what: &str) -> AppError {
        let rest = String::from_utf8_lossy(&self.s[self.i.min(self.s.len())..]);
        if rest.trim().is_empty() {
            return err(format!("The command ends too early ({what})."));
        }
        let near: String = rest.chars().take(24).collect();
        err(format!("Can't read this near \"{near}\" ({what})."))
    }
    fn ident(&mut self) -> Option<String> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_' || self.s[self.i] == b'$') {
            self.i += 1;
        }
        (self.i > start).then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
    fn string(&mut self) -> AppResult<String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            if c == q {
                return String::from_utf8(out).map_err(|_| err("That text isn't valid UTF-8."));
            }
            if c == b'\\' && self.i < self.s.len() {
                let e = self.s[self.i];
                self.i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'u' => {
                        let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4).unwrap_or(b"")).unwrap_or("");
                        let ch = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).ok_or_else(|| self.bad("a \\u escape needs four hex digits"))?;
                        self.i += 4;
                        out.extend_from_slice(ch.to_string().as_bytes());
                    }
                    other => out.push(other),
                }
            } else {
                out.push(c);
            }
        }
        Err(self.bad("the text isn't closed"))
    }
    fn number(&mut self) -> AppResult<Bson> {
        let start = self.i;
        if self.s[self.i] == b'-' || self.s[self.i] == b'+' {
            self.i += 1;
        }
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'e' | b'E') || (matches!(self.s[self.i], b'-' | b'+') && matches!(self.s[self.i - 1], b'e' | b'E'))) {
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        if let Ok(n) = t.parse::<i64>() {
            return Ok(i32::try_from(n).map_or(Bson::Int64(n), Bson::Int32));
        }
        t.parse::<f64>().map(Bson::Double).map_err(|_| self.bad("not a number"))
    }

    fn value(&mut self) -> AppResult<Bson> {
        match self.peek().ok_or_else(|| self.bad("a value is missing"))? {
            b'{' => {
                self.i += 1;
                let mut d = Document::new();
                loop {
                    if self.eat(b'}') {
                        break;
                    }
                    let key = match self.peek() {
                        Some(b'"') | Some(b'\'') => self.string()?,
                        _ => self.ident().ok_or_else(|| self.bad("a field name"))?,
                    };
                    self.expect(b':')?;
                    d.insert(key, self.value()?);
                    if !self.eat(b',') {
                        self.expect(b'}')?;
                        break;
                    }
                }
                Ok(Bson::Document(d))
            }
            b'[' => {
                self.i += 1;
                let mut a = vec![];
                loop {
                    if self.eat(b']') {
                        break;
                    }
                    a.push(self.value()?);
                    if !self.eat(b',') {
                        self.expect(b']')?;
                        break;
                    }
                }
                Ok(Bson::Array(a))
            }
            b'"' | b'\'' => Ok(Bson::String(self.string()?)),
            b'/' => self.regex(),
            b'-' | b'+' | b'0'..=b'9' => self.number(),
            _ => {
                let name = self.ident().ok_or_else(|| self.bad("not a value"))?;
                self.word(&name)
            }
        }
    }

    fn regex(&mut self) -> AppResult<Bson> {
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != b'/' {
            if self.s[self.i] == b'\\' {
                self.i += 1;
            }
            self.i += 1;
        }
        if self.i >= self.s.len() {
            return Err(self.bad("the pattern isn't closed"));
        }
        let pattern = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.i += 1;
        let fs = self.i;
        while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let options = String::from_utf8_lossy(&self.s[fs..self.i]).into_owned();
        // A regular expression's text can't hold a NUL byte (BSON stores it as a C string).
        let cs = |t: String| bson::raw::CString::try_from(t).map_err(|_| err("A regular expression can't contain a NUL character."));
        Ok(Bson::RegularExpression(bson::Regex { pattern: cs(pattern)?, options: cs(options)? }))
    }

    /// `true`, `null`, `ObjectId("…")`, `new Date()`, `NumberLong(5)` and the rest of the shell's helpers.
    fn word(&mut self, name: &str) -> AppResult<Bson> {
        match name {
            "true" => return Ok(Bson::Boolean(true)),
            "false" => return Ok(Bson::Boolean(false)),
            "null" => return Ok(Bson::Null),
            "MinKey" => return Ok(Bson::MinKey),
            "MaxKey" => return Ok(Bson::MaxKey),
            "Infinity" => return Ok(Bson::Double(f64::INFINITY)),
            "NaN" => return Ok(Bson::Double(f64::NAN)),
            "new" => {
                let inner = self.ident().ok_or_else(|| self.bad("a name after new"))?;
                if inner != "Date" {
                    return Err(err(NOT_JS));
                }
                return self.word("Date");
            }
            _ => {}
        }
        if !self.eat(b'(') {
            return Err(err(NOT_JS));
        }
        let mut args = vec![];
        loop {
            if self.eat(b')') {
                break;
            }
            args.push(self.value()?);
            if !self.eat(b',') {
                self.expect(b')')?;
                break;
            }
        }
        let text = |a: &[Bson]| match a.first() {
            Some(Bson::String(s)) => Some(s.clone()),
            Some(Bson::Int32(i)) => Some(i.to_string()),
            Some(Bson::Int64(i)) => Some(i.to_string()),
            _ => None,
        };
        let wrong = |h: &str| err(format!("{h}() got an argument it can't use."));
        Ok(match name {
            "ObjectId" => match text(&args) {
                None => Bson::ObjectId(bson::oid::ObjectId::new()),
                Some(h) => Bson::ObjectId(bson::oid::ObjectId::parse_str(&h).map_err(|_| err("ObjectId() needs 24 hex digits."))?),
            },
            "ISODate" | "Date" => match args.first() {
                None => Bson::DateTime(bson::DateTime::now()),
                Some(Bson::String(s)) => Bson::DateTime(bson::DateTime::parse_rfc3339_str(s).or_else(|_| bson::DateTime::parse_rfc3339_str(format!("{s}Z"))).or_else(|_| bson::DateTime::parse_rfc3339_str(format!("{s}T00:00:00Z"))).map_err(|_| err(format!("\"{s}\" isn't a date like 2025-03-01T10:00:00Z.")))?),
                Some(Bson::Int32(i)) => Bson::DateTime(bson::DateTime::from_millis(i64::from(*i))),
                Some(Bson::Int64(i)) => Bson::DateTime(bson::DateTime::from_millis(*i)),
                Some(Bson::Double(d)) => Bson::DateTime(bson::DateTime::from_millis(*d as i64)),
                _ => return Err(wrong(name)),
            },
            "NumberLong" => Bson::Int64(text(&args).and_then(|t| t.parse().ok()).ok_or_else(|| wrong(name))?),
            "NumberInt" => Bson::Int32(text(&args).and_then(|t| t.parse().ok()).ok_or_else(|| wrong(name))?),
            "NumberDecimal" => Bson::Decimal128(text(&args).and_then(|t| t.parse::<bson::Decimal128>().ok()).ok_or_else(|| err("NumberDecimal() needs a decimal number in quotes, like \"1.10\"."))?),
            "UUID" => {
                let u = bson::Uuid::parse_str(text(&args).ok_or_else(|| wrong(name))?).map_err(|_| err("UUID() needs a value like 123e4567-e89b-12d3-a456-426614174000."))?;
                Bson::Binary(bson::Binary::from_uuid(u))
            }
            "Timestamp" => match (args.first(), args.get(1)) {
                (Some(Bson::Int32(t)), Some(Bson::Int32(i))) => Bson::Timestamp(bson::Timestamp { time: *t as u32, increment: *i as u32 }),
                _ => return Err(err("Timestamp() needs two whole numbers: seconds and an increment.")),
            },
            "BinData" => match (args.first(), args.get(1)) {
                (Some(Bson::Int32(sub)), Some(Bson::String(b64))) => Bson::Binary(bson::Binary::from_base64(b64, Some(bson::spec::BinarySubtype::from(*sub as u8))).map_err(|_| err("BinData() needs base64 text."))?),
                _ => return Err(err("BinData() needs a subtype number and base64 text.")),
            },
            _ => return Err(err(NOT_JS)),
        })
    }
}

// ---- Statements

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Use(String),
    Show(String),
    /// `db.<coll>.<method>(args)` with chained cursor methods.
    Coll { coll: String, method: String, args: Vec<Bson>, chain: Vec<(String, Vec<Bson>)> },
    /// `db.<method>(args)`.
    Db { method: String, args: Vec<Bson> },
}

fn args_of(p: &mut P) -> AppResult<Vec<Bson>> {
    p.expect(b'(')?;
    let mut a = vec![];
    loop {
        if p.eat(b')') {
            return Ok(a);
        }
        a.push(p.value()?);
        if !p.eat(b',') {
            p.expect(b')')?;
            return Ok(a);
        }
    }
}

pub fn parse(line: &str) -> AppResult<Stmt> {
    let t = line.trim().trim_end_matches(';').trim();
    if t.is_empty() {
        return Err(err("Type a command, like db.orders.find({}) or show collections."));
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("use ") {
        let name = rest.trim();
        let name = t[t.len() - name.len()..].trim().trim_matches(|c| c == '"' || c == '\'');
        if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\' || c == '.' || c == '$' || c == '\0') {
            return Err(err("Name a database after use, like use shop."));
        }
        return Ok(Stmt::Use(name.to_string()));
    }
    if let Some(rest) = lower.strip_prefix("show ") {
        return match rest.trim() {
            "dbs" | "databases" => Ok(Stmt::Show("dbs".into())),
            "collections" | "tables" => Ok(Stmt::Show("collections".into())),
            "users" => Ok(Stmt::Show("users".into())),
            other => Err(err(format!("show {other} isn't supported. Try show dbs, show collections or show users."))),
        };
    }
    if lower == "it" {
        return Err(err("`it` isn't supported. Add .skip(n) to read the next documents."));
    }
    if !t.starts_with("db.") && !t.starts_with("db[") {
        return Err(err(if t.contains(['=', '{']) || ["var ", "let ", "const ", "for ", "for(", "function", "while"].iter().any(|k| lower.starts_with(k)) { NOT_JS } else { "Commands start with db. , use or show, like db.orders.find({})." }));
    }
    let mut p = P { s: t.as_bytes(), i: 2 };
    let first = if p.eat(b'.') { p.ident().ok_or_else(|| p.bad("a name after db."))? } else { return Err(err("Use db.getCollection(\"name\") for unusual collection names.")) };
    let stmt = if first == "getCollection" {
        let a = args_of(&mut p)?;
        let Some(Bson::String(coll)) = a.first() else { return Err(err("getCollection() needs the collection's name in quotes.")) };
        p.expect(b'.')?;
        let method = p.ident().ok_or_else(|| p.bad("a method name"))?;
        Stmt::Coll { coll: coll.clone(), method, args: args_of(&mut p)?, chain: vec![] }
    } else if p.peek() == Some(b'(') {
        Stmt::Db { method: first, args: args_of(&mut p)? }
    } else {
        p.expect(b'.')?;
        let method = p.ident().ok_or_else(|| p.bad("a method name"))?;
        Stmt::Coll { coll: first, method, args: args_of(&mut p)?, chain: vec![] }
    };
    let Stmt::Coll { coll, method, args, mut chain } = stmt else {
        return finish(p, stmt);
    };
    while p.eat(b'.') {
        let m = p.ident().ok_or_else(|| p.bad("a method name"))?;
        chain.push((m, args_of(&mut p)?));
    }
    finish(p, Stmt::Coll { coll, method, args, chain })
}

fn finish(mut p: P, s: Stmt) -> AppResult<Stmt> {
    if p.peek().is_some() {
        return Err(err(NOT_JS));
    }
    Ok(s)
}

// ---- From a statement to a command and a class

/// What comes back, and how it is shown.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// `cursor.firstBatch` of documents.
    Cursor,
    /// The reply document without `ok`.
    Doc,
    /// One field of the reply (`n`, `values`).
    Field(&'static str),
    /// Names from a `listCollections`/`listDatabases` reply.
    Names(&'static str),
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub db: String,
    pub class: Class,
    pub summary: String,
    pub cmd: Document,
    pub shape: Shape,
    /// For writes that change what a filter matches: the collection and filter, to count and sample first.
    pub matches: Option<(String, Document)>,
}

fn doc_arg(a: &[Bson], i: usize, what: &str) -> AppResult<Document> {
    match a.get(i) {
        None => Ok(Document::new()),
        Some(Bson::Document(d)) => Ok(d.clone()),
        Some(_) => Err(err(format!("{what} must be a document like {{ \"a\": 1 }}."))),
    }
}

fn same_pipeline_writes(p: &[Bson]) -> bool {
    p.iter().any(|s| s.as_document().is_some_and(|d| d.keys().any(|k| k.eq_ignore_ascii_case("$out") || k.eq_ignore_ascii_case("$merge"))))
}

/// The class of a raw `runCommand`, by the command's name: anything not known to be harmless is `Dangerous`.
pub fn classify_command(cmd: &Document) -> Class {
    let name = cmd.keys().next().map(|k| k.to_ascii_lowercase()).unwrap_or_default();
    match name.as_str() {
        "find" | "count" | "distinct" | "listcollections" | "listindexes" | "listdatabases" | "collstats" | "dbstats" | "ping" | "hello" | "ismaster" | "buildinfo" | "serverstatus" | "currentop" | "connectionstatus" | "usersinfo" | "rolesinfo" | "getparameter" | "validate" | "explain" | "whatsmyuri" | "hostinfo" => Class::Read,
        "aggregate" => cmd.get_array("pipeline").map_or(Class::Read, |p| if same_pipeline_writes(p) { Class::Write } else { Class::Read }),
        "insert" | "update" | "delete" | "findandmodify" => Class::Write,
        "create" | "createindexes" | "collmod" | "renamecollection" | "converttocapped" => Class::Schema,
        _ => Class::Dangerous,
    }
}

fn find_cmd(coll: &str, args: &[Bson], chain: &[(String, Vec<Bson>)], limit_one: bool) -> AppResult<(Document, bool)> {
    let mut cmd = doc! { "find": coll, "filter": doc_arg(args, 0, "The filter")? };
    let proj = doc_arg(args, 1, "The projection")?;
    if !proj.is_empty() {
        cmd.insert("projection", proj);
    }
    let mut explain = false;
    let mut count = false;
    let mut limit = if limit_one { 1 } else { 20 };
    for (m, a) in chain {
        let first = a.first();
        let n = || match first {
            Some(Bson::Int32(i)) => Ok(i64::from(*i)),
            Some(Bson::Int64(i)) => Ok(*i),
            Some(Bson::Double(d)) => Ok(*d as i64),
            _ => Err(err(format!(".{m}() needs a number."))),
        };
        match m.as_str() {
            "sort" => cmd.insert("sort", doc_arg(a, 0, "The sort")?),
            "skip" => cmd.insert("skip", n()?.max(0)),
            "limit" => {
                limit = n()?.clamp(0, 1000);
                None
            }
            "project" => cmd.insert("projection", doc_arg(a, 0, "The projection")?),
            "hint" => cmd.insert("hint", first.cloned().ok_or_else(|| err(".hint() needs an index name or keys."))?),
            "collation" => cmd.insert("collation", doc_arg(a, 0, "The collation")?),
            "maxTimeMS" => cmd.insert("maxTimeMS", n()?),
            "batchSize" | "toArray" | "pretty" | "noCursorTimeout" | "allowDiskUse" => None,
            "explain" => {
                explain = true;
                None
            }
            "count" | "itcount" => {
                count = true;
                None
            }
            other => return Err(err(format!(".{other}() isn't supported on find. Supported: sort, skip, limit, project, hint, collation, maxTimeMS, explain, count."))),
        };
    }
    if count {
        let mut c = doc! { "count": coll, "query": cmd.get_document("filter").cloned().unwrap_or_default() };
        for k in ["skip", "hint", "collation", "maxTimeMS"] {
            if let Some(v) = cmd.get(k) {
                c.insert(k, v.clone());
            }
        }
        if limit != 20 {
            c.insert("limit", limit);
        }
        return Ok((c, true));
    }
    cmd.insert("limit", limit.max(0));
    if limit == 0 {
        cmd.remove("limit");
    }
    cmd.insert("batchSize", 101);
    if explain {
        cmd = doc! { "explain": cmd, "verbosity": "queryPlanner" };
    }
    Ok((cmd, false))
}

/// A statement as the command to run, its class and what it will say it does.
pub fn plan(stmt: &Stmt, db: &str) -> AppResult<Plan> {
    let p = |class, summary: String, cmd, shape| Ok(Plan { db: db.into(), class, summary, cmd, shape, matches: None });
    match stmt {
        Stmt::Use(_) => Err(err("use changes the database; it has no command to run.")),
        Stmt::Show(what) => match what.as_str() {
            "dbs" => Ok(Plan { db: "admin".into(), class: Class::Read, summary: "List databases.".into(), cmd: doc! { "listDatabases": 1, "nameOnly": true }, shape: Shape::Names("databases"), matches: None }),
            "collections" => p(Class::Read, format!("List the collections of {db}."), doc! { "listCollections": 1, "nameOnly": true }, Shape::Names("cursor")),
            _ => p(Class::Read, format!("List the users of {db}."), doc! { "usersInfo": 1 }, Shape::Doc),
        },
        Stmt::Db { method, args } => match method.as_str() {
            "runCommand" | "adminCommand" => {
                let cmd = doc_arg(args, 0, "The command")?;
                if cmd.is_empty() {
                    return Err(err("runCommand() needs a command document, like { ping: 1 }."));
                }
                let (target, class) = (if method == "adminCommand" { "admin" } else { db }, classify_command(&cmd));
                Ok(Plan { db: target.into(), class, summary: format!("Run the command {} on {target}.", cmd.keys().next().map_or("?", String::as_str)), cmd, shape: Shape::Doc, matches: None })
            }
            "getCollectionNames" => p(Class::Read, format!("List the collections of {db}."), doc! { "listCollections": 1, "nameOnly": true }, Shape::Names("cursor")),
            "stats" => p(Class::Read, format!("Statistics of {db}."), doc! { "dbStats": 1 }, Shape::Doc),
            "currentOp" => Ok(Plan { db: "admin".into(), class: Class::Read, summary: "Running operations.".into(), cmd: doc! { "currentOp": 1 }, shape: Shape::Doc, matches: None }),
            "killOp" => {
                let id = args.first().cloned().ok_or_else(|| err("killOp() needs an operation id."))?;
                Ok(Plan { db: "admin".into(), class: Class::Dangerous, summary: "Stop a running operation.".into(), cmd: doc! { "killOp": 1, "op": id }, shape: Shape::Doc, matches: None })
            }
            "dropDatabase" => p(Class::Dangerous, format!("Drop the whole database {db}. This can't be undone."), doc! { "dropDatabase": 1 }, Shape::Doc),
            "createCollection" => {
                let Some(Bson::String(name)) = args.first() else { return Err(err("createCollection() needs a name in quotes.")) };
                let mut cmd = doc! { "create": name.clone() };
                for (k, v) in doc_arg(args, 1, "The options")? {
                    cmd.insert(k, v);
                }
                p(Class::Schema, format!("Create the collection {name} in {db}."), cmd, Shape::Doc)
            }
            "createView" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Bson::String(n)), Some(Bson::String(src)), Some(Bson::Array(pipe))) if !same_pipeline_writes(pipe) => p(Class::Schema, format!("Create the view {n} on {src}."), doc! { "create": n.clone(), "viewOn": src.clone(), "pipeline": pipe.clone() }, Shape::Doc),
                _ => Err(err("createView() needs a name, a source collection and a pipeline without $out or $merge.")),
            },
            "version" => p(Class::Read, "Server version.".into(), doc! { "buildInfo": 1 }, Shape::Field("version")),
            "hello" | "isMaster" => p(Class::Read, "Server role.".into(), doc! { "hello": 1 }, Shape::Doc),
            "serverStatus" => p(Class::Read, "Server status.".into(), doc! { "serverStatus": 1 }, Shape::Doc),
            "getName" => Err(err("db.getName() isn't supported. The prompt shows the current database.")),
            other => Err(err(format!("db.{other}() isn't supported. Use db.runCommand({{ ... }}) for other commands."))),
        },
        Stmt::Coll { coll, method, args, chain } => {
            let ns = format!("{db}.{coll}");
            let filter = || doc_arg(args, 0, "The filter");
            let with_matches = |mut pl: Plan, f: Document| {
                pl.matches = Some((coll.clone(), f));
                pl
            };
            match method.as_str() {
                "find" | "findOne" => {
                    let (cmd, counted) = find_cmd(coll, args, chain, method == "findOne")?;
                    if counted {
                        return p(Class::Read, format!("Count the matching documents of {ns}."), cmd, Shape::Field("n"));
                    }
                    let shape = if cmd.contains_key("explain") { Shape::Doc } else { Shape::Cursor };
                    p(Class::Read, format!("Find documents in {ns}."), cmd, shape)
                }
                "countDocuments" => p(Class::Read, format!("Count the matching documents of {ns}."), doc! { "count": coll.clone(), "query": filter()? }, Shape::Field("n")),
                "estimatedDocumentCount" => p(Class::Read, format!("Count the documents of {ns} from its metadata."), doc! { "count": coll.clone() }, Shape::Field("n")),
                "distinct" => {
                    let Some(Bson::String(key)) = args.first() else { return Err(err("distinct() needs a field name in quotes.")) };
                    p(Class::Read, format!("Distinct values of {key} in {ns}."), doc! { "distinct": coll.clone(), "key": key.clone(), "query": doc_arg(args, 1, "The filter")? }, Shape::Field("values"))
                }
                "aggregate" => {
                    let Some(Bson::Array(pipe)) = args.first() else { return Err(err("aggregate() needs a pipeline: a list of stages.")) };
                    let class = if same_pipeline_writes(pipe) { Class::Write } else { Class::Read };
                    p(class, format!("Run an aggregation on {ns}{}.", if class == Class::Write { " that writes its output" } else { "" }), doc! { "aggregate": coll.clone(), "pipeline": pipe.clone(), "cursor": { "batchSize": 101 } }, Shape::Cursor)
                }
                "insertOne" => {
                    let d = doc_arg(args, 0, "The document")?;
                    p(Class::Write, format!("Insert 1 document into {ns}."), doc! { "insert": coll.clone(), "documents": [d] }, Shape::Doc)
                }
                "insertMany" => {
                    let Some(Bson::Array(list)) = args.first() else { return Err(err("insertMany() needs a list of documents.")) };
                    p(Class::Write, format!("Insert {} document{} into {ns}.", list.len(), if list.len() == 1 { "" } else { "s" }), doc! { "insert": coll.clone(), "documents": list.clone() }, Shape::Doc)
                }
                "updateOne" | "updateMany" | "replaceOne" => {
                    let (f, u) = (filter()?, args.get(1).cloned().ok_or_else(|| err(format!("{method}() needs a filter and an update.")))?);
                    let many = method == "updateMany";
                    let o = doc_arg(args, 2, "The options")?;
                    let mut spec = doc! { "q": f.clone(), "u": u, "multi": many };
                    if o.get_bool("upsert").unwrap_or(false) {
                        spec.insert("upsert", true);
                    }
                    let class = if many && f.is_empty() { Class::Dangerous } else { Class::Write };
                    Ok(with_matches(Plan { db: db.into(), class, summary: format!("{} in {ns}{}.", match method.as_str() { "updateOne" => "Update 1 matching document", "updateMany" => "Update every matching document", _ => "Replace 1 matching document" }, if many && f.is_empty() { ": the filter is empty, so this changes EVERY document" } else { "" }), cmd: doc! { "update": coll.clone(), "updates": [spec] }, shape: Shape::Doc, matches: None }, f))
                }
                "deleteOne" | "deleteMany" => {
                    let f = filter()?;
                    let many = method == "deleteMany";
                    let class = if many && f.is_empty() { Class::Dangerous } else { Class::Write };
                    Ok(with_matches(Plan { db: db.into(), class, summary: format!("{} from {ns}{}. This can't be undone.", if many { "Delete every matching document" } else { "Delete 1 matching document" }, if many && f.is_empty() { ": the filter is empty, so this deletes EVERY document" } else { "" }), cmd: doc! { "delete": coll.clone(), "deletes": [{ "q": f.clone(), "limit": i32::from(!many) }] }, shape: Shape::Doc, matches: None }, f))
                }
                "findOneAndUpdate" | "findOneAndReplace" | "findOneAndDelete" => {
                    let f = filter()?;
                    let del = method == "findOneAndDelete";
                    let o = doc_arg(args, if del { 1 } else { 2 }, "The options")?;
                    let mut cmd = doc! { "findAndModify": coll.clone(), "query": f.clone() };
                    if del {
                        cmd.insert("remove", true);
                    } else {
                        cmd.insert("update", args.get(1).cloned().ok_or_else(|| err(format!("{method}() needs a filter and an update.")))?);
                    }
                    if let Some(s) = o.get("sort") {
                        cmd.insert("sort", s.clone());
                    }
                    if o.get_bool("upsert").unwrap_or(false) {
                        cmd.insert("upsert", true);
                    }
                    if o.get_str("returnDocument").is_ok_and(|v| v == "after") {
                        cmd.insert("new", true);
                    }
                    Ok(with_matches(Plan { db: db.into(), class: Class::Write, summary: format!("{} in {ns}.", if del { "Delete 1 matching document" } else { "Change 1 matching document" }), cmd, shape: Shape::Doc, matches: None }, f))
                }
                "createIndex" | "createIndexes" => {
                    let specs: Vec<Bson> = if method == "createIndex" {
                        let k = doc_arg(args, 0, "The keys")?;
                        if k.is_empty() {
                            return Err(err("createIndex() needs keys, like { \"status\": 1 }."));
                        }
                        let mut s = doc! { "key": k.clone() };
                        for (a, b) in doc_arg(args, 1, "The options")? {
                            s.insert(a, b);
                        }
                        if !s.contains_key("name") {
                            s.insert("name", k.iter().map(|(f, d)| format!("{f}_{}", match d { Bson::Int32(i) => i.to_string(), Bson::Int64(i) => i.to_string(), Bson::Double(x) => (*x as i64).to_string(), Bson::String(t) => t.clone(), _ => "x".into() })).collect::<Vec<_>>().join("_"));
                        }
                        vec![Bson::Document(s)]
                    } else {
                        match args.first() {
                            Some(Bson::Array(a)) => a.clone(),
                            _ => return Err(err("createIndexes() needs a list of index specs.")),
                        }
                    };
                    p(Class::Schema, format!("Create {} index{} on {ns}.", specs.len(), if specs.len() == 1 { "" } else { "es" }), doc! { "createIndexes": coll.clone(), "indexes": specs }, Shape::Doc)
                }
                "getIndexes" => p(Class::Read, format!("List the indexes of {ns}."), doc! { "listIndexes": coll.clone() }, Shape::Cursor),
                "dropIndex" | "dropIndexes" => {
                    let which = args.first().cloned().unwrap_or(Bson::String("*".into()));
                    if matches!(&which, Bson::String(s) if s == "_id_") {
                        return Err(err("The _id index can't be dropped."));
                    }
                    let all = matches!(&which, Bson::String(s) if s == "*");
                    p(Class::Dangerous, if all { format!("Drop EVERY index of {ns} except _id.") } else { format!("Drop an index of {ns}.") }, doc! { "dropIndexes": coll.clone(), "index": which }, Shape::Doc)
                }
                "drop" => p(Class::Dangerous, format!("Drop the collection {ns} with all its documents. This can't be undone."), doc! { "drop": coll.clone() }, Shape::Doc),
                "stats" => p(Class::Read, format!("Statistics of {ns}."), doc! { "collStats": coll.clone() }, Shape::Doc),
                "renameCollection" => {
                    let Some(Bson::String(to)) = args.first() else { return Err(err("renameCollection() needs the new name in quotes.")) };
                    Ok(Plan { db: "admin".into(), class: Class::Schema, summary: format!("Rename {ns} to {db}.{to}."), cmd: doc! { "renameCollection": ns.clone(), "to": format!("{db}.{to}") }, shape: Shape::Doc, matches: None })
                }
                other => Err(err(format!("{other}() isn't supported in the shell yet. Supported: find, findOne, countDocuments, estimatedDocumentCount, distinct, aggregate, insertOne, insertMany, updateOne, updateMany, replaceOne, deleteOne, deleteMany, findOneAnd*, createIndex, getIndexes, dropIndex, drop, stats, renameCollection."))),
            }
        }
    }
}

/// What the shell prints: one block of text per document or value.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ShellReply {
    pub blocks: Vec<String>,
    pub note: Option<String>,
}

fn show(b: &Bson) -> String {
    match b {
        Bson::String(s) => s.clone(),
        Bson::Document(d) => relaxed(d),
        other => serde_json::to_string_pretty(&other.clone().into_relaxed_extjson()).unwrap_or_default(),
    }
}

/// Formats a reply the way `shape` says. A document reply loses its `ok`.
pub fn reply_of(shape: &Shape, mut r: Document) -> ShellReply {
    match shape {
        Shape::Cursor => {
            let docs = r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default();
            let more = docs.len() > 100;
            ShellReply { blocks: docs.iter().take(100).map(relaxed).collect(), note: more.then(|| "Showing the first 100 documents. Add .limit(n) or .skip(n) to see others.".into()) }
        }
        Shape::Field(k) => ShellReply { blocks: vec![r.get(k).map(show).unwrap_or_else(|| "null".into())], note: None },
        Shape::Names(k) => {
            let list: Vec<Document> = if *k == "cursor" { r.get_document("cursor").map(|c| names(c, "firstBatch")).unwrap_or_default() } else { names(&r, k) };
            let mut n: Vec<String> = list.iter().filter_map(|d| d.get_str("name").ok().map(str::to_string)).collect();
            n.sort();
            ShellReply { blocks: n, note: None }
        }
        Shape::Doc => {
            r.remove("ok");
            r.remove("$clusterTime");
            r.remove("operationTime");
            ShellReply { blocks: vec![relaxed(&r)], note: None }
        }
    }
}

/// Sends a planned command (the gate already said yes) and formats the answer.
pub async fn execute(c: &MongoConn, plan: &Plan) -> AppResult<ShellReply> {
    let r = c.command(&plan.db, plan.cmd.clone()).await?;
    if let Some(Bson::Int64(id)) = r.get_document("cursor").ok().and_then(|c| c.get("id")) {
        if *id != 0 {
            let coll = plan.cmd.get_str(plan.cmd.keys().next().map_or("", String::as_str)).unwrap_or("").to_string();
            let _ = c.command(&plan.db, doc! { "killCursors": coll, "cursors": [*id] }).await;
        }
    }
    Ok(reply_of(&plan.shape, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(line: &str) -> Plan {
        plan(&parse(line).unwrap(), "shop").unwrap()
    }

    #[test]
    fn values_are_relaxed_json_with_the_shells_helpers() {
        let s = parse(r#"db.orders.find({ status: 'open', n: { $gt: 5, $lte: 1e3 }, when: ISODate("2025-03-01T10:00:00Z"), id: ObjectId("64b64c3f0000000000000001"), big: NumberLong("9007199254740993"), d: NumberDecimal("1.10"), r: /^a.*/i, tags: ["x", "y",], // note
 })"#).unwrap();
        let Stmt::Coll { args, .. } = s else { panic!() };
        let d = args[0].as_document().unwrap();
        assert_eq!(d.get_str("status").unwrap(), "open");
        assert!(matches!(d.get("big"), Some(Bson::Int64(9_007_199_254_740_993))));
        assert!(matches!(d.get("d"), Some(Bson::Decimal128(_))) && matches!(d.get("when"), Some(Bson::DateTime(_))) && matches!(d.get("id"), Some(Bson::ObjectId(_))));
        assert!(matches!(d.get("r"), Some(Bson::RegularExpression(r)) if r.pattern.as_str() == "^a.*" && r.options.as_str() == "i"));
        assert_eq!(d.get_array("tags").unwrap().len(), 2);
        assert_eq!(d.get_document("n").unwrap().get_f64("$lte").unwrap(), 1000.0);
        assert!(matches!(parse("db.c.find({ d: new Date(0) })").unwrap(), Stmt::Coll { .. }));
    }

    #[test]
    fn javascript_and_the_rest_get_the_plain_message() {
        for bad in ["var x = 1", "for (;;) {}", "db.orders.find({}).forEach(printjson)", "db.orders.find(x)", "db.orders.find({ a: Math.max(1) })", "db.a.find({}); db.b.find({})", "x = 5"] {
            let e = parse(bad).and_then(|s| plan(&s, "d").map(|_| ())).unwrap_err();
            assert!(e.message.contains("single commands, not JavaScript") || e.message.contains("isn't supported"), "{bad}: {}", e.message);
        }
        assert!(parse("db.orders.find({").is_err());
    }

    #[test]
    fn use_show_and_getcollection() {
        assert_eq!(parse("use shop;").unwrap(), Stmt::Use("shop".into()));
        assert_eq!(parse("use Shop").unwrap(), Stmt::Use("Shop".into()));
        assert!(parse("use a.b").is_err());
        assert_eq!(parse("show dbs").unwrap(), Stmt::Show("dbs".into()));
        assert!(matches!(parse(r#"db.getCollection("a.b").find()"#).unwrap(), Stmt::Coll { coll, .. } if coll == "a.b"));
    }

    #[test]
    fn each_command_has_its_class() {
        use Class::*;
        let table = [
            ("db.orders.find({}).sort({a:1}).limit(5)", Read),
            ("db.orders.aggregate([{ $match: {} }])", Read),
            ("db.orders.aggregate([{ $match: {} }, { $out: 'x' }])", Write),
            ("db.orders.insertOne({ a: 1 })", Write),
            ("db.orders.updateOne({ a: 1 }, { $set: { b: 2 } })", Write),
            ("db.orders.updateMany({ a: 1 }, { $set: { b: 2 } })", Write),
            ("db.orders.updateMany({}, { $set: { b: 2 } })", Dangerous),
            ("db.orders.deleteMany({})", Dangerous),
            ("db.orders.deleteMany({ a: 1 })", Write),
            ("db.orders.deleteOne({})", Write),
            ("db.orders.findOneAndDelete({ a: 1 })", Write),
            ("db.orders.createIndex({ a: 1 })", Schema),
            ("db.orders.dropIndex('a_1')", Dangerous),
            ("db.orders.dropIndexes()", Dangerous),
            ("db.orders.drop()", Dangerous),
            ("db.dropDatabase()", Dangerous),
            ("db.killOp(123)", Dangerous),
            ("db.createCollection('x')", Schema),
            ("db.runCommand({ ping: 1 })", Read),
            ("db.runCommand({ insert: 'o', documents: [{}] })", Write),
            ("db.runCommand({ createUser: 'x', pwd: 'y', roles: [] })", Dangerous),
            ("db.runCommand({ shutdown: 1 })", Dangerous),
            ("db.runCommand({ aggregate: 'o', pipeline: [{ $merge: { into: 'x' } }], cursor: {} })", Write),
            ("db.adminCommand({ listDatabases: 1 })", Read),
            ("db.runCommand({ eval: 'x' })", Dangerous),
            ("show collections", Read),
        ];
        for (line, want) in table {
            assert_eq!(pl(line).class, want, "{line}");
        }
        assert!(plan(&parse("db.orders.dropIndex('_id_')").unwrap(), "d").is_err());
        let p = pl("db.orders.updateMany({ a: 1 }, { $set: { b: 2 } })");
        assert!(p.matches.is_some());
    }

    #[test]
    fn find_chains_become_find_options() {
        let p = pl("db.orders.find({ a: 1 }, { b: 1 }).sort({ c: -1 }).skip(2).limit(5).hint('a_1').collation({ locale: 'en' })");
        assert_eq!((p.cmd.get_i64("limit").unwrap(), p.cmd.get_i64("skip").unwrap()), (5, 2));
        assert!(p.cmd.contains_key("sort") && p.cmd.contains_key("projection") && p.cmd.contains_key("hint") && p.cmd.contains_key("collation"));
        let c = pl("db.orders.find({ a: 1 }).count()");
        assert_eq!(c.shape, Shape::Field("n"));
        assert!(pl("db.orders.find().explain()").cmd.contains_key("explain"));
        assert!(parse("db.orders.find().forEach(f)").is_err());
        assert!(plan(&parse("db.orders.find().forEach()").unwrap(), "d").is_err());
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target, TlsMode};
    use crate::dbs::Engine;
    use crate::mongo::conn::MongoLogin;
    use crate::mongo::docs::{match_preview, run_reply, MgWrite, Parsed};
    use std::sync::Arc;
    use zeroize::Zeroizing;

    async fn conn() -> MongoConn {
        let reopen: crate::sql::pg::Reopen = Arc::new(|| Box::pin(async { transport::direct_for(Engine::Mongodb, &Target { address: "127.0.0.1".into(), port: 17017, tls: false, tls_server_name: None }).await }));
        let stream = (reopen)().await.unwrap();
        let login = MongoLogin { user: "app".into(), password: Some(Zeroizing::new("test-mongo".into())), auth_db: "admin".into(), tls: TlsMode::Off, server_name: "127.0.0.1".into() };
        MongoConn::connect(stream, login, reopen).await.unwrap().0
    }

    /// What the window does: parse, plan; reads run, everything else is held and then run through `run_reply`.
    async fn go(c: &MongoConn, db: &str, line: &str) -> AppResult<ShellReply> {
        let plan = plan(&parse(line)?, db)?;
        if plan.class == Class::Read {
            return execute(c, &plan).await;
        }
        let coll = plan.cmd.get_str(plan.cmd.keys().next().map_or("", String::as_str)).unwrap_or("").to_string();
        let op = match plan.matches.clone() {
            Some((coll, filter)) => Parsed::Match { class: plan.class, summary: plan.summary.clone(), cmd: plan.cmd.clone(), coll, filter },
            None => Parsed::Raw { class: plan.class, summary: plan.summary.clone(), cmd: plan.cmd.clone() },
        };
        let w = MgWrite { db: plan.db.clone(), coll, op };
        Ok(reply_of(&Shape::Doc, run_reply(c, &w).await?))
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_shell_reads_writes_and_drops() {
        let c = conn().await;
        let _ = go(&c, "d10", "db.dropDatabase()").await;
        go(&c, "d10", r#"db.orders.insertMany([{ _id: 1, status: "open", qty: 3 }, { _id: 2, status: "open", qty: 9 }, { _id: 3, status: "paid", qty: 1, when: ISODate("2025-03-01T10:00:00Z"), big: NumberLong("9007199254740993") }])"#).await.unwrap();
        assert_eq!(go(&c, "d10", "db.orders.countDocuments({})").await.unwrap().blocks, ["3"]);
        assert_eq!(go(&c, "d10", "db.orders.find({ status: 'open' }).sort({ qty: -1 }).limit(1)").await.unwrap().blocks.len(), 1);
        assert!(go(&c, "d10", "db.orders.find({ status: 'open' }).sort({ qty: -1 }).limit(1)").await.unwrap().blocks[0].contains("\"qty\": 9"));
        assert_eq!(go(&c, "d10", "db.orders.find({ status: 'open' }).count()").await.unwrap().blocks, ["2"]);
        assert_eq!(go(&c, "d10", "db.orders.distinct('status')").await.unwrap().blocks[0].matches("open").count(), 1);
        assert!(go(&c, "d10", "db.orders.find({ _id: 3 })").await.unwrap().blocks[0].contains("9007199254740993"));
        assert!(go(&c, "d10", "db.orders.find({ when: { $gt: ISODate('2025-01-01') } })").await.unwrap().blocks.len() == 1);
        assert!(go(&c, "d10", "db.orders.find().explain()").await.unwrap().blocks[0].contains("queryPlanner"));
        assert!(go(&c, "d10", "show collections").await.unwrap().blocks.contains(&"orders".to_string()));
        assert!(go(&c, "d10", "show dbs").await.unwrap().blocks.contains(&"d10".to_string()));
        assert_eq!(go(&c, "d10", "db.orders.aggregate([{ $group: { _id: '$status', n: { $sum: 1 } } }, { $sort: { _id: 1 } }])").await.unwrap().blocks.len(), 2);
        // The preview the gate shows: how many documents a change reaches.
        let p = plan(&parse("db.orders.updateMany({ status: 'open' }, { $set: { seen: true } })").unwrap(), "d10").unwrap();
        let (coll, f) = p.matches.clone().unwrap();
        let (n, sample) = match_preview(&c, "d10", &coll, &f).await.unwrap();
        assert_eq!((n, sample.len()), (2.0, 2));
        let r = go(&c, "d10", "db.orders.updateMany({ status: 'open' }, { $set: { seen: true } })").await.unwrap();
        assert!(r.blocks[0].contains("\"nModified\": 2"), "{}", r.blocks[0]);
        assert!(go(&c, "d10", "db.orders.deleteOne({ _id: 1 })").await.unwrap().blocks[0].contains("\"n\": 1"));
        assert!(go(&c, "d10", "db.orders.createIndex({ status: 1 })").await.is_ok());
        assert!(go(&c, "d10", "db.orders.getIndexes()").await.unwrap().blocks.len() == 2);
        assert!(go(&c, "d10", "db.orders.dropIndex('status_1')").await.is_ok());
        // A server error comes back in its words.
        assert!(go(&c, "d10", "db.orders.insertOne({ _id: 2 })").await.unwrap_err().message.contains("E11000"));
        assert!(go(&c, "d10", "db.runCommand({ bogusCommand: 1 })").await.is_err());
        go(&c, "d10", "db.orders.deleteMany({})").await.unwrap();
        assert_eq!(go(&c, "d10", "db.orders.countDocuments({})").await.unwrap().blocks, ["0"]);
        go(&c, "d10", "db.dropDatabase()").await.unwrap();
    }
}
