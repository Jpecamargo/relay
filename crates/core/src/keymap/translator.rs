//! dono: C5b
//! Tradutor de atalhos Mac→Windows (tabela do spec, regra geral, Cmd tocado, exceções por app,
//! overrides do config). Stubs do F2: assinaturas.

use arrayvec::ArrayVec;

use crate::types::KeymapConfig;

/// Capacidade da saída de uma tradução.
pub const MAX_TRANSLATED: usize = 8;

/// Evento de tecla já em scan code do Windows (Cmd chega como 0xE05B/0xE05C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub scan: u16,
    pub down: bool,
    pub autorepeat: bool,
}

#[derive(Debug)]
pub struct KeymapTranslator {
    _config: KeymapConfig,
}

impl KeymapTranslator {
    pub fn new(config: KeymapConfig) -> Self {
        Self { _config: config }
    }

    /// Traduz `event` para a sequência de eventos a injetar no destino. `foreground_exe` é o
    /// app em primeiro plano no Windows. Sem alocação.
    pub fn translate(
        &mut self,
        event: KeyEvent,
        _foreground_exe: Option<&str>,
        out: &mut ArrayVec<KeyEvent, MAX_TRANSLATED>,
    ) {
        out.push(event);
    }
}
