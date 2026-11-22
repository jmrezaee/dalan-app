//! Cells of a result, as the window sees them (spec section 3 "Values"). Every value is sent as text with a kind, so
//! nothing loses precision on the way to JavaScript (a bigint or a numeric is never a JS number). Large values arrive
//! cut, and the window asks for the whole one with `sql_cell`.

use serde::Serialize;
use specta::Type;

/// Text over this many characters is cut (spec: 64 KB).
pub const CUT_AT: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Null,
    Bool,
    Int,
    Decimal,
    Float,
    Text,
    Bytes,
    Json,
    Datetime,
    Array,
    Enum,
    Uuid,
    Interval,
    Unknown,
}

impl CellKind {
    /// From the type's name and `pg_type.typcategory`.
    pub fn of_pg(typname: &str, category: &str) -> CellKind {
        match typname {
            "bool" => CellKind::Bool,
            "int2" | "int4" | "int8" | "oid" | "xid" | "cid" => CellKind::Int,
            "numeric" | "money" => CellKind::Decimal,
            "float4" | "float8" => CellKind::Float,
            "bytea" => CellKind::Bytes,
            "json" | "jsonb" => CellKind::Json,
            "uuid" => CellKind::Uuid,
            _ => match category {
                "S" => CellKind::Text,
                "D" => CellKind::Datetime,
                "A" => CellKind::Array,
                "E" => CellKind::Enum,
                "T" => CellKind::Interval,
                "G" => CellKind::Text,
                "N" => CellKind::Decimal,
                _ => CellKind::Unknown,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Cell {
    pub kind: CellKind,
    /// The value's text form; `None` for NULL.
    pub v: Option<String>,
    /// The text was cut at `CUT_AT` characters.
    pub cut: bool,
}

impl Cell {
    /// `text` is the server's own text form of the value (`col::text`).
    pub fn new(kind: CellKind, text: Option<String>) -> Cell {
        match text {
            None => Cell {
                kind: CellKind::Null,
                v: None,
                cut: false,
            },
            Some(t) => {
                let cut = t.chars().count() > CUT_AT;
                let v = if cut {
                    t.chars().take(CUT_AT).collect()
                } else {
                    t
                };
                Cell {
                    kind,
                    v: Some(v),
                    cut,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_type_and_values_keep_every_digit() {
        assert_eq!(CellKind::of_pg("int8", "N"), CellKind::Int);
        assert_eq!(CellKind::of_pg("numeric", "N"), CellKind::Decimal);
        assert_eq!(CellKind::of_pg("jsonb", "U"), CellKind::Json);
        assert_eq!(CellKind::of_pg("timestamptz", "D"), CellKind::Datetime);
        assert_eq!(CellKind::of_pg("_int4", "A"), CellKind::Array);
        assert_eq!(CellKind::of_pg("order_status", "E"), CellKind::Enum);
        let big = Cell::new(CellKind::Int, Some("9223372036854775807".into()));
        assert_eq!(big.v.as_deref(), Some("9223372036854775807"));
        assert_eq!(Cell::new(CellKind::Text, None).kind, CellKind::Null);
    }

    #[test]
    fn long_text_is_cut_and_marked() {
        let c = Cell::new(CellKind::Text, Some("x".repeat(CUT_AT + 10)));
        assert!(c.cut);
        assert_eq!(c.v.unwrap().chars().count(), CUT_AT);
        assert!(!Cell::new(CellKind::Text, Some("short".into())).cut);
    }
}
