//! dono: W3
//! `ShellExecute` de URL, caminho ou nome de app. Stub do F2.

use relay_core::platform::PlatformError;

pub fn open_target(_target: &str) -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}
