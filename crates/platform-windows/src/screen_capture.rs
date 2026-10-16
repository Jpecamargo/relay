//! dono: W6
//! Captura de tela de um monitor em PNG (Windows.Graphics.Capture). Stub do F2.

use relay_core::platform::PlatformError;

pub fn capture_monitor_png(_display_id: &str) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::NotImplemented)
}
