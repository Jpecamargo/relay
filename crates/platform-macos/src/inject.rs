//! dono: M2
//! Injeção via CGEventPost (sentido inverso). Stub do F2.

use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::InputEvent;

#[derive(Debug, Default)]
pub struct MacInjector {}

impl InputInjector for MacInjector {
    fn inject(&mut self, _event: &InputEvent) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }

    fn type_text(&mut self, _text: &str) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }
}
