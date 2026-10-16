//! dono: MI3 (casco: MAC1)
use relay_core::types::AudioDevices;
use tauri::{AppHandle, State};

use crate::events::emit_status;
use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn list_audio_devices(state: State<'_, AppState>) -> AudioDevices {
    state.backend.list_audio_devices()
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_audio_streams(
    app: AppHandle,
    state: State<'_, AppState>,
    pc_to_mac: bool,
    mac_to_pc: bool,
) -> Result<(), String> {
    state.backend.set_audio_streams(pc_to_mac, mac_to_pc)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_audio_devices(
    state: State<'_, AppState>,
    output_id: Option<String>,
    input_id: Option<String>,
) -> Result<(), String> {
    state.backend.set_audio_devices(output_id, input_id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_stream_volume(state: State<'_, AppState>, volume: f32) -> Result<(), String> {
    state.backend.set_stream_volume(volume)
}
