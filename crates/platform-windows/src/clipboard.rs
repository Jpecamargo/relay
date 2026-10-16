//! dono: W5
//! Clipboard do Windows (listener, formatos, anti-eco, renderização atrasada). Stub do F2.

use relay_core::platform::{ChangeCallback, ClipboardBackend, PlatformError};
use relay_core::types::ClipboardContent;

#[derive(Debug, Default)]
pub struct WindowsClipboard {}

impl ClipboardBackend for WindowsClipboard {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn write_remote(&mut self, _content: ClipboardContent) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn set_change_callback(&mut self, _callback: ChangeCallback) {}
}
