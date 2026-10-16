//! dono: C5b
//! Rastreador de teclas pressionadas no destino.

use arrayvec::ArrayVec;

use super::translator::KeyEvent;

pub const MAX_PRESSED: usize = 32;

#[derive(Debug, Default)]
pub struct PressedTracker {
    pressed: ArrayVec<u16, MAX_PRESSED>,
}

impl PressedTracker {
    /// Registra um evento já traduzido (o que o destino vê).
    pub fn track(&mut self, event: KeyEvent) {
        let pos = self.pressed.iter().position(|&s| s == event.scan);
        match (event.down, pos) {
            (true, None) => {
                let _ = self.pressed.try_push(event.scan);
            }
            (false, Some(i)) => {
                self.pressed.remove(i);
            }
            _ => {}
        }
    }

    /// keyUp de toda tecla atualmente pressionada no destino, modificadores incluídos.
    /// Sai na ordem inversa da pressão.
    pub fn release_all(&mut self) -> ArrayVec<KeyEvent, MAX_PRESSED> {
        let ups = self
            .pressed
            .iter()
            .rev()
            .map(|&scan| KeyEvent {
                scan,
                down: false,
                autorepeat: false,
            })
            .collect();
        self.pressed.clear();
        ups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(scan: u16, down: bool) -> KeyEvent {
        KeyEvent {
            scan,
            down,
            autorepeat: false,
        }
    }

    #[test]
    fn release_all_solta_tudo_em_ordem_inversa() {
        let mut t = PressedTracker::default();
        t.track(ev(0x38, true));
        t.track(ev(0x0F, true));
        t.track(ev(0x0F, true)); // duplicata ignorada
        t.track(ev(0x2E, true));
        t.track(ev(0x2E, false));
        let ups = t.release_all();
        assert_eq!(ups.as_slice(), &[ev(0x0F, false), ev(0x38, false)]);
        assert!(t.release_all().is_empty());
    }
}
