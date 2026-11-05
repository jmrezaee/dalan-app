//! Shared app state. Managed by Tauri and handed to commands as `State<'_, AppState>`.

use std::sync::{
    atomic::{AtomicBool, AtomicI64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::vault::VaultSession;

const MAX_FAILURES: u32 = 5;
const COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Attempts {
    failures: u32,
    cooldown_until: Option<Instant>,
}

pub struct AppState {
    // Arc so a clone can move into spawn_blocking ('static). Mutex because rusqlite
    // connections are not Sync: one writer at a time, as architecture.md decides.
    pub db: Arc<Mutex<Connection>>,
    /// `Some` while unlocked. Dropping it wipes the key (see `Zeroizing`).
    session: Mutex<Option<VaultSession>>,
    attempts: Mutex<Attempts>,
    /// Last user activity, epoch ms. Drives auto-lock.
    last_activity: AtomicI64,
    /// True once the "locking soon" warning went out for the current idle stretch.
    pub warned: AtomicBool,
}

impl AppState {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
            session: Mutex::new(None),
            attempts: Mutex::new(Attempts::default()),
            last_activity: AtomicI64::new(now_ms()),
            warned: AtomicBool::new(false),
        }
    }

    /// Run blocking DB work off the async runtime.
    pub async fn with_db<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let mut conn = db
                .lock()
                .map_err(|_| AppError::internal("database lock poisoned"))?;
            f(&mut conn)
        })
        .await?
    }

    pub fn is_unlocked(&self) -> bool {
        self.session.lock().map(|s| s.is_some()).unwrap_or(false)
    }

    pub fn set_session(&self, s: Option<VaultSession>) {
        if let Ok(mut guard) = self.session.lock() {
            *guard = s; // the old value (if any) is dropped here and its key zeroized
        }
        self.touch();
    }

    /// The guard every command except vault_status/create/unlock goes through.
    pub fn require_unlocked(&self) -> AppResult<()> {
        if self.is_unlocked() {
            Ok(())
        } else {
            Err(AppError::locked())
        }
    }

    /// Borrow the key for the duration of `f`. The key never leaves Rust.
    pub fn with_session<T>(&self, f: impl FnOnce(&VaultSession) -> AppResult<T>) -> AppResult<T> {
        let guard = self
            .session
            .lock()
            .map_err(|_| AppError::internal("vault lock poisoned"))?;
        f(guard.as_ref().ok_or_else(AppError::locked)?)
    }

    pub fn touch(&self) {
        self.last_activity.store(now_ms(), Ordering::Relaxed);
        self.warned.store(false, Ordering::Relaxed);
    }

    pub fn idle_ms(&self) -> i64 {
        now_ms() - self.last_activity.load(Ordering::Relaxed)
    }

    /// Seconds left in a wrong-password cooldown, if one is active.
    pub fn cooldown_secs(&self) -> Option<u64> {
        let a = self.attempts.lock().ok()?;
        let left = a.cooldown_until?.checked_duration_since(Instant::now())?;
        Some(left.as_secs() + 1)
    }

    /// Call before trying a password. Errors while cooling down.
    pub fn check_cooldown(&self) -> AppResult<()> {
        match self.cooldown_secs() {
            Some(s) => Err(AppError::new(
                ErrorCode::TooManyAttempts,
                format!("Too many tries. Wait {s} seconds."),
            )),
            None => Ok(()),
        }
    }

    pub fn record_failure(&self) {
        if let Ok(mut a) = self.attempts.lock() {
            a.failures += 1;
            if a.failures >= MAX_FAILURES {
                a.failures = 0;
                a.cooldown_until = Some(Instant::now() + COOLDOWN);
            }
        }
    }

    pub fn reset_failures(&self) {
        if let Ok(mut a) = self.attempts.lock() {
            *a = Attempts::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn five_failures_start_a_cooldown_and_success_clears_it() {
        let s = AppState::new(open_in_memory().unwrap());
        for _ in 0..4 {
            s.record_failure();
            assert!(s.check_cooldown().is_ok());
        }
        s.record_failure();
        assert_eq!(
            s.check_cooldown().err().unwrap().code,
            ErrorCode::TooManyAttempts
        );
        s.reset_failures();
        assert!(s.check_cooldown().is_ok());
    }

    #[test]
    fn require_unlocked_is_locked_by_default() {
        let s = AppState::new(open_in_memory().unwrap());
        assert_eq!(s.require_unlocked().err().unwrap().code, ErrorCode::Locked);
    }
}
