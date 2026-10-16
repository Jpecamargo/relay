//! dono: C10
//! Motor de papéis, genérico sobre as traits de `platform`: Controller (captura → saída),
//! Receiver (entrada → injeção) e CommandDispatcher.
//!
//! Nenhum dos três é `Sync` nem abre thread: o AppCore/AgentCore serializa as chamadas
//! (`handle`/`on_message`/`tick`) numa thread só por papel e injeta o relógio quando precisa de
//! testes determinísticos (`*_at`). A rede entra por duas filas `rtrb` (canal rápido e canal de
//! controle) que o chamador esvazia para a `Session`.

mod commands;
mod controller;
mod receiver;
pub mod reverse;
/// Mocks de plataforma para testes (próprios e dos apps, via feature `test-util`).
#[cfg(any(test, feature = "test-util"))]
pub mod testutil;

use crate::keymap::tables::mac_to_scan;
use crate::types::{DisplayInfo, HotkeyAction, KeyCode, LayoutProfile, ProfileKey};

pub use commands::{COMMAND_TIMEOUT, CommandDispatcher};
pub use controller::Controller;
pub use receiver::Receiver;

/// Notificação do motor para o AppCore/AgentCore (UI, clipboard, comandos).
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// O foco mudou de máquina (`remote=true`: agora está no Windows).
    FocusChanged {
        remote: bool,
    },
    /// Troca de foco com `clipboard.enabled` e `mode=on_switch`: o chamador lê o clipboard local e
    /// manda `ClipboardSync::on_switch(current)`.
    ClipboardSwitch,
    LockChanged(bool),
    PauseChanged(bool),
    /// Atalho que o motor não resolve sozinho: `Command`, `ClipboardSendNow`, `WakePc`.
    Hotkey(HotkeyAction),
}

pub type EventHandler = Box<dyn FnMut(EngineEvent) + Send>;

/// Scan code (set 1) de um código cru da captura; `None` para tecla sem mapeamento.
pub(crate) fn scan_of(code: KeyCode) -> Option<u16> {
    match code {
        KeyCode::Scan(s) => Some(s),
        KeyCode::Mac(m) => mac_to_scan(m),
    }
}

pub(crate) fn sat_i16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Telas locais + telas do par + perfil de layout: o que o `TransitionEngine` precisa.
#[derive(Debug)]
pub(crate) struct LayoutState {
    pub local: Vec<DisplayInfo>,
    pub peer: Vec<DisplayInfo>,
    pub profile: LayoutProfile,
}

impl LayoutState {
    pub fn new(profile: LayoutProfile, local: Vec<DisplayInfo>) -> Self {
        Self {
            local,
            peer: Vec::new(),
            profile,
        }
    }

    pub fn empty_profile() -> LayoutProfile {
        LayoutProfile {
            key: ProfileKey::default(),
            name: String::new(),
            displays: Vec::new(),
            windows_detached: false,
        }
    }

    pub fn combined(&self) -> Vec<DisplayInfo> {
        self.local.iter().chain(&self.peer).cloned().collect()
    }
}
