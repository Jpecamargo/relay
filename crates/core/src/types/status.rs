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

/// Como a versão deste app se compara à do par (ver `version::version_sync`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VersionSync {
    /// Iguais, ou o par não informa uma versão que se entenda.
    #[default]
    InSync,
    /// Este app é o mais antigo: precisa se atualizar.
    LocalBehind,
    /// O par é o mais antigo e já sabe receber o pedido de atualização.
    PeerBehind,
    /// O par é o mais antigo mas é velho demais para receber o pedido: a pessoa atualiza lá.
    PeerBehindManual,
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
    /// O PC é conhecido (mDNS ou endereço manual) mas o TCP de controle não responde há tempo
    /// demais: no Windows, em geral o firewall.
    pub pc_unreachable: bool,
    pub clipboard_sync: bool,
    pub audio: AudioState,
    /// Versão do app do par, só com a sessão de pé e se ele a informou.
    pub peer_version: Option<String>,
    pub version_sync: VersionSync,
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
    /// Volume atual da saída do PC, 0..1 (o painel inicializa o slider com ele).
    pub volume: Option<f32>,
    pub muted: Option<bool>,
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
