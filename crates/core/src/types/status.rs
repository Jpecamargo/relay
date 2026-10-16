//! dono: F2
//! Estado e telemetria publicados do core para a UI.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::display::Machine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ConnectionStatus {
    Disconnected,
    Discovering,
    Connecting,
    Pairing,
    Connected,
    Reconnecting,
    /// Versões de protocolo diferentes (D23).
    IncompatibleVersion {
        local: u16,
        remote: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioState {
    pub pc_to_mac: bool,
    pub mac_to_pc: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatusSnapshot {
    pub status: ConnectionStatus,
    pub peer_name: Option<String>,
    /// Máquina que recebe o input agora.
    pub focus: Machine,
    pub locked: bool,
    pub paused: bool,
    pub secure_input: bool,
    pub clipboard_sync: bool,
    pub audio: AudioState,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LatencyStats {
    pub rtt_ms: f32,
    pub jitter_ms: f32,
    pub loss_pct: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioStats {
    pub buffer_ms: f32,
    pub loss_pct: f32,
    pub latency_ms: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SystemMetrics {
    pub cpu_pct: f32,
    pub ram_used_mb: u32,
    pub ram_total_mb: u32,
    pub gpu_pct: Option<f32>,
    pub gpu_mem_mb: Option<u32>,
    pub gpu_temp_c: Option<f32>,
    pub cpu_temp_c: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TransferProgress {
    pub transfer_id: u32,
    pub file_name: String,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PermissionState {
    Granted,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PermissionsStatus {
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PermissionKind {
    Accessibility,
    InputMonitoring,
}
