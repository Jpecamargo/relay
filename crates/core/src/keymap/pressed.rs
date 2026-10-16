//! dono: C5b
//! Rastreador de teclas pressionadas no destino. Stubs do F2: assinaturas.

use arrayvec::ArrayVec;

use super::translator::KeyEvent;

pub const MAX_PRESSED: usize = 32;

#[derive(Debug, Default)]
pub struct PressedTracker {
    _pressed: ArrayVec<u16, MAX_PRESSED>,
}

impl PressedTracker {
    /// Registra um evento já traduzido (o que o destino vê).
    pub fn track(&mut self, _event: KeyEvent) {}

    /// keyUp de toda tecla atualmente pressionada no destino, modificadores incluídos.
    pub fn release_all(&mut self) -> ArrayVec<KeyEvent, MAX_PRESSED> {
        ArrayVec::new()
    }
}
