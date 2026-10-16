//! dono: W3
//! Ações de energia (lock, suspend, restart, shutdown). Stubs do F2.

use relay_core::platform::PlatformError;

pub fn lock() -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn suspend() -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn restart() -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn shutdown() -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}
