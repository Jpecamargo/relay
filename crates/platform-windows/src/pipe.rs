//! dono: W7
//! Pipe `\\.\pipe\relay-inject`: `InjectFrame` de 32 bytes (puro) e o cliente usado pelo agente.
//! Stub do F2.

use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::InputEvent;

#[derive(Debug, Default)]
pub struct InjectPipeClient {}

impl InjectPipeClient {
    /// `true` se o helper do serviço está conectado.
    pub fn is_available(&self) -> bool {
        false
    }
}

impl InputInjector for InjectPipeClient {
    fn inject(&mut self, _event: &InputEvent) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }

    fn type_text(&mut self, _text: &str) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }
}
