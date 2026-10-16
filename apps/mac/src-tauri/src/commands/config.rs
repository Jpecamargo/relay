//! dono: MI1 (casco: MAC1)
use relay_core::types::Config;
use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.backend.get_config()
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_config(state: State<'_, AppState>, config: Config) -> Result<(), String> {
    state.backend.save_config(config)
}
