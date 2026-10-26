//! dono: MI1 (casco: MAC1)
use relay_core::types::{DiscoveredPeer, ServiceStatus, StatusSnapshot};
use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_status(state: State<'_, AppState>) -> StatusSnapshot {
    state.backend.get_status()
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_discovered(state: State<'_, AppState>) -> Vec<DiscoveredPeer> {
    state.backend.list_discovered()
}

/// O Mac não tem serviço do sistema: devolve o padrão (tudo falso).
#[tauri::command(rename_all = "snake_case")]
pub fn get_service_status() -> ServiceStatus {
    ServiceStatus::default()
}
