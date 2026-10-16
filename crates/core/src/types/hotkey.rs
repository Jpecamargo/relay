//! dono: F2
//! Combinações de teclas e ações de atalho.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::command::RemoteCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Modifier {
    Ctrl,
    Option,
    Shift,
    Cmd,
}

/// Combinação de teclas. `key` segue `KeyboardEvent.code` do W3C ("Digit1", "KeyL", "F5"),
/// neutro de plataforma e direto do gravador de atalhos da UI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeyCombo {
    pub modifiers: Vec<Modifier>,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum HotkeyAction {
    /// Pula para a tela de número `n` (1 = MacBook; D19).
    JumpTo(u8),
    ToggleLock,
    TogglePause,
    Command(RemoteCommand),
    ClipboardSendNow,
    WakePc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HotkeyBinding {
    pub combo: KeyCombo,
    pub action: HotkeyAction,
}
