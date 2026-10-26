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

/// Endereço manual do Mac (`ip:porta`): alvo do pareamento quando o mDNS não acha o Mac.
#[tauri::command(rename_all = "snake_case")]
pub fn set_manual_address(
    state: State<'_, AgentState>,
    address: Option<String>,
) -> Result<(), String> {
    let address = address
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty());
    if let Some(a) = &address {
        relay_core::discovery::parse_manual_address(a).map_err(|e| e.to_string())?;
    }
    let mut config = state.backend.config();
    config.network.manual_address = address;
    state.backend.save_config(config).map_err(|e| e.to_string())
}
