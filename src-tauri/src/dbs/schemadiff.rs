//! Schema compare and sync (D16, E3; replica/databases/restore-copy.md §9).
//!
//! A `Catalog` is one schema (PostgreSQL) or database (MySQL/MariaDB) as `sql/schemacat.rs` reads it: tables with
//! their columns, keys, checks, foreign keys and indexes, and the other objects (views, functions, triggers,
//! sequences, enums) as their own CREATE statement. Definitions are stored without the schema's own name, so two
//! schemas compare equal when only their names differ.
//!
//! `diff` lists what makes the target differ from the source (added = only in the source, removed = only in the
//! target, changed = in both but different). `script` turns the chosen differences into statements that make the
//! target match the source, in dependency order, with every drop last. A drop or a column type change is dangerous:
//! the gate asks for the typed name.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Pg,
    My,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    /// PostgreSQL: `GENERATED … AS IDENTITY` or `GENERATED ALWAYS AS (…) STORED`; MySQL: part of `definition`.
    pub generated: Option<String>,
    /// What follows the name in a CREATE TABLE (`bigint NOT NULL DEFAULT 0`).
    pub definition: String,
}

/// `p` primary key, `u` unique, `c` check, `f` foreign key (PostgreSQL also `x` exclusion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub name: String,
    pub kind: String,
    /// PostgreSQL: `pg_get_constraintdef` (`PRIMARY KEY (id)`); MySQL: the line of SHOW CREATE TABLE.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    /// PostgreSQL: `CREATE INDEX …`; MySQL: the `KEY`/`UNIQUE KEY` line.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub constraints: Vec<Constraint>,
    pub indexes: Vec<Index>,
    /// CREATE TABLE with its columns, primary key, unique keys and checks, but no foreign keys (they come after
    /// every table exists).
    pub create: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ObjKind {
    Enum,
    Sequence,
    View,
    Function,
    Trigger,
}

impl ObjKind {
    fn word(self) -> &'static str {
        match self {
            ObjKind::Enum => "enum",
            ObjKind::Sequence => "sequence",
            ObjKind::View => "view",
            ObjKind::Function => "function",
            ObjKind::Trigger => "trigger",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obj {
    pub kind: ObjKind,
    /// Functions carry their argument types (`total(integer)`); triggers their table (`orders.touch`).
    pub name: String,
    pub definition: String,
    /// The statement that removes it.
    pub drop: String,
    /// Enums: the labels in order (new ones can be added in place).
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub family: Family,
    pub schema: String,
    pub tables: Vec<Table>,
    pub objects: Vec<Obj>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

/// One difference. `table` is set for what belongs to a table (columns, keys, indexes).
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiffItem {
    pub id: String,
    /// table, column, primary_key, unique, check, foreign_key, exclusion, index, enum, sequence, view, function,
    /// trigger.
    pub kind: String,
    pub table: Option<String>,
    pub name: String,
    pub change: Change,
    pub source_ddl: Option<String>,
    pub target_ddl: Option<String>,
    /// What changed, in words ("type integer → bigint").
    pub detail: Vec<String>,
    /// Applying it drops something or changes a column's type: the gate asks for the typed name.
    pub dangerous: bool,
}

/// A table's whole CREATE on each side, for the tree's table rows.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TableDdl {
    pub name: String,
    pub source: Option<String>,
    pub target: Option<String>,
}

fn constraint_kind(k: &str) -> &'static str {
    match k {
        "p" => "primary_key",
        "u" => "unique",
        "c" => "check",
        "f" => "foreign_key",
        _ => "exclusion",
    }
}

/// An index definition without its own name, so the same index under another name matches.
fn index_key(family: Family, ix: &Index) -> String {
    match family {
        Family::Pg => ix
            .definition
            .replacen(&format!(" {} ON ", quote_pg(&ix.name)), " ON ", 1)
            .replacen(&format!(" {} ON ", ix.name), " ON ", 1),
        Family::My => ix
            .definition
            .replacen(&format!("{} ", quote_my(&ix.name)), "", 1),
    }
}

fn constraint_key(family: Family, c: &Constraint) -> String {
    match family {
        Family::Pg => format!("{}:{}", c.kind, c.definition),
        // MySQL lines start with `CONSTRAINT \`name\``: compare what follows.
        Family::My => format!(
            "{}:{}",
            c.kind,
            c.definition
                .replacen(&format!("CONSTRAINT {} ", quote_my(&c.name)), "", 1)
        ),
    }
}

pub fn quote_pg(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

pub fn quote_my(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn qn(f: Family, name: &str) -> String {
    match f {
        Family::Pg => quote_pg(name),
        Family::My => quote_my(name),
    }
}

/// Removes `schema.` qualifications of the catalog's own schema from a definition.
pub fn unqualify(family: Family, schema: &str, text: &str) -> String {
    match family {
        Family::Pg => {
            let quoted = format!("\"{}\".", schema.replace('"', "\"\""));
            let mut t = text.replace(&quoted, "");
            if quote_pg(schema) == schema {
                // Only where a name starts: not inside another identifier (`xpublic.t`).
                let pat = format!("{schema}.");
                let mut out = String::with_capacity(t.len());
                let mut rest = t.as_str();
                while let Some(i) = rest.find(&pat) {
                    let before = rest[..i].chars().last();
                    let starts =
                        before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '"'));
                    out.push_str(&rest[..i]);
                    if !starts {
                        out.push_str(&pat);
                    }
                    rest = &rest[i + pat.len()..];
                }
                out.push_str(rest);
                t = out;
            }
            t
        }
        Family::My => text.replace(&format!("{}.", quote_my(schema)), ""),
    }
}

fn column_detail(a: &Column, b: &Column) -> (Vec<String>, bool) {
    let mut d = Vec::new();
    let mut dangerous = false;
    if a.type_name != b.type_name {
        d.push(format!("type {} → {}", b.type_name, a.type_name));
        dangerous = true;
    }
    if a.nullable != b.nullable {
        d.push(if a.nullable {
            "not null → nullable".to_string()
        } else {
            "nullable → not null".to_string()
        });
    }
    if a.default != b.default {
        d.push(format!(
            "default {} → {}",
            b.default.as_deref().unwrap_or("none"),
            a.default.as_deref().unwrap_or("none")
        ));
    }
    if a.generated != b.generated {
        d.push(format!(
            "generated {} → {}",
            b.generated.as_deref().unwrap_or("no"),
            a.generated.as_deref().unwrap_or("no")
        ));
    }
    if d.is_empty() && a.definition != b.definition {
        d.push(format!("{} → {}", b.definition, a.definition));
    }
    (d, dangerous)
}

/// What makes `target` differ from `source`.
pub fn diff(source: &Catalog, target: &Catalog) -> (Vec<DiffItem>, Vec<TableDdl>) {
    let f = source.family;
    let mut items = Vec::new();
    let mut ddls = Vec::new();
    let tt: BTreeMap<&str, &Table> = target.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let st: BTreeMap<&str, &Table> = source.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    for (name, s) in &st {
        let Some(t) = tt.get(name) else {
            items.push(DiffItem {
                id: format!("table:{name}"),
                kind: "table".into(),
                table: None,
                name: name.to_string(),
                change: Change::Added,
                source_ddl: Some(s.create.clone()),
                target_ddl: None,
                detail: vec![format!("{} columns", s.columns.len())],
                dangerous: false,
            });
            // Its foreign keys and indexes are listed under it, so they can be created after every table exists.
            for c in s.constraints.iter().filter(|c| c.kind == "f") {
                items.push(member(
                    name,
                    "foreign_key",
                    &c.name,
                    Change::Added,
                    Some(c.definition.clone()),
                    None,
                    vec![],
                    false,
                ));
            }
            // MySQL's CREATE TABLE already holds its indexes; PostgreSQL creates them after.
            for ix in s.indexes.iter().filter(|_| f == Family::Pg) {
                items.push(member(
                    name,
                    "index",
                    &ix.name,
                    Change::Added,
                    Some(ix.definition.clone()),
                    None,
                    vec![],
                    false,
                ));
            }
            ddls.push(TableDdl {
                name: name.to_string(),
                source: Some(s.create.clone()),
                target: None,
            });
            continue;
        };
        let before = items.len();
        // Columns, by name.
        for c in &s.columns {
            match t.columns.iter().find(|x| x.name == c.name) {
                None => items.push(member(
                    name,
                    "column",
                    &c.name,
                    Change::Added,
                    Some(c.definition.clone()),
                    None,
                    vec![],
                    false,
                )),
                Some(o) if o != c => {
                    let (detail, dangerous) = column_detail(c, o);
                    if !detail.is_empty() {
                        items.push(member(
                            name,
                            "column",
                            &c.name,
                            Change::Changed,
                            Some(c.definition.clone()),
                            Some(o.definition.clone()),
                            detail,
                            dangerous,
                        ));
                    }
                }
                _ => {}
            }
        }
        for o in &t.columns {
            if !s.columns.iter().any(|c| c.name == o.name) {
                items.push(member(
                    name,
                    "column",
                    &o.name,
                    Change::Removed,
                    None,
                    Some(o.definition.clone()),
                    vec![],
                    true,
                ));
            }
        }
        // Constraints: the same definition under any name matches; else by name (changed), else added or removed.
        let mut t_left: Vec<&Constraint> = t.constraints.iter().collect();
        let mut s_left = Vec::new();
        for c in &s.constraints {
            if let Some(i) = t_left
                .iter()
                .position(|o| constraint_key(f, o) == constraint_key(f, c))
            {
                t_left.remove(i);
            } else {
                s_left.push(c);
            }
        }
        for c in s_left {
            let kind = constraint_kind(&c.kind);
            match t_left.iter().position(|o| {
                o.name == c.name && o.kind == c.kind || c.kind == "p" && o.kind == "p"
            }) {
                Some(i) => {
                    let o = t_left.remove(i);
                    items.push(member(
                        name,
                        kind,
                        &c.name,
                        Change::Changed,
                        Some(c.definition.clone()),
                        Some(o.definition.clone()),
                        vec![format!("{} → {}", o.definition, c.definition)],
                        false,
                    ));
                }
                None => items.push(member(
                    name,
                    kind,
                    &c.name,
                    Change::Added,
                    Some(c.definition.clone()),
                    None,
                    vec![],
                    false,
                )),
            }
        }
        for o in t_left {
            items.push(member(
                name,
                constraint_kind(&o.kind),
                &o.name,
                Change::Removed,
                None,
                Some(o.definition.clone()),
                vec![],
                true,
            ));
        }
        // Indexes: the same way.
        let mut t_left: Vec<&Index> = t.indexes.iter().collect();
        let mut s_left = Vec::new();
        for ix in &s.indexes {
            if let Some(i) = t_left
                .iter()
                .position(|o| index_key(f, o) == index_key(f, ix))
            {
                t_left.remove(i);
            } else {
                s_left.push(ix);
            }
        }
        for ix in s_left {
            match t_left.iter().position(|o| o.name == ix.name) {
                Some(i) => {
                    let o = t_left.remove(i);
                    items.push(member(
                        name,
                        "index",
                        &ix.name,
                        Change::Changed,
                        Some(ix.definition.clone()),
                        Some(o.definition.clone()),
                        vec![],
                        false,
                    ));
                }
                None => items.push(member(
                    name,
                    "index",
                    &ix.name,
                    Change::Added,
                    Some(ix.definition.clone()),
                    None,
                    vec![],
                    false,
                )),
            }
        }
        for o in t_left {
            items.push(member(
                name,
                "index",
                &o.name,
                Change::Removed,
                None,
                Some(o.definition.clone()),
                vec![],
                true,
            ));
        }
        if items.len() > before {
            ddls.push(TableDdl {
                name: name.to_string(),
                source: Some(s.create.clone()),
                target: Some(t.create.clone()),
            });
        }
    }
    for (name, t) in &tt {
        if !st.contains_key(name) {
            items.push(DiffItem {
                id: format!("table:{name}"),
                kind: "table".into(),
                table: None,
                name: name.to_string(),
                change: Change::Removed,
                source_ddl: None,
                target_ddl: Some(t.create.clone()),
                detail: vec![format!("{} columns", t.columns.len())],
                dangerous: true,
            });
            ddls.push(TableDdl {
                name: name.to_string(),
                source: None,
                target: Some(t.create.clone()),
            });
        }
    }
    // Other objects, by kind and name.
    for o in &source.objects {
        match target
            .objects
            .iter()
            .find(|x| x.kind == o.kind && x.name == o.name)
        {
            None => items.push(object(o, Change::Added, Some(o), None)),
            Some(t) if t.definition != o.definition => {
                items.push(object(o, Change::Changed, Some(o), Some(t)))
            }
            _ => {}
        }
    }
    for t in &target.objects {
        if !source
            .objects
            .iter()
            .any(|x| x.kind == t.kind && x.name == t.name)
        {
            items.push(object(t, Change::Removed, None, Some(t)));
        }
    }
    let _ = f;
    (items, ddls)
}

#[allow(clippy::too_many_arguments)]
fn member(
    table: &str,
    kind: &str,
    name: &str,
    change: Change,
    source_ddl: Option<String>,
    target_ddl: Option<String>,
    detail: Vec<String>,
    dangerous: bool,
) -> DiffItem {
    DiffItem {
        id: format!("{kind}:{table}.{name}"),
        kind: kind.into(),
        table: Some(table.into()),
        name: name.into(),
        change,
        source_ddl,
        target_ddl,
        detail,
        dangerous,
    }
}

fn object(o: &Obj, change: Change, s: Option<&Obj>, t: Option<&Obj>) -> DiffItem {
    DiffItem {
        id: format!("{}:{}", o.kind.word(), o.name),
        kind: o.kind.word().into(),
        table: None,
        name: o.name.clone(),
        change,
        source_ddl: s.map(|x| x.definition.clone()),
        target_ddl: t.map(|x| x.definition.clone()),
        detail: vec![],
        dangerous: change == Change::Removed,
    }
}

/// One statement of a sync script.
#[derive(Debug, Clone, Serialize, Type)]
pub struct Statement {
    pub sql: String,
    pub dangerous: bool,
}

/// Statements that make the target match the source for the chosen differences, in this order: enums and
/// sequences, new tables, columns, keys and checks, indexes, foreign keys, views, functions and triggers, and then
/// every drop (foreign keys and dependants first, tables after their columns). Each step is in the order of the
/// catalog; a view that depends on another one added in the same script must come later in the source catalog.
pub fn script(
    source: &Catalog,
    target: &Catalog,
    items: &[DiffItem],
    chosen: &HashSet<String>,
) -> Vec<Statement> {
    let f = source.family;
    let pick: Vec<&DiffItem> = items.iter().filter(|i| chosen.contains(&i.id)).collect();
    let tbl = |n: &str| qn(f, n);
    let s_table = |n: &str| source.tables.iter().find(|t| t.name == n);
    let t_table = |n: &str| target.tables.iter().find(|t| t.name == n);
    let s_obj = |i: &DiffItem| {
        source
            .objects
            .iter()
            .find(|o| o.kind.word() == i.kind && o.name == i.name)
    };
    let t_obj = |i: &DiffItem| {
        target
            .objects
            .iter()
            .find(|o| o.kind.word() == i.kind && o.name == i.name)
    };
    let mut steps: [Vec<Statement>; 9] = Default::default();
    let mut drops: [Vec<Statement>; 4] = Default::default();
    let safe = |sql: String| Statement {
        sql,
        dangerous: false,
    };
    let risky = |sql: String| Statement {
        sql,
        dangerous: true,
    };
    for i in &pick {
        let table = i.table.as_deref().unwrap_or("");
        match (i.kind.as_str(), i.change) {
            ("enum" | "sequence", Change::Added) => {
                if let Some(o) = s_obj(i) {
                    steps[0].push(safe(o.definition.clone()));
                }
            }
            ("enum", Change::Changed) => {
                if let (Some(o), Some(t)) = (s_obj(i), t_obj(i)) {
                    for l in o.labels.iter().filter(|l| !t.labels.contains(l)) {
                        steps[0].push(safe(format!(
                            "ALTER TYPE {} ADD VALUE {}",
                            quote_pg(&o.name),
                            lit_pg(l)
                        )));
                    }
                    if t.labels.iter().any(|l| !o.labels.contains(l)) {
                        steps[0].push(safe(format!("-- {}: labels can't be removed from an enum in place; recreate the type by hand", o.name)));
                    }
                }
            }
            ("sequence", Change::Changed) => {
                if let Some(o) = s_obj(i) {
                    steps[0].push(safe(o.definition.replacen(
                        "CREATE SEQUENCE",
                        "ALTER SEQUENCE",
                        1,
                    )));
                }
            }
            ("table", Change::Added) => {
                if let Some(t) = s_table(&i.name) {
                    steps[1].push(safe(t.create.clone()));
                }
            }
            ("table", Change::Removed) => {
                drops[3].push(risky(format!("DROP TABLE {}", tbl(&i.name))))
            }
            ("column", Change::Added) => {
                if let Some(c) =
                    s_table(table).and_then(|t| t.columns.iter().find(|c| c.name == i.name))
                {
                    steps[2].push(safe(format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        tbl(table),
                        qn(f, &c.name),
                        c.definition
                    )));
                }
            }
            ("column", Change::Changed) => {
                let (Some(c), Some(o)) = (
                    s_table(table).and_then(|t| t.columns.iter().find(|c| c.name == i.name)),
                    t_table(table).and_then(|t| t.columns.iter().find(|c| c.name == i.name)),
                ) else {
                    continue;
                };
                let col = qn(f, &c.name);
                match f {
                    Family::My => steps[2].push(Statement {
                        sql: format!(
                            "ALTER TABLE {} MODIFY COLUMN {col} {}",
                            tbl(table),
                            c.definition
                        ),
                        dangerous: c.type_name != o.type_name,
                    }),
                    Family::Pg => {
                        let at = format!("ALTER TABLE {} ALTER COLUMN {col}", tbl(table));
                        if c.default != o.default && o.default.is_some() {
                            steps[2].push(safe(format!("{at} DROP DEFAULT")));
                        }
                        if c.type_name != o.type_name {
                            steps[2].push(risky(format!(
                                "{at} TYPE {} USING {col}::{}",
                                c.type_name, c.type_name
                            )));
                        }
                        if c.nullable != o.nullable {
                            steps[2].push(safe(format!(
                                "{at} {} NOT NULL",
                                if c.nullable { "DROP" } else { "SET" }
                            )));
                        }
                        if c.default != o.default {
                            if let Some(d) = &c.default {
                                steps[2].push(safe(format!("{at} SET DEFAULT {d}")));
                            }
                        }
                        if c.generated != o.generated {
                            let identity = |g: &Option<String>| {
                                g.as_deref()
                                    .filter(|g| g.ends_with("AS IDENTITY"))
                                    .map(|g| {
                                        if g.contains("ALWAYS") {
                                            "ALWAYS"
                                        } else {
                                            "BY DEFAULT"
                                        }
                                    })
                            };
                            match (identity(&c.generated), identity(&o.generated)) {
                                (Some(want), None) if o.generated.is_none() => {
                                    steps[2].push(safe(format!("{at} ADD GENERATED {want} AS IDENTITY")))
                                }
                                (None, Some(_)) if c.generated.is_none() => {
                                    steps[2].push(safe(format!("{at} DROP IDENTITY")))
                                }
                                (Some(want), Some(_)) => {
                                    steps[2].push(safe(format!("{at} SET GENERATED {want}")))
                                }
                                _ => steps[2].push(safe(format!(
                                    "-- {table}.{}: change “{}” to “{}” by hand (a generated column's expression can't be altered in place)",
                                    c.name,
                                    o.generated.as_deref().unwrap_or("not generated"),
                                    c.generated.as_deref().unwrap_or("not generated")
                                ))),
                            }
                        }
                    }
                }
            }
            ("column", Change::Removed) => drops[2].push(risky(format!(
                "ALTER TABLE {} DROP COLUMN {}",
                tbl(table),
                qn(f, &i.name)
            ))),
            ("primary_key" | "unique" | "check" | "exclusion" | "foreign_key", ch) => {
                let s_c = s_table(table).and_then(|t| {
                    t.constraints
                        .iter()
                        .find(|c| c.name == i.name || i.kind == "primary_key" && c.kind == "p")
                });
                let t_c = t_table(table).and_then(|t| {
                    t.constraints
                        .iter()
                        .find(|c| c.name == i.name || i.kind == "primary_key" && c.kind == "p")
                });
                let fk = i.kind == "foreign_key";
                if matches!(ch, Change::Changed | Change::Removed) {
                    if let Some(o) = t_c {
                        let st = drop_constraint(f, table, o);
                        if ch == Change::Removed {
                            drops[if fk { 0 } else { 1 }].push(risky(st));
                        } else {
                            // Replaced right away (a changed key or check): dropped just before its new form.
                            steps[if fk { 6 } else { 3 }].push(safe(st));
                        }
                    }
                }
                if matches!(ch, Change::Changed | Change::Added) {
                    if let Some(c) = s_c {
                        // A new table already has its keys and checks; only its foreign keys come later.
                        let new_table = !fk && t_table(table).is_none();
                        if !new_table {
                            steps[if fk { 6 } else { 3 }].push(safe(add_constraint(f, table, c)));
                        }
                    }
                }
            }
            ("index", ch) => {
                if matches!(ch, Change::Changed | Change::Removed) {
                    let st = drop_index(f, table, &i.name);
                    if ch == Change::Removed {
                        drops[1].push(risky(st));
                    } else {
                        steps[4].push(safe(st));
                    }
                }
                if matches!(ch, Change::Changed | Change::Added) {
                    if let Some(ix) =
                        s_table(table).and_then(|t| t.indexes.iter().find(|x| x.name == i.name))
                    {
                        steps[4].push(safe(match f {
                            Family::Pg => ix.definition.clone(),
                            Family::My => {
                                format!("ALTER TABLE {} ADD {}", tbl(table), ix.definition)
                            }
                        }));
                    }
                }
            }
            ("view" | "function" | "trigger", ch) => {
                let step = match i.kind.as_str() {
                    "view" => 7,
                    _ => 8,
                };
                if ch == Change::Removed {
                    if let Some(o) = t_obj(i) {
                        drops[0].push(risky(o.drop.clone()));
                    }
                    continue;
                }
                if ch == Change::Changed {
                    if let Some(o) = t_obj(i) {
                        steps[step].push(safe(o.drop.clone()));
                    }
                }
                if let Some(o) = s_obj(i) {
                    steps[step].push(safe(o.definition.clone()));
                }
            }
            ("enum" | "sequence", Change::Removed) => {
                if let Some(o) = t_obj(i) {
                    drops[3].push(risky(o.drop.clone()));
                }
            }
            _ => {}
        }
    }
    steps
        .into_iter()
        .flatten()
        .chain(drops.into_iter().flatten())
        .collect()
}

fn lit_pg(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn add_constraint(f: Family, table: &str, c: &Constraint) -> String {
    match f {
        Family::Pg => format!(
            "ALTER TABLE {} ADD CONSTRAINT {} {}",
            quote_pg(table),
            quote_pg(&c.name),
            c.definition
        ),
        Family::My => format!("ALTER TABLE {} ADD {}", quote_my(table), c.definition),
    }
}

fn drop_constraint(f: Family, table: &str, c: &Constraint) -> String {
    match (f, c.kind.as_str()) {
        (Family::Pg, _) => format!(
            "ALTER TABLE {} DROP CONSTRAINT {}",
            quote_pg(table),
            quote_pg(&c.name)
        ),
        (Family::My, "p") => format!("ALTER TABLE {} DROP PRIMARY KEY", quote_my(table)),
        (Family::My, "f") => format!(
            "ALTER TABLE {} DROP FOREIGN KEY {}",
            quote_my(table),
            quote_my(&c.name)
        ),
        (Family::My, _) => format!(
            "ALTER TABLE {} DROP CONSTRAINT {}",
            quote_my(table),
            quote_my(&c.name)
        ),
    }
}

fn drop_index(f: Family, table: &str, name: &str) -> String {
    match f {
        Family::Pg => format!("DROP INDEX {}", quote_pg(name)),
        Family::My => format!(
            "ALTER TABLE {} DROP INDEX {}",
            quote_my(table),
            quote_my(name)
        ),
    }
}

/// The script as text: a header, the statements, and the drops under their own heading.
pub fn script_text(f: Family, target_schema: &str, title: &str, stmts: &[Statement]) -> String {
    let mut out =
        format!("-- {title}\n-- Made by Dalan. Drops come last, under their own heading.\n\n");
    out.push_str(&match f {
        Family::Pg => format!("SET search_path TO {};\n\n", quote_pg(target_schema)),
        Family::My => format!("USE {};\n\n", quote_my(target_schema)),
    });
    let mut in_drops = false;
    for s in stmts {
        let is_drop = s.dangerous && s.sql.trim_start().to_ascii_uppercase().starts_with("DROP")
            || s.dangerous && s.sql.contains(" DROP ");
        if is_drop && !in_drops {
            out.push_str("-- Drops: these remove objects and the data in them.\n\n");
            in_drops = true;
        }
        let sql = s.sql.trim_end().trim_end_matches(';');
        if sql.trim_start().starts_with("--") {
            out.push_str(sql);
            out.push_str("\n\n");
        } else if f == Family::My && (sql.contains("BEGIN") || sql.starts_with("CREATE TRIGGER")) {
            // Routine bodies hold `;`: the mysql client needs another delimiter around them.
            out.push_str(&format!("DELIMITER $$\n{sql}$$\nDELIMITER ;\n\n"));
        } else {
            out.push_str(sql);
            out.push_str(";\n\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, nullable: bool, default: Option<&str>) -> Column {
        Column {
            name: name.into(),
            type_name: ty.into(),
            nullable,
            default: default.map(String::from),
            generated: None,
            definition: format!(
                "{ty}{}{}",
                default.map(|d| format!(" DEFAULT {d}")).unwrap_or_default(),
                if nullable { "" } else { " NOT NULL" }
            ),
        }
    }

    fn pg(tables: Vec<Table>, objects: Vec<Obj>) -> Catalog {
        Catalog {
            family: Family::Pg,
            schema: "public".into(),
            tables,
            objects,
        }
    }

    fn table(
        name: &str,
        columns: Vec<Column>,
        constraints: Vec<(&str, &str, &str)>,
        indexes: Vec<(&str, &str)>,
    ) -> Table {
        Table {
            name: name.into(),
            create: format!("CREATE TABLE {name} (…)"),
            columns,
            constraints: constraints
                .into_iter()
                .map(|(n, k, d)| Constraint {
                    name: n.into(),
                    kind: k.into(),
                    definition: d.into(),
                })
                .collect(),
            indexes: indexes
                .into_iter()
                .map(|(n, d)| Index {
                    name: n.into(),
                    definition: d.into(),
                })
                .collect(),
        }
    }

    fn view(name: &str, sql: &str) -> Obj {
        Obj {
            kind: ObjKind::View,
            name: name.into(),
            definition: format!("CREATE VIEW {name} AS {sql}"),
            drop: format!("DROP VIEW {name}"),
            labels: vec![],
        }
    }

    #[test]
    fn differences_are_found_and_the_script_puts_drops_last() {
        let source = pg(
            vec![
                table(
                    "customers",
                    vec![
                        col("id", "bigint", false, None),
                        col("email", "text", false, None),
                        col("tier", "text", true, Some("'basic'::text")),
                    ],
                    vec![
                        ("customers_pkey", "p", "PRIMARY KEY (id)"),
                        ("customers_email_key", "u", "UNIQUE (email)"),
                    ],
                    vec![(
                        "customers_tier_idx",
                        "CREATE INDEX customers_tier_idx ON customers USING btree (tier)",
                    )],
                ),
                table(
                    "orders",
                    vec![
                        col("id", "bigint", false, None),
                        col("customer_id", "bigint", false, None),
                    ],
                    vec![
                        ("orders_pkey", "p", "PRIMARY KEY (id)"),
                        (
                            "orders_customer_fk",
                            "f",
                            "FOREIGN KEY (customer_id) REFERENCES customers(id)",
                        ),
                    ],
                    vec![],
                ),
            ],
            vec![view("big", "SELECT 1"), view("new_view", "SELECT 2")],
        );
        let target = pg(
            vec![
                table(
                    "customers",
                    // id is an integer here, email nullable, tier missing, an old column.
                    vec![
                        col("id", "integer", false, None),
                        col("email", "text", true, None),
                        col("legacy", "text", true, None),
                    ],
                    // The same key under another name matches; the unique key is missing.
                    vec![("pk_customers", "p", "PRIMARY KEY (id)")],
                    vec![(
                        "idx_tier_old",
                        "CREATE INDEX idx_tier_old ON customers USING btree (email)",
                    )],
                ),
                table(
                    "old_stuff",
                    vec![col("id", "integer", false, None)],
                    vec![],
                    vec![],
                ),
            ],
            vec![view("big", "SELECT 0"), view("gone", "SELECT 3")],
        );
        let (items, ddls) = diff(&source, &target);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        for want in [
            "table:orders",
            "foreign_key:orders.orders_customer_fk",
            "column:customers.id",
            "column:customers.email",
            "column:customers.tier",
            "column:customers.legacy",
            "unique:customers.customers_email_key",
            "index:customers.customers_tier_idx",
            "index:customers.idx_tier_old",
            "table:old_stuff",
            "view:big",
            "view:new_view",
            "view:gone",
        ] {
            assert!(ids.contains(&want), "missing {want} in {ids:?}");
        }
        assert!(
            !ids.iter().any(|i| i.starts_with("primary_key")),
            "a renamed but equal key is no difference: {ids:?}"
        );
        let id = items
            .iter()
            .find(|i| i.id == "column:customers.id")
            .unwrap();
        assert_eq!(id.detail, vec!["type integer → bigint"]);
        assert!(id.dangerous);
        let email = items
            .iter()
            .find(|i| i.id == "column:customers.email")
            .unwrap();
        assert_eq!(email.detail, vec!["nullable → not null"]);
        assert!(!email.dangerous);
        assert!(
            items
                .iter()
                .find(|i| i.id == "table:old_stuff")
                .unwrap()
                .dangerous
        );
        assert!(ddls
            .iter()
            .any(|d| d.name == "customers" && d.source.is_some() && d.target.is_some()));

        let all: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
        let st = script(&source, &target, &items, &all);
        let sql: Vec<&str> = st.iter().map(|s| s.sql.as_str()).collect();
        let at = |needle: &str| {
            sql.iter()
                .position(|s| s.contains(needle))
                .unwrap_or_else(|| panic!("no {needle} in {sql:#?}"))
        };
        assert!(
            at("CREATE TABLE orders") < at("ADD CONSTRAINT orders_customer_fk"),
            "tables before their foreign keys"
        );
        assert!(
            at("ADD COLUMN tier text DEFAULT 'basic'::text")
                < at("CREATE INDEX customers_tier_idx")
        );
        assert!(sql.contains(&"ALTER TABLE customers ALTER COLUMN id TYPE bigint USING id::bigint"));
        assert!(sql.contains(&"ALTER TABLE customers ALTER COLUMN email SET NOT NULL"));
        assert!(sql
            .contains(&"ALTER TABLE customers ADD CONSTRAINT customers_email_key UNIQUE (email)"));
        // A changed view is replaced; the views come after the tables and columns.
        assert!(at("DROP VIEW big") < at("CREATE VIEW big AS SELECT 1"));
        assert!(at("ALTER COLUMN id TYPE") < at("CREATE VIEW big"));
        // Every drop comes last, foreign keys and views first, tables last.
        let first_drop = at("DROP VIEW gone");
        for later in [
            "DROP INDEX idx_tier_old",
            "DROP COLUMN legacy",
            "DROP TABLE old_stuff",
        ] {
            assert!(at(later) > first_drop, "{later}");
            assert!(st[at(later)].dangerous);
        }
        assert!(at("DROP COLUMN legacy") < at("DROP TABLE old_stuff"));
        assert!(st
            .iter()
            .take(first_drop)
            .all(|s| !s.sql.starts_with("DROP TABLE")));
        // Only what was chosen.
        let one: HashSet<String> = ["column:customers.tier".to_string()].into();
        let st = script(&source, &target, &items, &one);
        assert_eq!(st.len(), 1);
        let text = script_text(
            Family::Pg,
            "staging",
            "Make staging match public",
            &script(&source, &target, &items, &all),
        );
        assert!(text.contains("SET search_path TO staging;"));
        assert!(text.find("-- Drops:").unwrap() < text.find("DROP TABLE old_stuff;").unwrap());
        assert!(text.find("-- Drops:").unwrap() > text.find("CREATE TABLE orders").unwrap());
    }

    #[test]
    fn mysql_uses_its_own_statements_and_enums_grow_in_place() {
        let my = |tables| Catalog {
            family: Family::My,
            schema: "shop".into(),
            tables,
            objects: vec![],
        };
        let c = |name: &str, def: &str, ty: &str| Column {
            name: name.into(),
            type_name: ty.into(),
            nullable: !def.contains("NOT NULL"),
            default: None,
            generated: None,
            definition: def.into(),
        };
        let source = my(vec![Table {
            name: "t".into(),
            create: "CREATE TABLE `t` (…)".into(),
            columns: vec![
                c("id", "bigint NOT NULL AUTO_INCREMENT", "bigint"),
                c("note", "varchar(40) DEFAULT NULL", "varchar(40)"),
            ],
            constraints: vec![Constraint {
                name: "PRIMARY".into(),
                kind: "p".into(),
                definition: "PRIMARY KEY (`id`)".into(),
            }],
            indexes: vec![Index {
                name: "by_note".into(),
                definition: "KEY `by_note` (`note`)".into(),
            }],
        }]);
        let target = my(vec![Table {
            name: "t".into(),
            create: "CREATE TABLE `t` (…)".into(),
            columns: vec![c("id", "int NOT NULL AUTO_INCREMENT", "int")],
            constraints: vec![
                Constraint {
                    name: "PRIMARY".into(),
                    kind: "p".into(),
                    definition: "PRIMARY KEY (`id`)".into(),
                },
                Constraint {
                    name: "t_fk".into(),
                    kind: "f".into(),
                    definition: "CONSTRAINT `t_fk` FOREIGN KEY (`id`) REFERENCES `u` (`id`)".into(),
                },
            ],
            indexes: vec![Index {
                name: "note_idx".into(),
                definition: "KEY `note_idx` (`id`)".into(),
            }],
        }]);
        let (items, _) = diff(&source, &target);
        let all: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
        let sql: Vec<String> = script(&source, &target, &items, &all)
            .into_iter()
            .map(|s| s.sql)
            .collect();
        assert!(
            sql.contains(
                &"ALTER TABLE `t` MODIFY COLUMN `id` bigint NOT NULL AUTO_INCREMENT".to_string()
            ),
            "{sql:#?}"
        );
        assert!(
            sql.contains(&"ALTER TABLE `t` ADD COLUMN `note` varchar(40) DEFAULT NULL".to_string())
        );
        assert!(sql.contains(&"ALTER TABLE `t` ADD KEY `by_note` (`note`)".to_string()));
        assert!(sql.contains(&"ALTER TABLE `t` DROP INDEX `note_idx`".to_string()));
        assert_eq!(sql.last().unwrap(), "ALTER TABLE `t` DROP INDEX `note_idx`");
        assert!(
            sql.iter()
                .position(|s| s.contains("DROP FOREIGN KEY `t_fk`"))
                .unwrap()
                < sql.len() - 1
        );

        let e = |labels: &[&str]| Obj {
            kind: ObjKind::Enum,
            name: "mood".into(),
            definition: format!("CREATE TYPE mood AS ENUM ({})", labels.join(", ")),
            drop: "DROP TYPE mood".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        };
        let s = pg(vec![], vec![e(&["sad", "ok", "happy"])]);
        let t = pg(vec![], vec![e(&["sad", "ok"])]);
        let (items, _) = diff(&s, &t);
        let all: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
        let st = script(&s, &t, &items, &all);
        assert_eq!(st[0].sql, "ALTER TYPE mood ADD VALUE 'happy'");
    }

    #[test]
    fn the_schemas_own_name_is_removed_from_definitions() {
        assert_eq!(
            unqualify(
                Family::Pg,
                "public",
                "CREATE INDEX i ON public.t USING btree (x); SELECT xpublic.y, \"public\".z"
            ),
            "CREATE INDEX i ON t USING btree (x); SELECT xpublic.y, z"
        );
        assert_eq!(unqualify(Family::Pg, "Shop", "ON \"Shop\".t"), "ON t");
        assert_eq!(
            unqualify(Family::My, "shop", "select `shop`.`t`.`id` from `shop`.`t`"),
            "select `t`.`id` from `t`"
        );
    }
}
