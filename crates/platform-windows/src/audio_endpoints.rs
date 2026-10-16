//! dono: W4a
//! Endpoints de áudio (enumeração, volume, mudo, troca de saída padrão). Stub do F2.

use relay_core::platform::PlatformError;
use relay_core::types::AudioDevice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Render,
    Capture,
}

pub fn list_endpoints(_kind: EndpointKind) -> Result<Vec<AudioDevice>, PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn set_volume(_level: f32) -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn set_mute(_muted: bool) -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn set_default_endpoint(_id: &str) -> Result<(), PlatformError> {
    Err(PlatformError::NotImplemented)
}

/// Guarda a saída padrão original e a restaura em `restore()` e no `Drop`.
#[derive(Debug, Default)]
pub struct DefaultOutputGuard {}
