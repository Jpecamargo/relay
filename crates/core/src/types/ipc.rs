//! dono: F2
//! Payloads de comandos Tauri e eventos core→UI que não são tipos de domínio.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::command::AudioDevice;
use super::display::{DisplayInfo, LayoutWarning};

/// Payload de `relay:displays` e retorno de `get_displays`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplaysPayload {
    pub local: Vec<DisplayInfo>,
    pub peer: Vec<DisplayInfo>,
}

/// Retorno de `save_layout`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SaveLayoutResult {
    pub warnings: Vec<LayoutWarning>,
}

/// Retorno de `list_audio_devices`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioDevices {
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
}

/// Payload de `relay:secure-input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecureInputPayload {
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase")]
pub enum PairingState {
    Code,
    Paired,
    Failed,
}

/// Payload de `relay:pairing`. `expires_at`: epoch em milissegundos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingEvent {
    pub state: PairingState,
    pub code: Option<String>,
    #[ts(type = "number | null")]
    pub expires_at: Option<u64>,
}

/// Retorno de `new_pairing_code`. `expires_at`: epoch em milissegundos (120 s de validade).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewPairingCode {
    pub code: String,
    #[ts(type = "number")]
    pub expires_at: u64,
}

/// Retorno de `get_service_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub helper_active: bool,
    /// VB-CABLE ("CABLE Input") instalado, detectado pelos endpoints de áudio.
    pub vbcable_detected: bool,
}

/// Payload de `relay:update-available`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateAvailable {
    pub version: String,
}
