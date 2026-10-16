//! dono: WI3
//! Costuras de plataforma da mídia: o que o agente usa do Windows (Core Audio, Explorer), atrás
//! de traits para rodar com mocks no Mac. A implementação real só faz efeito no Windows.

use std::path::PathBuf;

use relay_core::platform::{AudioCapture, AudioPlayback, PlatformError};
use relay_core::types::AudioDevices;
use relay_platform_windows::audio_endpoints::{self, DefaultOutputGuard};
use relay_platform_windows::audio_stream::{
    EndpointRender, LoopbackCapture, ProcessLoopbackCapture,
};
use relay_platform_windows::explorer;

/// Guarda da saída padrão (D14, modo `system`): restaura a original ao `restore()` e no `Drop`.
pub trait OutputGuard: Send {
    fn switch_to(&mut self, id: &str) -> Result<(), PlatformError>;
    fn restore(&mut self) -> Result<(), PlatformError>;
}

impl OutputGuard for DefaultOutputGuard {
    fn switch_to(&mut self, id: &str) -> Result<(), PlatformError> {
        DefaultOutputGuard::switch_to(self, id)
    }
    fn restore(&mut self) -> Result<(), PlatformError> {
        DefaultOutputGuard::restore(self)
    }
}

/// Tudo de áudio que o agente pede ao sistema.
pub trait AudioHost: Send + Sync {
    fn devices(&self) -> Result<AudioDevices, PlatformError>;
    /// Registra a saída padrão atual como a original.
    fn guard(&self) -> Result<Box<dyn OutputGuard>, PlatformError>;
    /// Loopback de um endpoint de render (`None` = padrão).
    fn loopback(&self, endpoint: Option<String>) -> Box<dyn AudioCapture>;
    /// Loopback só do processo `exe` (e filhos).
    fn process_loopback(&self, exe: &str) -> Box<dyn AudioCapture>;
    /// Reprodução num endpoint de render (`None` = padrão).
    fn render(&self, endpoint: Option<String>) -> Box<dyn AudioPlayback>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsAudioHost;

impl AudioHost for WindowsAudioHost {
    fn devices(&self) -> Result<AudioDevices, PlatformError> {
        audio_endpoints::list_all()
    }
    fn guard(&self) -> Result<Box<dyn OutputGuard>, PlatformError> {
        Ok(Box::new(DefaultOutputGuard::new()?))
    }
    fn loopback(&self, endpoint: Option<String>) -> Box<dyn AudioCapture> {
        Box::new(LoopbackCapture::new(endpoint))
    }
    fn process_loopback(&self, exe: &str) -> Box<dyn AudioCapture> {
        Box::new(ProcessLoopbackCapture::new(exe))
    }
    fn render(&self, endpoint: Option<String>) -> Box<dyn AudioPlayback> {
        Box::new(EndpointRender::new(endpoint))
    }
}

/// Destino de um `DropFiles`: pasta do Explorer sob o cursor, senão Downloads (D20).
pub type DropTarget = Box<dyn Fn() -> Option<PathBuf> + Send + Sync>;

pub fn windows_drop_target() -> DropTarget {
    Box::new(|| explorer::explorer_folder_at_cursor().or_else(explorer::downloads_dir))
}

/// VB-CABLE instalado? (detecção real por nome dos endpoints; para a UI do agente.)
pub fn detect_vbcable() -> bool {
    audio_endpoints::list_all()
        .map(|d| audio_endpoints::find_vbcable(&d).input.is_some())
        .unwrap_or(false)
}
