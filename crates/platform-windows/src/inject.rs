//! dono: W1
//! Injeção via SendInput. Stub do F2.

use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::InputEvent;

#[derive(Debug, Default)]
pub struct WindowsInjector {}

impl InputInjector for WindowsInjector {
    fn inject(&mut self, _event: &InputEvent) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }

    fn type_text(&mut self, _text: &str) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }
}
