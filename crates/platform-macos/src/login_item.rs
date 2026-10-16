//! dono: M3
//! Login item via SMAppService. Stub do F2.

use relay_core::platform::PlatformError;

#[derive(Debug, Default)]
pub struct LoginItem {}

impl LoginItem {
    pub fn register(&self) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    pub fn unregister(&self) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    pub fn is_registered(&self) -> Result<bool, PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}
