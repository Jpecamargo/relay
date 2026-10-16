//! dono: C5b
//! Detector de atalhos reservados e configurados (`HotkeyAction`). Stubs do F2: assinaturas.

use crate::types::{HotkeyAction, HotkeyBinding, KeyCode};

#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyOutcome {
    /// Não é atalho: segue para a tradução.
    Pass,
    /// Faz parte de um atalho: não aparece na saída. `Some` quando a combinação completou.
    Consumed(Option<HotkeyAction>),
}

#[derive(Debug)]
pub struct HotkeyMatcher {
    _bindings: Vec<HotkeyBinding>,
}

impl HotkeyMatcher {
    pub fn new(bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            _bindings: bindings,
        }
    }

    /// Alimenta um evento de tecla cru da captura.
    pub fn feed(&mut self, _code: KeyCode, _down: bool) -> HotkeyOutcome {
        HotkeyOutcome::Pass
    }
}
