//! Shared app state. Managed by Tauri and handed to commands as `State<'_, AppState>`.

use std::sync::{atomic::AtomicBool, Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub struct AppState {
    // Arc so a clone can move into spawn_blocking ('static). Mutex because rusqlite
    // connections are not Sync: one writer at a time, as architecture.md decides.
    pub db: Arc<Mutex<Connection>>,
    /// True while the vault key is held in memory (set by vault_unlock in P2).
    pub unlocked: AtomicBool,
}

impl AppState {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
            unlocked: AtomicBool::new(false),
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
}
