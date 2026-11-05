//! Keychain commands (F02; S05-S07). Private keys never leave Rust: summaries carry only the public part.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::keys::{
    self, GenerateInput, IdentityInput, IdentitySummary, ImportInput, KeySummary, KeyUpdate,
};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn keys_list(state: State<'_, AppState>) -> AppResult<Vec<KeySummary>> {
    state.with_db_session(|c, s| keys::list(c, s)).await
}

/// Generating (RSA especially) is CPU work, so it runs off the async threads and before the
/// database lock is taken.
#[tauri::command]
#[specta::specta]
pub async fn key_generate(
    state: State<'_, AppState>,
    input: GenerateInput,
) -> AppResult<KeySummary> {
    state.require_unlocked()?;
    let new = tokio::task::spawn_blocking(move || keys::generate(&input)).await??;
    state
        .with_db_session(move |c, s| keys::store(c, s, &new))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn key_import(state: State<'_, AppState>, input: ImportInput) -> AppResult<KeySummary> {
    state.require_unlocked()?;
    let new = tokio::task::spawn_blocking(move || keys::parse_import(&input)).await??;
    state
        .with_db_session(move |c, s| keys::store(c, s, &new))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn key_update(state: State<'_, AppState>, input: KeyUpdate) -> AppResult<KeySummary> {
    state
        .with_db_session(move |c, s| keys::update(c, s, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn key_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| keys::delete(c, s, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn identities_list(state: State<'_, AppState>) -> AppResult<Vec<IdentitySummary>> {
    state
        .with_db_session(|c, s| keys::identities_list(c, &s.vault_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn identity_save(
    state: State<'_, AppState>,
    input: IdentityInput,
) -> AppResult<IdentitySummary> {
    state
        .with_db_session(move |c, s| keys::identity_save(c, s, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn identity_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| keys::identity_delete(c, s, &id))
        .await
}

/// Attaches (or, with no text and no path, removes) an OpenSSH user certificate. A path chosen in
/// the dialog is read here in Rust.
#[tauri::command]
#[specta::specta]
pub async fn key_set_certificate(
    state: State<'_, AppState>,
    id: String,
    certificate: Option<String>,
    path: Option<String>,
) -> AppResult<KeySummary> {
    state
        .with_db_session(move |c, s| {
            let text = match (&certificate, &path) {
                (Some(t), _) if !t.trim().is_empty() => Some(t.clone()),
                (_, Some(p)) => {
                    let meta = std::fs::metadata(p)
                        .map_err(|_| AppError::not_found("Couldn't open that file."))?;
                    if meta.len() > 64 * 1024 {
                        return Err(AppError::validation(
                            "That file is too big to be a certificate.",
                        ));
                    }
                    Some(
                        std::fs::read_to_string(p)
                            .map_err(|_| AppError::validation("That file isn't text."))?,
                    )
                }
                _ => None,
            };
            keys::set_certificate(c, s, &id, text.as_deref())
        })
        .await
}
