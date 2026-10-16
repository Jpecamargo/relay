//! dono: W3
//! Configuração de repetição de tecla do Windows. Stub do F2.

use relay_core::platform::{KeyRepeat, PlatformError};

pub fn keyboard_repeat() -> Result<KeyRepeat, PlatformError> {
    Err(PlatformError::NotImplemented)
}
