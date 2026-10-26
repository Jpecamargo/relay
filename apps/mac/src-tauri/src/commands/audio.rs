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
    mic_to_peer: bool,
) -> Result<(), String> {
    tracing::info!(mic_to_peer, "comando set_audio_streams");
    state.backend.set_audio_streams(mic_to_peer)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_audio_devices(
    state: State<'_, AppState>,
    input_id: Option<String>,
) -> Result<(), String> {
    state.backend.set_audio_devices(input_id)
}
