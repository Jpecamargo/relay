//! dono: WA1
use relay_core::types::{Host, NewPairingCode};
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn new_pairing_code(state: State<'_, AgentState>) -> Result<NewPairingCode, String> {
    state.backend.new_pairing_code().map_err(|e| e.to_string())
}

/// Este PC disca com o código do outro lado; `layout_host` omitido vale este computador (D44).
#[tauri::command(rename_all = "snake_case")]
pub async fn pair_with_code(
    state: State<'_, AgentState>,
    code: String,
    address: Option<String>,
    layout_host: Option<Host>,
) -> Result<(), String> {
    // O pareamento espera a rede: sai da thread do comando para não travar a UI.
    let backend = state.backend.clone();
    tauri::async_runtime::spawn_blocking(move || {
        backend
            .pair_with_code(code, address, layout_host.unwrap_or(Host::Local))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(rename_all = "snake_case")]
pub fn unpair(state: State<'_, AgentState>) -> Result<(), String> {
    state.backend.unpair().map_err(|e| e.to_string())
}
