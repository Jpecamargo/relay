//! dono: C5b
//! Detector de atalhos reservados e configurados (`HotkeyAction`).

use arrayvec::ArrayVec;

use super::tables::mac_to_scan;
use super::translator::{combo_to_scan, modifier_slot, mods_from_slots};
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
    /// (modificadores, scan da tecla, ação)
    bindings: Vec<(u8, u16, HotkeyAction)>,
    /// Modificadores segurados, um bit por slot (esquerdo/direito).
    slots: u8,
    /// Teclas de atalho com o down engolido: o up e o autorepeat também somem.
    swallowed: ArrayVec<u16, 8>,
}

impl HotkeyMatcher {
    pub fn new(bindings: Vec<HotkeyBinding>) -> Self {
        let bindings = bindings
            .into_iter()
            .filter_map(|b| combo_to_scan(&b.combo).map(|(m, k)| (m, k, b.action)))
            .collect();
        Self {
            bindings,
            slots: 0,
            swallowed: ArrayVec::new(),
        }
    }

    /// Alimenta um evento de tecla cru da captura.
    pub fn feed(&mut self, code: KeyCode, down: bool) -> HotkeyOutcome {
        let scan = match code {
            KeyCode::Scan(s) => Some(s),
            KeyCode::Mac(m) => mac_to_scan(m),
        };
        let Some(scan) = scan else {
            return HotkeyOutcome::Pass;
        };
        if let Some(slot) = modifier_slot(scan) {
            if down {
                self.slots |= 1 << slot;
            } else {
                self.slots &= !(1 << slot);
            }
            return HotkeyOutcome::Pass;
        }
        let swallowed = self.swallowed.iter().position(|&s| s == scan);
        if !down {
            return match swallowed {
                Some(i) => {
                    self.swallowed.remove(i);
                    HotkeyOutcome::Consumed(None)
                }
                None => HotkeyOutcome::Pass,
            };
        }
        if swallowed.is_some() {
            return HotkeyOutcome::Consumed(None);
        }
        let mods = mods_from_slots(self.slots);
        match self.bindings.iter().find(|b| b.0 == mods && b.1 == scan) {
            Some((_, _, action)) => {
                let _ = self.swallowed.try_push(scan);
                HotkeyOutcome::Consumed(Some(action.clone()))
            }
            None => HotkeyOutcome::Pass,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{KeyCombo, Modifier, default_hotkeys};

    const CTRL: u16 = 0x1D;
    const ALT: u16 = 0x38;

    fn feed(m: &mut HotkeyMatcher, scan: u16, down: bool) -> HotkeyOutcome {
        m.feed(KeyCode::Scan(scan), down)
    }

    fn matcher() -> HotkeyMatcher {
        HotkeyMatcher::new(default_hotkeys())
    }

    #[test]
    fn ctrl_option_digitos_pulam_para_a_tela() {
        let mut m = matcher();
        feed(&mut m, CTRL, true);
        feed(&mut m, ALT, true);
        for n in 1..=3u8 {
            let scan = 0x01 + u16::from(n);
            assert_eq!(
                feed(&mut m, scan, true),
                HotkeyOutcome::Consumed(Some(HotkeyAction::JumpTo(n)))
            );
            assert_eq!(feed(&mut m, scan, false), HotkeyOutcome::Consumed(None));
        }
    }

    #[test]
    fn todos_os_digitos_1_a_9_pulam_quando_configurados() {
        let bindings = (1..=9u8)
            .map(|n| HotkeyBinding {
                combo: KeyCombo {
                    modifiers: vec![Modifier::Ctrl, Modifier::Option],
                    key: format!("Digit{n}"),
                },
                action: HotkeyAction::JumpTo(n),
            })
            .collect();
        let mut m = HotkeyMatcher::new(bindings);
        feed(&mut m, CTRL, true);
        feed(&mut m, ALT, true);
        for n in 1..=9u8 {
            let scan = 0x01 + u16::from(n);
            assert_eq!(
                feed(&mut m, scan, true),
                HotkeyOutcome::Consumed(Some(HotkeyAction::JumpTo(n)))
            );
            feed(&mut m, scan, false);
        }
    }

    #[test]
    fn l_trava_e_p_pausa() {
        let mut m = matcher();
        feed(&mut m, CTRL, true);
        feed(&mut m, ALT, true);
        assert_eq!(
            feed(&mut m, 0x26, true),
            HotkeyOutcome::Consumed(Some(HotkeyAction::ToggleLock))
        );
        assert_eq!(
            feed(&mut m, 0x19, true),
            HotkeyOutcome::Consumed(Some(HotkeyAction::TogglePause))
        );
    }

    #[test]
    fn atalho_da_lista_do_config_e_modificadores_exatos() {
        let mut m = HotkeyMatcher::new(vec![HotkeyBinding {
            combo: KeyCombo {
                modifiers: vec![Modifier::Cmd, Modifier::Shift],
                key: "KeyV".into(),
            },
            action: HotkeyAction::ClipboardSendNow,
        }]);
        // Sem os modificadores: passa.
        assert_eq!(feed(&mut m, 0x2F, true), HotkeyOutcome::Pass);
        assert_eq!(feed(&mut m, 0x2F, false), HotkeyOutcome::Pass);
        // Cmd direito + Shift: dispara.
        feed(&mut m, 0xE05C, true);
        feed(&mut m, 0x2A, true);
        assert_eq!(
            feed(&mut m, 0x2F, true),
            HotkeyOutcome::Consumed(Some(HotkeyAction::ClipboardSendNow))
        );
        // Repetição do down não dispara de novo e o up some.
        assert_eq!(feed(&mut m, 0x2F, true), HotkeyOutcome::Consumed(None));
        assert_eq!(feed(&mut m, 0x2F, false), HotkeyOutcome::Consumed(None));
        // Modificador a mais: não casa.
        feed(&mut m, CTRL, true);
        assert_eq!(feed(&mut m, 0x2F, true), HotkeyOutcome::Pass);
    }

    #[test]
    fn tecla_comum_e_codigo_desconhecido_passam() {
        let mut m = matcher();
        assert_eq!(feed(&mut m, 0x2E, true), HotkeyOutcome::Pass);
        assert_eq!(m.feed(KeyCode::Mac(0xFFFF), true), HotkeyOutcome::Pass);
    }
}
