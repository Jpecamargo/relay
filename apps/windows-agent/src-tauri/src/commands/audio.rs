//! Áudio: o PC lista os próprios dispositivos; não é fonte de microfone na 0.3 (D45).
use relay_core::types::AudioDevices;

use super::controls::unsupported;

#[tauri::command(rename_all = "snake_case")]
pub fn list_audio_devices() -> AudioDevices {
    relay_platform_windows::audio_endpoints::list_all().unwrap_or_default()
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_audio_streams(_mic_to_peer: bool) -> Result<(), String> {
    Err(unsupported())
}

/// A escolha de entrada vale para quem é fonte de microfone (o Mac); no PC não há o que gravar.
#[tauri::command(rename_all = "snake_case")]
pub fn set_audio_devices(_input_id: Option<String>) {}
