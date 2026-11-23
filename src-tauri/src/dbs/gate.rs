//! The write gate shared by the SQL engines (spec section 3, "The gate for SQL"; MongoDB joins in D8). One decision
//! function for every statement, wherever it comes from: the editor, a grid commit, a structure change. The window
//! never decides. A statement that needs an answer is *held* here under an id bound to its connection; the id is
//! good once, for two minutes, and a dangerous one also needs the database's exact name typed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::store::RedisWriteMode as Mode;
use crate::sql::classify::Class;

const PENDING_FOR: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Show what it will do and ask.
    Confirm,
    /// Ask, and make the person type the database's name.
    Typed,
    Refuse,
}

/// What the database's mode says about one class of statement.
pub fn decide(mode: Mode, class: Class) -> Decision {
    match (class, mode) {
        (Class::Read, _) => Decision::Run,
        (_, Mode::ReadOnly) => Decision::Refuse,
        (Class::Write, Mode::Confirm) => Decision::Confirm,
        (Class::Write, Mode::Open) => Decision::Run,
        (Class::Schema, _) => Decision::Confirm,
        (Class::Dangerous, _) => Decision::Typed,
    }
}

/// The strongest decision of a script: refuse over typed over confirm over run.
pub fn strongest(a: Decision, b: Decision) -> Decision {
    fn rank(d: Decision) -> u8 {
        match d {
            Decision::Run => 0,
            Decision::Confirm => 1,
            Decision::Typed => 2,
            Decision::Refuse => 3,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

pub fn read_only_error(db_name: &str) -> AppError {
    AppError::new(
        ErrorCode::DbReadOnlyMode,
        format!("{db_name} is read-only in Dalan. Nothing was sent."),
    )
}

/// A held confirmation may run now: the mode is asked again (it may have changed since the dialog opened), and a
/// typed confirmation must carry the database's exact name.
pub fn approve(mode: Mode, class: Class, db_name: &str, typed_name: Option<&str>) -> AppResult<()> {
    match decide(mode, class) {
        Decision::Run | Decision::Confirm => Ok(()),
        Decision::Typed if typed_name == Some(db_name) => Ok(()),
        Decision::Typed => Err(AppError::new(
            ErrorCode::DbConfirmNeeded,
            "The name doesn't match the database's name. Nothing was sent.",
        )),
        Decision::Refuse => Err(read_only_error(db_name)),
    }
}

struct Held<T> {
    conn_id: String,
    value: T,
    class: Class,
    created: Instant,
}

/// What is kept between "prepare" and "commit", for every connection. An id from another tab is unknown.
pub struct PendingStore<T>(Mutex<HashMap<String, Held<T>>>);

impl<T> Default for PendingStore<T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

fn needed(message: &str) -> AppError {
    AppError::new(ErrorCode::DbConfirmNeeded, message)
}

impl<T> PendingStore<T> {
    pub fn put(&self, conn_id: &str, class: Class, value: T) -> AppResult<String> {
        let id = uuid::Uuid::now_v7().to_string();
        let now = Instant::now();
        let mut map = self
            .0
            .lock()
            .map_err(|_| AppError::internal("gate lock poisoned"))?;
        // Confirmations nobody answered are dropped here, so the map cannot grow without end.
        map.retain(|_, h| now.duration_since(h.created) < PENDING_FOR);
        map.insert(
            id.clone(),
            Held {
                conn_id: conn_id.to_string(),
                value,
                class,
                created: now,
            },
        );
        Ok(id)
    }

    /// The class of what is held, without using it up (the typed name is checked first; a typo keeps the dialog).
    pub fn peek_class(&self, conn_id: &str, id: &str) -> AppResult<Class> {
        let map = self
            .0
            .lock()
            .map_err(|_| AppError::internal("gate lock poisoned"))?;
        match map.get(id) {
            Some(h) if h.conn_id == conn_id && h.created.elapsed() < PENDING_FOR => Ok(h.class),
            _ => Err(needed(
                "That confirmation was already used or isn't for this tab. Nothing was sent.",
            )),
        }
    }

    /// Removes and returns what was held. Removing first is what makes a second commit fail.
    pub fn take(&self, conn_id: &str, id: &str) -> AppResult<(T, Class)> {
        let mut map = self
            .0
            .lock()
            .map_err(|_| AppError::internal("gate lock poisoned"))?;
        let held = match map.get(id) {
            Some(h) if h.conn_id == conn_id => map.remove(id),
            _ => None,
        }
        .ok_or_else(|| {
            needed("That confirmation was already used or isn't for this tab. Nothing was sent.")
        })?;
        if held.created.elapsed() >= PENDING_FOR {
            return Err(needed(
                "That confirmation is more than two minutes old. Nothing was sent. Try the change again.",
            ));
        }
        Ok((held.value, held.class))
    }

    /// A connection closed: what it held goes with it.
    pub fn drop_conn(&self, conn_id: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.retain(|_, h| h.conn_id != conn_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modes_decide_each_class() {
        use Decision::*;
        let table = [
            (Mode::ReadOnly, [Run, Refuse, Refuse, Refuse]),
            (Mode::Confirm, [Run, Confirm, Confirm, Typed]),
            (Mode::Open, [Run, Run, Confirm, Typed]),
        ];
        for (mode, want) in table {
            let got = [Class::Read, Class::Write, Class::Schema, Class::Dangerous]
                .map(|c| decide(mode, c));
            assert_eq!(got, want, "{mode:?}");
        }
        assert_eq!(strongest(Run, Typed), Typed);
        assert_eq!(strongest(Confirm, Refuse), Refuse);
    }

    #[test]
    fn a_typed_name_must_match_and_a_read_only_database_refuses_even_with_it() {
        assert!(approve(
            Mode::Confirm,
            Class::Dangerous,
            "app-prod",
            Some("app-prod")
        )
        .is_ok());
        assert!(approve(Mode::Confirm, Class::Dangerous, "app-prod", Some("app")).is_err());
        assert!(approve(Mode::Confirm, Class::Dangerous, "app-prod", None).is_err());
        assert!(approve(Mode::ReadOnly, Class::Write, "app-prod", Some("app-prod")).is_err());
        assert!(approve(Mode::Confirm, Class::Write, "app-prod", None).is_ok());
    }

    #[test]
    fn a_held_confirmation_is_good_once_and_only_for_its_own_connection() {
        let s: PendingStore<u8> = PendingStore::default();
        let id = s.put("c1", Class::Write, 7).unwrap();
        assert!(s.take("c2", &id).is_err());
        // The wrong connection did not use it up... it did not find it, and it is still there.
        assert_eq!(s.take("c1", &id).unwrap().0, 7);
        assert!(s.take("c1", &id).is_err());
        let id = s.put("c1", Class::Write, 8).unwrap();
        s.drop_conn("c1");
        assert!(s.take("c1", &id).is_err());
    }
}
