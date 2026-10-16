//! dono: M3
//! Leitura de telas com notificação de mudança. Stub do F2.

use relay_core::platform::{ChangeCallback, DisplaySource, PlatformError};
use relay_core::types::DisplayInfo;

#[derive(Debug, Default)]
pub struct MacDisplays {}

impl DisplaySource for MacDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn set_change_callback(&mut self, _callback: ChangeCallback) {}
}
