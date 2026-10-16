//! dono: WA1
use relay_core::types::Config;
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn agent_get_config(state: State<'_, AgentState>) -> Config {
    state.backend.config()
}

#[tauri::command(rename_all = "snake_case")]
pub fn agent_save_config(state: State<'_, AgentState>, config: Config) -> Result<(), String> {
    state.backend.save_config(config).map_err(|e| e.to_string())
}
