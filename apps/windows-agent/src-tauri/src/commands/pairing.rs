//! dono: WA1
use relay_core::types::NewPairingCode;
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn agent_new_pairing_code(state: State<'_, AgentState>) -> Result<NewPairingCode, String> {
    state.backend.new_pairing_code().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn agent_revoke_pair(state: State<'_, AgentState>) -> Result<(), String> {
    state.backend.revoke_pair().map_err(|e| e.to_string())
}
