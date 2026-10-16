//! dono: F2
//! Comandos remotos (Mac → Windows) e seus resultados.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::status::SystemMetrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MediaAction {
    PlayPause,
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PowerAction {
    Lock,
    Suspend,
    Restart,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum RemoteCommand {
    SetVolume {
        level: f32,
    },
    SetMute {
        muted: bool,
    },
    ListOutputDevices,
    SetOutputDevice {
        id: String,
    },
    Media {
        action: MediaAction,
    },
    /// `Restart` e `Shutdown` exigem `confirm: true` (senão `confirmation_required`).
    Power {
        action: PowerAction,
        confirm: bool,
    },
    CaptureScreen {
        display_id: String,
    },
    OpenTarget {
        target: String,
    },
    TypeText {
        text: String,
    },
    GetMetrics,
}

/// Dispositivo de áudio (endpoint de saída ou entrada).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CommandOutput {
    Devices(Vec<AudioDevice>),
    Metrics(SystemMetrics),
    Text(String),
    /// Resultado de `CaptureScreen` dentro do executor. Nunca vai no fio dentro de um
    /// `CommandResult`: o agente o converte em `ClipboardOffer` (Png) e responde `output: None`.
    Png(Vec<u8>),
}

/// Códigos de erro fixos de `CommandResult::error`.
pub mod error_code {
    pub const TIMEOUT: &str = "timeout";
    pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
    pub const VBCABLE_MISSING: &str = "vbcable_missing";
    pub const VIRTUAL_OUTPUT_MISSING: &str = "virtual_output_missing";
    pub const UNSUPPORTED: &str = "unsupported";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandResult {
    pub id: u32,
    pub ok: bool,
    pub output: Option<CommandOutput>,
    /// Um dos códigos de [`error_code`] ou texto livre.
    pub error: Option<String>,
}

impl CommandResult {
    pub fn ok(id: u32, output: Option<CommandOutput>) -> Self {
        Self {
            id,
            ok: true,
            output,
            error: None,
        }
    }

    pub fn err(id: u32, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            output: None,
            error: Some(error.into()),
        }
    }
}
