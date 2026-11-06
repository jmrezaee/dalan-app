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
    TooManyAttempts,
    /// Importing an encrypted key without its passphrase.
    NeedsPassphrase,
    /// The text isn't a private key we can read.
    InvalidKey,
    NotFound,
    Validation,
    InUse,
    AuthFailed,
    HostUnreachable,
    HostKeyChanged,
    KexFailed,
    PortInUse,
    PermissionDenied,
    /// The server doesn't offer the SFTP subsystem.
    SftpDisabled,
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
    pub fn wrong_password() -> Self {
        Self::new(ErrorCode::WrongPassword, "That password didn't work.")
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

    /// Hard rule 7: every code serialises to exactly `{ code, message }`, and `ErrorCode` can't grow without this list.
    #[test]
    fn every_error_code_serialises_to_exactly_code_and_message() {
        use ErrorCode::*;
        let all = [
            Locked,
            WrongPassword,
            TooManyAttempts,
            NeedsPassphrase,
            InvalidKey,
            NotFound,
            Validation,
            InUse,
            AuthFailed,
            HostUnreachable,
            HostKeyChanged,
            KexFailed,
            PortInUse,
            PermissionDenied,
            SftpDisabled,
            Internal,
        ];
        // An exhaustive match: adding a variant is a compile error here until the list above has it.
        for c in all {
            match c {
                Locked | WrongPassword | TooManyAttempts | NeedsPassphrase | InvalidKey
                | NotFound | Validation | InUse | AuthFailed | HostUnreachable | HostKeyChanged
                | KexFailed | PortInUse | PermissionDenied | SftpDisabled | Internal => {}
            }
            let json = serde_json::to_value(AppError::new(c, "m")).unwrap();
            let obj = json.as_object().unwrap();
            assert_eq!(obj.len(), 2, "{c:?}");
            assert!(obj["code"].is_string() && obj["message"] == "m", "{c:?}");
        }
    }

    #[test]
    fn serialises_to_code_and_message() {
        let json = serde_json::to_value(AppError::new(ErrorCode::HostKeyChanged, "x")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "host_key_changed", "message": "x" })
        );
    }
}
