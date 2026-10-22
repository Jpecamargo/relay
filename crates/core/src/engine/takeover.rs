//! Quem mexeu por último manda: input físico na máquina que está sendo controlada pede o controle
//! de volta (`Message::TakeControl`), sem esperar o outro lado devolver o foco pela borda. As
//! capturas já descartam o que o próprio Relay injeta, então todo evento que chega aqui é físico.

use std::time::{Duration, Instant};

use crate::types::{CapturedEvent, ScrollPhase};

/// Movimento acumulado que conta como intenção (tremida de mouse parado não conta).
const MOVE_PX: i32 = 8;
/// Pausa no movimento que zera o acumulado.
const MOVE_GAP: Duration = Duration::from_millis(250);
/// Intervalo entre pedidos enquanto o outro lado não confirma.
const RETRY: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub struct LocalActivity {
    moved: i32,
    last_move: Option<Instant>,
    sent: Option<Instant>,
}

impl LocalActivity {
    /// Evento físico local com o outro lado no controle: `true` quando é hora de mandar
    /// `TakeControl`.
    pub fn on_event(&mut self, ev: &CapturedEvent, now: Instant) -> bool {
        let intent = match *ev {
            CapturedEvent::MouseMove { dx, dy, .. } => {
                if self
                    .last_move
                    .is_none_or(|t| now.saturating_duration_since(t) > MOVE_GAP)
                {
                    self.moved = 0;
                }
                self.last_move = Some(now);
                self.moved = self.moved.saturating_add(dx.abs().saturating_add(dy.abs()));
                self.moved >= MOVE_PX
            }
            CapturedEvent::Button { down, .. } | CapturedEvent::Media { down, .. } => down,
            CapturedEvent::Key {
                down, autorepeat, ..
            } => down && !autorepeat,
            // A inércia do trackpad continua depois que os dedos saíram: não é intenção.
            CapturedEvent::Scroll { phase, .. } => phase != ScrollPhase::Momentum,
            CapturedEvent::Magnify { .. } | CapturedEvent::Rotate { .. } => true,
        };
        if !intent
            || self
                .sent
                .is_some_and(|t| now.saturating_duration_since(t) < RETRY)
        {
            return false;
        }
        self.sent = Some(now);
        self.moved = 0;
        true
    }

    /// O controle voltou para esta máquina.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{KeyCode, MouseButton};

    fn mv(dx: i32, dy: i32) -> CapturedEvent {
        CapturedEvent::MouseMove {
            dx,
            dy,
            pos: (0.0, 0.0),
        }
    }

    #[test]
    fn tremida_nao_toma_e_movimento_de_verdade_toma_uma_vez() {
        let t0 = Instant::now();
        let mut a = LocalActivity::default();
        assert!(!a.on_event(&mv(2, 1), t0));
        // Parado por mais que MOVE_GAP: o acumulado recomeça.
        assert!(!a.on_event(&mv(3, 2), t0 + Duration::from_millis(400)));
        assert!(a.on_event(&mv(4, 0), t0 + Duration::from_millis(410)));
        // Enquanto o outro lado não confirma, não repete antes de RETRY.
        assert!(!a.on_event(&mv(20, 0), t0 + Duration::from_millis(420)));
        assert!(a.on_event(&mv(20, 0), t0 + Duration::from_millis(950)));
    }

    #[test]
    fn clique_e_tecla_tomam_soltar_repeticao_e_inercia_nao() {
        let t0 = Instant::now();
        let up = CapturedEvent::Button {
            button: MouseButton::Left,
            down: false,
        };
        let repeat = CapturedEvent::Key {
            code: KeyCode::Mac(0),
            down: true,
            flags: 0,
            autorepeat: true,
        };
        let inertia = CapturedEvent::Scroll {
            dx_px: 0.0,
            dy_px: 4.0,
            phase: ScrollPhase::Momentum,
            continuous: true,
        };
        let mut a = LocalActivity::default();
        for ev in [up, repeat, inertia] {
            assert!(!a.on_event(&ev, t0), "{ev:?}");
        }
        let click = CapturedEvent::Button {
            button: MouseButton::Left,
            down: true,
        };
        assert!(a.on_event(&click, t0));
        a.reset();
        let key = CapturedEvent::Key {
            code: KeyCode::Mac(0),
            down: true,
            flags: 0,
            autorepeat: false,
        };
        assert!(a.on_event(&key, t0));
    }
}
