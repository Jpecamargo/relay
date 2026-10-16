//! dono: MI3 (casco: MAC1)
use tauri::{AppHandle, State};

use crate::events::emit_status;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn clipboard_send_now(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.clipboard_send_now()
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_clipboard_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    state.backend.set_clipboard_sync(enabled)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn receive_remote_files(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.receive_remote_files()
}
