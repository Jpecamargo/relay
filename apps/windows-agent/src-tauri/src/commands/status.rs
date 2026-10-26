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

/// Pares vistos pelo mDNS: o PC também busca (D41), e a tela "Digitar código" usa a lista.
#[tauri::command(rename_all = "snake_case")]
pub fn list_discovered(state: State<'_, AgentState>) -> Vec<DiscoveredPeer> {
    state.backend.list_discovered()
}
