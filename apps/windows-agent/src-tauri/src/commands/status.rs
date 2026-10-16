//! dono: WA1
use relay_core::types::{ServiceStatus, StatusSnapshot};
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn agent_get_status(state: State<'_, AgentState>) -> StatusSnapshot {
    state.backend.status()
}

#[tauri::command(rename_all = "snake_case")]
pub fn agent_get_service_status(state: State<'_, AgentState>) -> ServiceStatus {
    state.backend.service_status()
}
