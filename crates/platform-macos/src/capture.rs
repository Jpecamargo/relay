//! dono: M1
//! Captura via CGEventTap no nível HID. Stub do F2.

use relay_core::platform::{CaptureError, InputCapture};
use relay_core::types::CapturedEvent;

#[derive(Debug, Default)]
pub struct MacCapture {}

impl InputCapture for MacCapture {
    fn start(&mut self, _sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        Err(CaptureError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_swallow(&self, _swallow: bool) {}

    fn is_swallowing(&self) -> bool {
        false
    }
}
