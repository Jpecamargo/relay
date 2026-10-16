//! dono: MI2 (casco: MAC1)
use relay_core::types::{CommandResult, RemoteCommand};
use tauri::{AppHandle, State};

use crate::events::emit_status;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn send_command(
    app: AppHandle,
    state: State<'_, AppState>,
    command: RemoteCommand,
) -> CommandResult {
    let result = state.backend.send_command(command);
    crate::events::emit_command_result(&app, &result);
    result
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_cursor_lock(
    app: AppHandle,
    state: State<'_, AppState>,
    locked: bool,
) -> Result<(), String> {
    state.backend.set_cursor_lock(locked)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_sharing_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<(), String> {
    state.backend.set_sharing_paused(paused)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn wake_pc(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.wake_pc()
}
