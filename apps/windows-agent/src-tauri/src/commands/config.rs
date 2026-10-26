//! dono: WA1
use relay_core::types::Config;
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_config(state: State<'_, AgentState>) -> Config {
    state.backend.config()
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_config(state: State<'_, AgentState>, config: Config) -> Result<(), String> {
    state.backend.save_config(config).map_err(|e| e.to_string())
}

/// Liga/desliga a sincronização de clipboard: grava no Config (o núcleo republica o status).
#[tauri::command(rename_all = "snake_case")]
pub fn set_clipboard_sync(state: State<'_, AgentState>, enabled: bool) -> Result<(), String> {
    let mut config = state.backend.config();
    config.clipboard.enabled = enabled;
    state.backend.save_config(config).map_err(|e| e.to_string())
}

/// O PC não escolhe o endereço do Mac: é o Mac que disca. Aceita e ignora.
#[tauri::command(rename_all = "snake_case")]
pub fn set_manual_address(_address: Option<String>) {}
