//! dono: WA1
use relay_core::types::{DiscoveredPeer, ServiceStatus, StatusSnapshot};
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_status(state: State<'_, AgentState>) -> StatusSnapshot {
    state.backend.status()
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_service_status(state: State<'_, AgentState>) -> ServiceStatus {
    state.backend.service_status()
}

/// O agente é quem escuta (Listener): o Mac é quem descobre e disca, então não há lista aqui.
#[tauri::command(rename_all = "snake_case")]
pub fn list_discovered() -> Vec<DiscoveredPeer> {
    Vec::new()
}
