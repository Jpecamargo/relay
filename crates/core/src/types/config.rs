//! dono: F2
//! Estrutura do Config (a persistência em TOML é do C9). Nunca carrega chaves.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::clipboard::ClipboardFormat;
use super::consts::*;
use super::display::LayoutProfile;
use super::hotkey::{HotkeyAction, HotkeyBinding, KeyCombo, Modifier};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Config {
    pub schema_version: u32,
    pub network: NetworkConfig,
    pub transition: TransitionConfig,
    pub keymap: KeymapConfig,
    pub hotkeys: Vec<HotkeyBinding>,
    pub clipboard: ClipboardConfig,
    pub audio: AudioConfig,
    pub layout: LayoutConfig,
    pub general: GeneralConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            network: NetworkConfig::default(),
            transition: TransitionConfig::default(),
            keymap: KeymapConfig::default(),
            hotkeys: default_hotkeys(),
            clipboard: ClipboardConfig::default(),
            audio: AudioConfig::default(),
            layout: LayoutConfig::default(),
            general: GeneralConfig::default(),
        }
    }
}

/// Atalhos padrão do D19: Ctrl+Option+1/2/3 pulam para as telas, +L trava, +P pausa.
pub fn default_hotkeys() -> Vec<HotkeyBinding> {
    let combo = |key: &str| KeyCombo {
        modifiers: vec![Modifier::Ctrl, Modifier::Option],
        key: key.to_string(),
    };
    vec![
        HotkeyBinding {
            combo: combo("Digit1"),
            action: HotkeyAction::JumpTo(1),
        },
        HotkeyBinding {
            combo: combo("Digit2"),
            action: HotkeyAction::JumpTo(2),
        },
        HotkeyBinding {
            combo: combo("Digit3"),
            action: HotkeyAction::JumpTo(3),
        },
        HotkeyBinding {
            combo: combo("KeyL"),
            action: HotkeyAction::ToggleLock,
        },
        HotkeyBinding {
            combo: combo("KeyP"),
            action: HotkeyAction::TogglePause,
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Ports {
    pub control: u16,
    pub input: u16,
    pub audio_pc_to_mac: u16,
    pub audio_mac_to_pc: u16,
    pub transfer: u16,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            control: PORT_CONTROL,
            input: PORT_INPUT,
            audio_pc_to_mac: PORT_AUDIO_PC_TO_MAC,
            audio_mac_to_pc: PORT_AUDIO_MAC_TO_PC,
            transfer: PORT_TRANSFER,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct NetworkConfig {
    pub ports: Ports,
    /// Endereço manual do agente (plano B ao mDNS): IP, IP:porta ou hostname.
    pub manual_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct TransitionConfig {
    /// Teto de tempo empurrando a borda: passado isso, troca de máquina mesmo sem a pressão
    /// de `edge_resistance_pt` (0 = sem teto, só a pressão decide).
    pub dwell_ms: u32,
    /// Pressão (pt somados de movimento para fora da borda) para trocar de máquina (0 = qualquer
    /// empurrão). Movimento normal passa na hora; esbarrão lento não.
    pub edge_resistance_pt: f32,
    /// Modificador que precisa estar pressionado para trocar (`None` = não exige).
    pub require_modifier: Option<Modifier>,
    pub corner_dead_zone_mm: f32,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        Self {
            dwell_ms: 0,
            edge_resistance_pt: 8.0,
            require_modifier: None,
            corner_dead_zone_mm: 10.0,
        }
    }
}

/// Regra de atalho. `id` identifica uma regra embutida (`enabled=false` a desativa; `to`
/// preenchido a substitui). Com `id` novo e `from` + `to`, adiciona uma regra própria.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeymapRule {
    pub id: String,
    pub enabled: bool,
    pub from: Option<KeyCombo>,
    pub to: Option<KeyCombo>,
}

/// Regras próprias para um app em primeiro plano no Windows (`exe` comparado sem caixa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppException {
    pub exe: String,
    pub rules: Vec<KeymapRule>,
}

/// Tradução de modificadores no sentido inverso (D25): modificador do Windows → do Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct ReverseModifiers {
    pub ctrl: Modifier,
    pub win: Modifier,
    pub alt: Modifier,
}

impl Default for ReverseModifiers {
    fn default() -> Self {
        Self {
            ctrl: Modifier::Cmd,
            win: Modifier::Ctrl,
            alt: Modifier::Option,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct KeymapConfig {
    pub cmd_tap_timeout_ms: u32,
    pub rules: Vec<KeymapRule>,
    pub app_exceptions: Vec<AppException>,
    pub reverse_modifiers: ReverseModifiers,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            cmd_tap_timeout_ms: 400,
            rules: Vec::new(),
            app_exceptions: Vec::new(),
            reverse_modifiers: ReverseModifiers::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename_all = "snake_case")]
pub enum ClipboardMode {
    OnSwitch,
    OnCopy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct ClipboardConfig {
    /// Sincronização ligada (espelho de `set_clipboard_sync`).
    pub enabled: bool,
    pub mode: ClipboardMode,
    #[ts(type = "number")]
    pub max_bytes: u64,
    pub formats: Vec<ClipboardFormat>,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: ClipboardMode::OnSwitch,
            max_bytes: 50 * 1024 * 1024,
            formats: vec![
                ClipboardFormat::Text,
                ClipboardFormat::Html,
                ClipboardFormat::Rtf,
                ClipboardFormat::Png,
                ClipboardFormat::FileList,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename_all = "snake_case")]
pub enum PcAudioMode {
    /// Loopback por processo (padrão, D14).
    Process,
    /// Troca a saída padrão para a saída virtual configurada.
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct AudioConfig {
    pub pc_mode: PcAudioMode,
    pub process_names: Vec<String>,
    pub virtual_output_name: Option<String>,
    pub jitter_target_ms: u32,
    pub mac_output_id: Option<String>,
    pub mac_input_id: Option<String>,
    /// Volume do stream do PC no Mac, 0.0..=1.0.
    pub stream_volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            pc_mode: PcAudioMode::Process,
            process_names: vec!["Discord.exe".to_string()],
            virtual_output_name: None,
            jitter_target_ms: 20,
            mac_output_id: None,
            mac_input_id: None,
            stream_volume: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct LayoutConfig {
    pub profiles: Vec<LayoutProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GeneralConfig {
    pub launch_at_login: bool,
}
