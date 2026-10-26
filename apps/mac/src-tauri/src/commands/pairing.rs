//! dono: MI1 (casco: MAC1)
use relay_core::types::{Host, NewPairingCode};
use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub async fn pair_with_code(
    state: State<'_, AppState>,
    code: String,
    address: Option<String>,
    // D44: o host do layout fica guardado até o layout o aplicar depois do pareamento.
    layout_host: Option<Host>,
) -> Result<(), String> {
    let backend = state.backend.clone();
    // Sem escolha, o padrão é este computador (D44).
    backend.set_pending_layout_host(Some(layout_host.unwrap_or(Host::Local)));
    backend.pair_with_code(code, address).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn new_pairing_code(state: State<'_, AppState>) -> Result<NewPairingCode, String> {
    state.backend.new_pairing_code()
}

#[tauri::command(rename_all = "snake_case")]
pub fn unpair(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.backend.unpair()?;
    crate::events::emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_manual_address(
    state: State<'_, AppState>,
    address: Option<String>,
) -> Result<(), String> {
    state.backend.set_manual_address(address)
}
