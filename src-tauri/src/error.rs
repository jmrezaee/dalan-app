//! One error shape for every command: `{ code, message }`.
//! The UI maps `code` to friendly copy. `message` is for logs and fallbacks.

use serde::Serialize;
use specta::Type;

/// Codes from replica/architecture.md. Add new ones there first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Locked,
    WrongPassword,
    NotFound,
    Validation,
    InUse,
    AuthFailed,
    HostUnreachable,
    HostKeyChanged,
    KexFailed,
    PortInUse,
    PermissionDenied,
    Internal,
}

#[derive(Debug, Clone, Serialize, Type, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn locked() -> Self {
        Self::new(ErrorCode::Locked, "The vault is locked.")
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, what)
    }
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Validation, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, msg)
    }
}

// `?` on database errors becomes an `internal` AppError. Raw SQL text never reaches the UI as copy.
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(format!("database error: {e}"))
    }
}

impl From<rusqlite_migration::Error> for AppError {
    fn from(e: rusqlite_migration::Error) -> Self {
        Self::internal(format!("migration error: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::internal(format!("io error: {e}"))
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::internal(format!("background task failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_to_code_and_message() {
        let json = serde_json::to_value(AppError::new(ErrorCode::HostKeyChanged, "x")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "host_key_changed", "message": "x" })
        );
    }
}
