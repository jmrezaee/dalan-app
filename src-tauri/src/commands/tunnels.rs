//! Port-forward commands (F06; S09, S10). Live state also arrives as `TunnelChanged` events.

use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult};
use crate::events::TunnelChanged;
use crate::ssh::{PromptAnswer, SessionEvent};
use crate::state::AppState;
use crate::tunnels::{self, Emit, ForwardInput, ForwardSummary};

fn emitter(app: AppHandle) -> Emit {
    Arc::new(move |s| {
        let _ = TunnelChanged {
            id: s.id,
            state: s.state,
            connections: s.connections,
            error: s.error,
        }
        .emit(&app);
    })
}

#[tauri::command]
#[specta::specta]
pub async fn forwards_list(state: State<'_, AppState>) -> AppResult<Vec<ForwardSummary>> {
    let rules = state
        .with_db_session(|c, s| tunnels::list_rules(c, &s.vault_id))
        .await?;
    Ok(rules
        .into_iter()
        .map(|(rule, host)| {
            let live = state.tunnels.status(&rule.id);
            ForwardSummary::new(rule, host, live)
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn forward_save(
    state: State<'_, AppState>,
    input: ForwardInput,
) -> AppResult<ForwardSummary> {
    if input
        .id
        .as_deref()
        .is_some_and(|id| state.tunnels.is_running(id))
    {
        return Err(AppError::validation("Stop the rule before changing it."));
    }
    let id = state
        .with_db_session(move |c, s| tunnels::save_rule(c, &s.vault_id, &input))
        .await?;
    let rules = state
        .with_db_session(|c, s| tunnels::list_rules(c, &s.vault_id))
        .await?;
    rules
        .into_iter()
        .find(|(r, _)| r.id == id)
        .map(|(rule, host)| ForwardSummary::new(rule, host, None))
        .ok_or_else(|| AppError::internal("saved rule is missing"))
}

#[tauri::command]
#[specta::specta]
pub async fn forward_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.tunnels.stop(&id);
    state
        .with_db_session(move |c, s| tunnels::delete_rule(c, &s.vault_id, &id))
        .await
}

/// Starts a rule. Host-key and password questions arrive on `events` (answer with `forward_respond`).
#[tauri::command]
#[specta::specta]
pub async fn forward_start(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    events: Channel<SessionEvent>,
) -> AppResult<()> {
    state.require_unlocked()?;
    let rid = id.clone();
    let rule = state
        .with_db_session(move |c, s| tunnels::get_rule(c, &s.vault_id, &rid))
        .await?;
    let user: tunnels::UserEvents = Box::new(move |e| {
        let _ = events.send(e);
    });
    state
        .tunnels
        .start(state.store(), rule, Some(user), emitter(app))
}

#[tauri::command]
#[specta::specta]
pub fn forward_stop(state: State<'_, AppState>, id: String) {
    state.tunnels.stop(&id);
}

#[tauri::command]
#[specta::specta]
pub fn forward_respond(
    state: State<'_, AppState>,
    id: String,
    prompt_id: String,
    answer: PromptAnswer,
) -> AppResult<()> {
    state.tunnels.respond(&id, &prompt_id, answer)
}

/// After unlock: start every rule marked "Start when Dalan unlocks". Nobody can answer questions
/// at that moment, so a rule that needs a new host key or password reports an error instead.
pub async fn autostart(app: &AppHandle, state: &AppState) {
    let Ok(rules) = state
        .with_db_session(|c, s| tunnels::autostart_rules(c, &s.vault_id))
        .await
    else {
        return;
    };
    for rule in rules {
        let _ = state
            .tunnels
            .start(state.store(), rule, None, emitter(app.clone()));
    }
}
