//! Masking for copies and exports (D14, E5; replica/databases/restore-copy.md §9). Values are changed in Dalan's
//! stream, between the source and the target or the file: unmasked values never reach either.
//!
//! Every rule is keyed: the same input gives the same output with the same key (a saved job keeps its key), so a
//! hashed id still joins across tables and a fake name is the same in every row it appears in. The fakes come from a
//! small built-in generator (no network, no data set).

use hmac::{KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MaskKind {
    Keep,
    Null,
    /// `value` in every row.
    Fixed,
    FakeEmail,
    FakeName,
    FakePhone,
    FakeAddress,
    FakeIban,
    /// Same shape (digits stay digits, letters letters), keyed: joins survive.
    Hash,
    /// The column's values, shuffled within each batch of rows.
    Shuffle,
    /// The first `value` characters.
    Truncate,
    /// Dates moved by up to ± `value` days (the same value moves the same way).
    DateShift,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MaskRule {
    pub column: String,
    pub kind: MaskKind,
    #[serde(default)]
    pub value: Option<String>,
}

/// A column that looks personal, and the rule Dalan suggests for it.
#[derive(Debug, Clone, Serialize, Type)]
pub struct MaskSuggestion {
    pub table: String,
    pub column: String,
    pub kind: MaskKind,
}

/// The rule a column's name suggests, if it looks personal.
pub fn suggest(column: &str) -> Option<MaskKind> {
    let c = column.to_ascii_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| c.contains(x));
    Some(if has(&["email", "e_mail", "mail"]) {
        MaskKind::FakeEmail
    } else if has(&["phone", "mobile", "tel", "fax"]) {
        MaskKind::FakePhone
    } else if has(&["iban", "account_no", "account_number", "bank"]) {
        MaskKind::FakeIban
    } else if has(&[
        "ssn",
        "national_id",
        "passport",
        "tax_id",
        "card",
        "credit",
        "cvv",
    ]) {
        MaskKind::Hash
    } else if has(&["birth", "dob"]) {
        MaskKind::DateShift
    } else if has(&["address", "street", "city", "zip", "postcode", "postal"]) {
        MaskKind::FakeAddress
    } else if c == "name"
        || has(&[
            "first_name",
            "firstname",
            "last_name",
            "lastname",
            "full_name",
            "fullname",
            "surname",
            "given_name",
        ])
    {
        MaskKind::FakeName
    } else {
        return None;
    })
}

const FIRST: &[&str] = &[
    "Ada", "Bo", "Cyra", "Dario", "Elin", "Farid", "Gita", "Hugo", "Iris", "Jonas", "Kaveh",
    "Lena", "Milo", "Nora", "Omid", "Pia", "Reza", "Sana", "Tomas", "Vera",
];
const LAST: &[&str] = &[
    "Alavi", "Berg", "Costa", "Dahl", "Ekström", "Ferreira", "Gold", "Hansen", "Ito", "Jensen",
    "Karimi", "Lund", "Moreau", "Novak", "Okafor", "Petrov", "Rahimi", "Silva", "Tanaka", "Weber",
];
const STREETS: &[&str] = &[
    "Maple", "Harbor", "Linden", "Station", "Orchard", "Mill", "River", "Church", "Park", "Hill",
];
const SUFFIX: &[&str] = &["Street", "Road", "Lane", "Avenue", "Way"];

/// The masker for one run: the key and the rules by column position.
pub struct Masker {
    key: [u8; 32],
    rules: Vec<(usize, MaskRule)>,
}

impl Masker {
    /// `columns`: the stream's column names in order. Rules for columns not in it are ignored.
    pub fn new(key: [u8; 32], rules: &[MaskRule], columns: &[String]) -> Option<Masker> {
        let rules: Vec<(usize, MaskRule)> = rules
            .iter()
            .filter(|r| r.kind != MaskKind::Keep)
            .filter_map(|r| {
                columns
                    .iter()
                    .position(|c| *c == r.column)
                    .map(|i| (i, r.clone()))
            })
            .collect();
        (!rules.is_empty()).then_some(Masker { key, rules })
    }

    fn mac(&self, column: &str, value: &str, n: u32) -> [u8; 32] {
        let mut m =
            <hmac::Hmac<Sha256> as KeyInit>::new_from_slice(&self.key).expect("any key length");
        m.update(column.as_bytes());
        m.update(&[0]);
        m.update(value.as_bytes());
        m.update(&n.to_le_bytes());
        m.finalize().into_bytes().into()
    }

    /// A stream of keyed bytes for one value (as long as it needs).
    fn bytes(&self, column: &str, value: &str, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 32);
        let mut n = 0;
        while out.len() < len {
            out.extend_from_slice(&self.mac(column, value, n));
            n += 1;
        }
        out.truncate(len.max(1));
        out
    }

    /// One value, by one rule. Shuffle is done per batch in [`Masker::batch`].
    pub fn value(&self, rule: &MaskRule, v: Option<&str>) -> Option<String> {
        let v = v?;
        let col = rule.column.as_str();
        Some(match rule.kind {
            MaskKind::Keep | MaskKind::Shuffle => v.to_string(),
            MaskKind::Null => return None,
            MaskKind::Fixed => rule.value.clone().unwrap_or_default(),
            MaskKind::Truncate => v
                .chars()
                .take(
                    rule.value
                        .as_deref()
                        .and_then(|n| n.trim().parse().ok())
                        .unwrap_or(1),
                )
                .collect(),
            MaskKind::Hash => self.shape(col, v, false),
            MaskKind::FakePhone => self.shape(col, v, true),
            MaskKind::FakeIban => {
                let keep: String = v
                    .chars()
                    .take(2)
                    .filter(char::is_ascii_alphabetic)
                    .collect();
                format!(
                    "{keep}{}",
                    self.shape(col, &v[keep.len()..], false)
                        .to_ascii_uppercase()
                )
            }
            MaskKind::FakeName => {
                let b = self.bytes(col, v, 2);
                format!(
                    "{} {}",
                    FIRST[b[0] as usize % FIRST.len()],
                    LAST[b[1] as usize % LAST.len()]
                )
            }
            MaskKind::FakeEmail => {
                let b = self.bytes(col, v, 4);
                format!(
                    "{}.{}{}@example.test",
                    FIRST[b[0] as usize % FIRST.len()].to_ascii_lowercase(),
                    LAST[b[1] as usize % LAST.len()]
                        .to_ascii_lowercase()
                        .replace('ö', "o"),
                    u16::from_le_bytes([b[2], b[3]]) % 1000
                )
            }
            MaskKind::FakeAddress => {
                let b = self.bytes(col, v, 4);
                format!(
                    "{} {} {}",
                    1 + u16::from_le_bytes([b[0], b[1]]) % 999,
                    STREETS[b[2] as usize % STREETS.len()],
                    SUFFIX[b[3] as usize % SUFFIX.len()]
                )
            }
            MaskKind::DateShift => {
                let range: i64 = rule
                    .value
                    .as_deref()
                    .and_then(|n| n.trim().parse().ok())
                    .unwrap_or(30)
                    .max(1);
                let b = self.bytes(col, v, 8);
                let days = (i64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]))
                    .rem_euclid(2 * range + 1))
                    - range;
                // Never zero: a shifted date is never the real one.
                let days = if days == 0 { range } else { days };
                shift_date(v, days).unwrap_or_else(|| v.to_string())
            }
        })
    }

    /// Same shape, keyed: digits become digits, letters letters of the same case; the rest stays (dashes, spaces,
    /// `@`, dots). `digits_only`: letters are kept too (phone numbers with extensions).
    fn shape(&self, col: &str, v: &str, digits_only: bool) -> String {
        let b = self.bytes(col, v, v.len().max(1));
        v.chars()
            .zip(b.iter().cycle())
            .map(|(c, x)| match c {
                '0'..='9' => (b'0' + x % 10) as char,
                'a'..='z' if !digits_only => (b'a' + x % 26) as char,
                'A'..='Z' if !digits_only => (b'A' + x % 26) as char,
                other => other,
            })
            .collect()
    }

    /// Masks a batch of rows in place (shuffle mixes the column within the batch).
    pub fn batch(&self, rows: &mut [Vec<Option<String>>]) {
        for (i, rule) in &self.rules {
            if rule.kind == MaskKind::Shuffle {
                // Fisher–Yates with keyed choices: a batch with the same values shuffles the same way.
                let n = rows.len();
                for k in (1..n).rev() {
                    let b = self.bytes(
                        &rule.column,
                        &format!(
                            "{k}:{}",
                            rows[k].get(*i).cloned().flatten().unwrap_or_default()
                        ),
                        4,
                    );
                    let j = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize % (k + 1);
                    let (a, z) = (
                        rows[k].get(*i).cloned().flatten(),
                        rows[j].get(*i).cloned().flatten(),
                    );
                    if let Some(x) = rows[k].get_mut(*i) {
                        *x = z;
                    }
                    if let Some(x) = rows[j].get_mut(*i) {
                        *x = a;
                    }
                }
                continue;
            }
            for row in rows.iter_mut() {
                if let Some(cell) = row.get_mut(*i) {
                    *cell = self.value(rule, cell.as_deref());
                }
            }
        }
    }

    /// Masks the named top-level fields of MongoDB documents (strings, numbers and dates by their text; null stays).
    pub fn documents(&self, docs: &mut [bson::Document]) {
        for (_, rule) in &self.rules {
            for d in docs.iter_mut() {
                let Some(v) = d.get(&rule.column) else {
                    continue;
                };
                let text = match v {
                    bson::Bson::String(s) => Some(s.clone()),
                    bson::Bson::Null => None,
                    other => Some(other.to_string()),
                };
                let new = match rule.kind {
                    MaskKind::Null => bson::Bson::Null,
                    MaskKind::Shuffle | MaskKind::Keep => continue,
                    _ => self
                        .value(rule, text.as_deref())
                        .map_or(bson::Bson::Null, bson::Bson::String),
                };
                d.insert(rule.column.clone(), new);
            }
        }
    }
}

/// A new random key for a run that isn't a saved job.
pub fn new_key() -> [u8; 32] {
    use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    k
}

/// `YYYY-MM-DD…` moved by `days` (what follows the date is kept).
fn shift_date(v: &str, days: i64) -> Option<String> {
    let d = v.get(..10)?;
    let p: Vec<&str> = d.split('-').collect();
    if p.len() != 3 || p[0].len() != 4 {
        return None;
    }
    let (y, m, dd): (i64, i64, i64) = (p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?);
    // Days from the civil date and back (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let z = era * 146_097 + doe + days;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let nd = doy - (153 * mp + 2) / 5 + 1;
    let nm = if mp < 10 { mp + 3 } else { mp - 9 };
    let ny = yoe + era * 400 + i64::from(nm <= 2);
    Some(format!("{ny:04}-{nm:02}-{nd:02}{}", &v[10..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(column: &str, kind: MaskKind, value: Option<&str>) -> MaskRule {
        MaskRule {
            column: column.into(),
            kind,
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn rules_are_keyed_shaped_and_joins_survive() {
        let cols: Vec<String> = [
            "id", "email", "phone", "name", "born", "note", "card", "iban", "city",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let rules = vec![
            rule("id", MaskKind::Hash, None),
            rule("email", MaskKind::FakeEmail, None),
            rule("phone", MaskKind::FakePhone, None),
            rule("name", MaskKind::FakeName, None),
            rule("born", MaskKind::DateShift, Some("10")),
            rule("note", MaskKind::Truncate, Some("3")),
            rule("card", MaskKind::Null, None),
            rule("iban", MaskKind::FakeIban, None),
            rule("city", MaskKind::Fixed, Some("Nowhere")),
        ];
        let m = Masker::new([7; 32], &rules, &cols).unwrap();
        let row = |id: &str| {
            vec![
                Some(id.into()),
                Some("ada@real.org".into()),
                Some("+44 (20) 7946-0018".into()),
                Some("Ada Lovelace".into()),
                Some("1815-12-10 08:00:00".into()),
                Some("secret note".into()),
                Some("4111111111111111".into()),
                Some("GB82WEST12345698765432".into()),
                Some("London".into()),
            ]
        };
        let mut rows = vec![row("1042"), row("1042"), vec![None; 9]];
        m.batch(&mut rows);
        let r = &rows[0];
        assert_eq!(rows[0], rows[1], "the same input gives the same output");
        assert_ne!(r[0].as_deref(), Some("1042"));
        assert!(
            r[0].as_deref().unwrap().chars().all(|c| c.is_ascii_digit())
                && r[0].as_ref().unwrap().len() == 4,
            "{r:?}"
        );
        assert!(r[1].as_deref().unwrap().ends_with("@example.test"));
        let phone = r[2].as_deref().unwrap();
        assert!(
            phone.starts_with("+")
                && phone.contains(" (")
                && phone.len() == 18
                && phone != "+44 (20) 7946-0018",
            "{phone}"
        );
        assert!(FIRST
            .iter()
            .any(|f| r[3].as_deref().unwrap().starts_with(f)));
        let born = r[4].as_deref().unwrap();
        assert!(
            born.ends_with(" 08:00:00")
                && born.starts_with("1815-1")
                && born != "1815-12-10 08:00:00",
            "{born}"
        );
        assert_eq!(r[5].as_deref(), Some("sec"));
        assert_eq!(r[6], None);
        assert!(r[7].as_deref().unwrap().starts_with("GB") && r[7].as_ref().unwrap().len() == 22);
        assert_eq!(r[8].as_deref(), Some("Nowhere"));
        assert!(
            rows[2].iter().take(8).all(Option::is_none),
            "NULL stays NULL: {:?}",
            rows[2]
        );
        // Another key, another result: nothing guessable from the output alone.
        let other = Masker::new([8; 32], &rules, &cols).unwrap();
        let mut again = vec![row("1042")];
        other.batch(&mut again);
        assert_ne!(again[0][0], rows[0][0]);
    }

    #[test]
    fn shuffle_keeps_the_values_and_suggestions_find_personal_columns() {
        let cols = vec!["x".to_string()];
        let m = Masker::new([1; 32], &[rule("x", MaskKind::Shuffle, None)], &cols).unwrap();
        let mut rows: Vec<Vec<Option<String>>> =
            (0..20).map(|i| vec![Some(i.to_string())]).collect();
        m.batch(&mut rows);
        let mut got: Vec<i32> = rows
            .iter()
            .map(|r| r[0].as_deref().unwrap().parse().unwrap())
            .collect();
        assert_ne!(got, (0..20).collect::<Vec<_>>());
        got.sort();
        assert_eq!(got, (0..20).collect::<Vec<_>>());
        assert_eq!(suggest("customer_email"), Some(MaskKind::FakeEmail));
        assert_eq!(suggest("Mobile"), Some(MaskKind::FakePhone));
        assert_eq!(suggest("date_of_birth"), Some(MaskKind::DateShift));
        assert_eq!(suggest("name"), Some(MaskKind::FakeName));
        assert_eq!(suggest("total"), None);
        assert_eq!(shift_date("2024-03-01", -1).as_deref(), Some("2024-02-29"));
        assert_eq!(
            shift_date("2023-12-31T10:00", 1).as_deref(),
            Some("2024-01-01T10:00")
        );
        assert!(Masker::new([0; 32], &[rule("x", MaskKind::Keep, None)], &cols).is_none());
    }
}
