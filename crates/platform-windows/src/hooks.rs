//! dono: W2
//! Captura via hooks WH_KEYBOARD_LL e WH_MOUSE_LL (sentido inverso). Stub do F2.

use relay_core::platform::{CaptureError, InputCapture};
use relay_core::types::CapturedEvent;

#[derive(Debug, Default)]
pub struct WindowsHooks {}

impl InputCapture for WindowsHooks {
    fn start(&mut self, _sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        Err(CaptureError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_swallow(&self, _swallow: bool) {}

    fn is_swallowing(&self) -> bool {
        false
    }
}
