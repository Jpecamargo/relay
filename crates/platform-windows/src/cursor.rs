//! dono: W2
//! Controle do cursor do Windows (preso em um centro enquanto o Mac controla). Stub do F2.

use relay_core::platform::{CursorControl, PlatformError};

#[derive(Debug, Default)]
pub struct WindowsCursor {}

impl CursorControl for WindowsCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn show(&mut self) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn associate(&mut self, _associated: bool) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn warp(&mut self, _x: i32, _y: i32) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn position(&self) -> Result<(i32, i32), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}
