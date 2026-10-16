//! dono: C7
//! Motor de transição de borda: entrada proporcional, parede, empurrão/modificador, zona morta,
//! retorno no receptor, pulo direto e trava. Stubs do F2: assinaturas.

use std::time::Instant;

use crate::types::{DisplayInfo, LayoutProfile, Modifier, TransitionConfig};

#[derive(Debug, Clone, PartialEq)]
pub enum LocalDecision {
    Stay,
    Enter {
        display_id: String,
        entry_px: (i32, i32),
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoteDecision {
    Stay,
    Return {
        mac_display: String,
        point_px: (i32, i32),
    },
}

#[derive(Debug)]
pub struct TransitionEngine {
    _config: TransitionConfig,
}

impl TransitionEngine {
    pub fn new(
        config: TransitionConfig,
        _profile: LayoutProfile,
        _displays: Vec<DisplayInfo>,
    ) -> Self {
        Self { _config: config }
    }

    /// Movimento local no Mac. Sem alocação no caminho `Stay`.
    pub fn on_local_move(
        &mut self,
        _display_id: &str,
        _pos_px: (i32, i32),
        _delta: (i32, i32),
        _modifiers: &[Modifier],
        _now: Instant,
    ) -> LocalDecision {
        LocalDecision::Stay
    }

    /// Lado receptor: posição do cursor no desktop virtual do Windows.
    pub fn on_remote_cursor(&mut self, _pos_px: (i32, i32), _now: Instant) -> RemoteDecision {
        RemoteDecision::Stay
    }

    /// Centro da tela de número `n` (1-based, D19).
    pub fn jump_to(&self, _n: u8) -> Option<LocalDecision> {
        None
    }

    pub fn set_locked(&mut self, _locked: bool) {}
}
