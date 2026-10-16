//! dono: M5
//! Clipboard do macOS (NSPasteboard). Stub do F2.

use relay_core::platform::{ChangeCallback, ClipboardBackend, PlatformError};
use relay_core::types::ClipboardContent;

#[derive(Debug, Default)]
pub struct MacClipboard {}

impl ClipboardBackend for MacClipboard {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn write_remote(&mut self, _content: ClipboardContent) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn set_change_callback(&mut self, _callback: ChangeCallback) {}
}
