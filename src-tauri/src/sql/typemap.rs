//! Column types between PostgreSQL and MySQL/MariaDB (D13 part 2; replica/databases/restore-copy.md §3 "Type
//! mapping"). Each mapping says the target type, how values are read and written on the way (`Conv`), and, when
//! something is lost, why in plain words. The person can override any target type in the review.

/// How a value is read from the source and written to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conv {
    /// As text, unchanged.
    Plain,
    /// PostgreSQL boolean ↔ MySQL tinyint(1): `1`/`0`.
    Bool,
    /// Binary: read as hex, written as a hex literal (MySQL) or `\x…` (PostgreSQL).
    Bytes,
    /// PostgreSQL arrays, composites, ranges, hstore, json → JSON text.
    Json,
    /// PostgreSQL timestamptz → a UTC datetime.
    Utc,
    /// PostgreSQL interval → ISO 8601 text.
    Interval,
    /// PostgreSQL time with time zone → time.
    TimeOnly,
    /// MySQL zero dates (`0000-00-00`) → NULL.
    ZeroDate,
    /// MySQL `set('a','b')` value `a,b` → PostgreSQL `text[]` `{a,b}`.
    Set,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapped {
    pub target: String,
    pub conv: Conv,
    pub lossy: Option<String>,
}

fn m(target: impl Into<String>, conv: Conv, lossy: Option<&str>) -> Mapped {
    Mapped {
        target: target.into(),
        conv,
        lossy: lossy.map(str::to_string),
    }
}

/// `numeric(10,2)` → `Some("10,2")`.
fn args(t: &str) -> Option<&str> {
    let a = t.find('(')?;
    let b = t.rfind(')')?;
    (b > a).then(|| &t[a + 1..b])
}

/// A PostgreSQL column (`format_type` text, its `typcategory`) to MySQL. `key`: the column is in a key or index,
/// where MySQL can't take an unbounded text.
pub fn pg_to_my(pg: &str, category: &str, key: bool) -> Mapped {
    let t = pg.trim().to_ascii_lowercase();
    let base = t.split('(').next().unwrap_or("").trim();
    if t.ends_with("[]") || category == "A" {
        return m("json", Conv::Json, Some("an array is stored as JSON"));
    }
    if category == "E" {
        // The labels are added by the caller (it knows them).
        return m("enum", Conv::Plain, None);
    }
    match base {
        "smallint" => m("smallint", Conv::Plain, None),
        "integer" => m("int", Conv::Plain, None),
        "bigint" => m("bigint", Conv::Plain, None),
        "numeric" | "decimal" => match args(&t) {
            Some(a) => m(format!("decimal({a})"), Conv::Plain, None),
            None => m(
                "decimal(65,30)",
                Conv::Plain,
                Some("an unbounded numeric becomes decimal(65,30): more digits are rounded"),
            ),
        },
        "real" => m("float", Conv::Plain, None),
        "double precision" => m("double", Conv::Plain, None),
        "boolean" => m("tinyint(1)", Conv::Bool, None),
        "text" | "citext" if key => m(
            "varchar(768)",
            Conv::Plain,
            Some("a text in a key becomes varchar(768): longer values fail"),
        ),
        "text" | "citext" | "name" => m("longtext", Conv::Plain, None),
        "character varying" => match args(&t).and_then(|a| a.parse::<u32>().ok()) {
            Some(n) if key && n > 768 => m(
                "varchar(768)",
                Conv::Plain,
                Some("a varchar key over 3,072 bytes is cut to varchar(768): longer values fail"),
            ),
            Some(n) if n > 16383 => m("longtext", Conv::Plain, None),
            Some(n) => m(format!("varchar({n})"), Conv::Plain, None),
            None if key => m(
                "varchar(768)",
                Conv::Plain,
                Some("an unbounded varchar in a key becomes varchar(768)"),
            ),
            None => m("longtext", Conv::Plain, None),
        },
        "character" => m(
            format!("char({})", args(&t).unwrap_or("1")),
            Conv::Plain,
            None,
        ),
        "bytea" => m("longblob", Conv::Bytes, None),
        "timestamp with time zone" => m(
            "datetime(6)",
            Conv::Utc,
            Some("the time zone is dropped, values are stored in UTC"),
        ),
        "timestamp without time zone" | "timestamp" => m("datetime(6)", Conv::Plain, None),
        "date" => m("date", Conv::Plain, None),
        "time without time zone" | "time" => m("time(6)", Conv::Plain, None),
        "time with time zone" => m("time(6)", Conv::TimeOnly, Some("the time zone is dropped")),
        "interval" => m(
            "varchar(64)",
            Conv::Interval,
            Some("an interval is stored as ISO 8601 text"),
        ),
        "uuid" => m("char(36)", Conv::Plain, None),
        "json" | "jsonb" => m("json", Conv::Plain, None),
        "inet" | "cidr" | "macaddr" | "macaddr8" => {
            m("varchar(64)", Conv::Plain, Some("stored as text"))
        }
        "money" => m(
            "decimal(19,2)",
            Conv::Plain,
            Some("money loses its currency format"),
        ),
        "xml" => m("longtext", Conv::Plain, Some("stored as text")),
        "bit" | "bit varying" => m("varchar(64)", Conv::Plain, Some("bits are stored as text")),
        "point" | "line" | "lseg" | "box" | "path" | "polygon" | "circle" | "tsvector"
        | "tsquery" => m("longtext", Conv::Plain, Some("stored as its text form")),
        _ => m(
            "json",
            Conv::Json,
            Some("a type MySQL doesn't have is stored as JSON"),
        ),
    }
}

/// A MySQL or MariaDB column (`COLUMN_TYPE`, e.g. `int unsigned`, `enum('a','b')`) to PostgreSQL.
pub fn my_to_pg(column_type: &str) -> Mapped {
    let t = column_type.trim().to_ascii_lowercase();
    let unsigned = t.contains("unsigned");
    let base = t.split(['(', ' ']).next().unwrap_or("");
    match base {
        "tinyint" if t.starts_with("tinyint(1)") => m("boolean", Conv::Bool, None),
        "tinyint" | "smallint" if unsigned => m("integer", Conv::Plain, None),
        "tinyint" | "smallint" | "year" => m("smallint", Conv::Plain, None),
        "mediumint" | "int" | "integer" if unsigned => m("bigint", Conv::Plain, None),
        "mediumint" | "int" | "integer" => m("integer", Conv::Plain, None),
        "bigint" if unsigned => m(
            "numeric(20)",
            Conv::Plain,
            Some("bigint unsigned becomes numeric(20)"),
        ),
        "bigint" => m("bigint", Conv::Plain, None),
        "decimal" | "numeric" => m(
            format!("numeric({})", args(&t).unwrap_or("10,0")),
            Conv::Plain,
            None,
        ),
        "float" => m("real", Conv::Plain, None),
        "double" | "real" => m("double precision", Conv::Plain, None),
        "bit" if args(&t) == Some("1") => m("boolean", Conv::Bool, None),
        "bit" => m("bytea", Conv::Bytes, Some("bits are stored as bytes")),
        "char" => m(
            format!("character({})", args(&t).unwrap_or("1")),
            Conv::Plain,
            None,
        ),
        "varchar" => m(
            format!("character varying({})", args(&t).unwrap_or("255")),
            Conv::Plain,
            None,
        ),
        "tinytext" | "text" | "mediumtext" | "longtext" => m("text", Conv::Plain, None),
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            m("bytea", Conv::Bytes, None)
        }
        "date" => m("date", Conv::ZeroDate, None),
        "datetime" => m(
            format!("timestamp({})", args(&t).unwrap_or("0")),
            Conv::ZeroDate,
            Some("zero dates become NULL"),
        ),
        "timestamp" => m(
            format!("timestamp({})", args(&t).unwrap_or("0")),
            Conv::ZeroDate,
            Some("zero dates become NULL"),
        ),
        "time" => m(
            "interval",
            Conv::Plain,
            Some("a MySQL time can pass 24 hours, so it becomes an interval"),
        ),
        "json" => m("jsonb", Conv::Plain, None),
        "enum" => m("enum", Conv::Plain, None),
        "set" => m("text[]", Conv::Set, Some("a set becomes an array of text")),
        "geometry" | "point" | "linestring" | "polygon" | "multipoint" | "multilinestring"
        | "multipolygon" | "geometrycollection" => m(
            "bytea",
            Conv::Bytes,
            Some("geometry is copied as its binary form (no PostGIS type)"),
        ),
        _ => m(
            "text",
            Conv::Plain,
            Some("an unknown type is stored as text"),
        ),
    }
}

/// The labels of `enum('a','b')` / `set('x','y')` (quotes doubled inside).
pub fn my_labels(column_type: &str) -> Vec<String> {
    let Some(inner) = args(column_type) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inq = false;
    let b: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if inq {
            if c == '\'' && b.get(i + 1) == Some(&'\'') {
                cur.push('\'');
                i += 1;
            } else if c == '\'' {
                inq = false;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        } else if c == '\'' {
            inq = true;
        }
        i += 1;
    }
    out
}

/// A default from one engine that means the same on the other: numbers, quoted text, the current time. Anything
/// else (a function, a sequence) is `None` and the column gets no default.
pub fn default_for(default: &str, to_my: bool) -> Option<String> {
    let d = default.trim();
    let d = d.split("::").next().unwrap_or(d).trim();
    let low = d.to_ascii_lowercase();
    if matches!(
        low.as_str(),
        "now()" | "current_timestamp" | "current_timestamp()" | "localtimestamp"
    ) {
        return Some("CURRENT_TIMESTAMP".into());
    }
    if low == "true" || low == "false" {
        return Some(if to_my {
            if low == "true" { "1" } else { "0" }.into()
        } else {
            low
        });
    }
    if d.parse::<f64>().is_ok() {
        return Some(d.to_string());
    }
    if d.len() >= 2 && d.starts_with('\'') && d.ends_with('\'') {
        return Some(d.to_string());
    }
    // MySQL's information_schema gives bare text for a string default.
    if !to_my && !d.is_empty() && !d.contains('(') && !low.starts_with("null") {
        return Some(format!("'{}'", d.replace('\'', "''")));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_to_mysql_marks_what_is_lost() {
        let t = |pg: &str| pg_to_my(pg, "", false);
        assert_eq!(t("integer").target, "int");
        assert_eq!(t("numeric(10,2)").target, "decimal(10,2)");
        assert!(t("numeric").lossy.is_some());
        assert_eq!(
            (t("boolean").target.as_str(), t("boolean").conv),
            ("tinyint(1)", Conv::Bool)
        );
        assert_eq!(
            t("timestamp with time zone").lossy.as_deref(),
            Some("the time zone is dropped, values are stored in UTC")
        );
        assert_eq!(t("character varying(40)").target, "varchar(40)");
        assert_eq!(pg_to_my("text", "S", true).target, "varchar(768)");
        assert_eq!(
            pg_to_my("character varying(2000)", "S", true).target,
            "varchar(768)"
        );
        assert_eq!(t("uuid").target, "char(36)");
        assert_eq!(
            (t("integer[]").target.as_str(), t("integer[]").conv),
            ("json", Conv::Json)
        );
        assert_eq!(t("bytea").conv, Conv::Bytes);
        assert_eq!(pg_to_my("mood", "E", false).target, "enum");
        assert!(t("tsrange").lossy.is_some());
    }

    #[test]
    fn mysql_to_postgres_keeps_values_whole() {
        assert_eq!(my_to_pg("tinyint(1)").target, "boolean");
        assert_eq!(my_to_pg("int unsigned").target, "bigint");
        assert_eq!(my_to_pg("bigint unsigned").target, "numeric(20)");
        assert_eq!(my_to_pg("int").target, "integer");
        assert_eq!(my_to_pg("varchar(100)").target, "character varying(100)");
        assert_eq!(my_to_pg("decimal(10,2)").target, "numeric(10,2)");
        assert_eq!(
            (
                my_to_pg("datetime(6)").target.as_str(),
                my_to_pg("datetime").conv
            ),
            ("timestamp(6)", Conv::ZeroDate)
        );
        assert_eq!(my_to_pg("json").target, "jsonb");
        assert_eq!(
            (
                my_to_pg("set('a','b')").target.as_str(),
                my_to_pg("set('a','b')").conv
            ),
            ("text[]", Conv::Set)
        );
        assert_eq!(my_to_pg("year").target, "smallint");
        assert_eq!(my_to_pg("longblob").conv, Conv::Bytes);
        assert_eq!(
            my_labels("enum('a','it''s','b,c')"),
            vec!["a", "it's", "b,c"]
        );
    }

    #[test]
    fn defaults_cross_when_they_mean_the_same() {
        assert_eq!(
            default_for("now()", true).as_deref(),
            Some("CURRENT_TIMESTAMP")
        );
        assert_eq!(default_for("'open'::text", true).as_deref(), Some("'open'"));
        assert_eq!(default_for("nextval('t_id_seq'::regclass)", true), None);
        assert_eq!(default_for("true", true).as_deref(), Some("1"));
        assert_eq!(default_for("open", false).as_deref(), Some("'open'"));
        assert_eq!(
            default_for("CURRENT_TIMESTAMP", false).as_deref(),
            Some("CURRENT_TIMESTAMP")
        );
        assert_eq!(default_for("0", false).as_deref(), Some("0"));
    }
}
