//! dono: WI3
//! Costuras de plataforma da mídia: o que o agente usa do Windows (Core Audio, Explorer), atrás
//! de traits para rodar com mocks no Mac. A implementação real só faz efeito no Windows.

use std::path::PathBuf;

use relay_core::platform::{AudioCapture, AudioPlayback, PlatformError};
use relay_core::types::AudioDevices;
use relay_platform_windows::audio_endpoints::{self, DefaultOutputGuard, EndpointKind};
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

/// Mudo que o agente pôs numa saída do PC enquanto o som vai para o Mac; `restore()` e o `Drop`
/// o desfazem.
pub trait LocalMute: Send {
    fn restore(&mut self) -> Result<(), PlatformError>;
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
    /// Silencia a saída `endpoint` (`None` = padrão). `None` no retorno: já estava muda pelo
    /// usuário, e então não é do agente desfazer.
    fn mute(&self, endpoint: Option<String>) -> Result<Option<Box<dyn LocalMute>>, PlatformError>;
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
    fn mute(&self, endpoint: Option<String>) -> Result<Option<Box<dyn LocalMute>>, PlatformError> {
        let id = match endpoint {
            Some(id) => id,
            None => audio_endpoints::default_endpoint_id(EndpointKind::Render)?,
        };
        if audio_endpoints::get_mute_of(&id)? {
            return Ok(None);
        }
        // Marcador antes do mudo: se o agente cair com o PC mudo, o próximo boot desfaz.
        let _ = std::fs::create_dir_all(mute_marker().parent().unwrap_or(&mute_marker()));
        let _ = std::fs::write(mute_marker(), &id);
        audio_endpoints::set_mute_of(&id, true)?;
        Ok(Some(Box::new(EndpointMute(Some(id)))))
    }
}

fn mute_marker() -> PathBuf {
    crate::logging::log_dir()
        .parent()
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("saida-silenciada")
}

struct EndpointMute(Option<String>);

impl LocalMute for EndpointMute {
    fn restore(&mut self) -> Result<(), PlatformError> {
        if let Some(id) = self.0.clone() {
            audio_endpoints::set_mute_of(&id, false)?;
            let _ = std::fs::remove_file(mute_marker());
            self.0 = None;
        }
        Ok(())
    }
}

impl Drop for EndpointMute {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Desfaz o mudo deixado por um agente que caiu com o som indo para o Mac.
pub fn recover_local_mute() {
    let Ok(id) = std::fs::read_to_string(mute_marker()) else {
        return;
    };
    match audio_endpoints::set_mute_of(id.trim(), false) {
        Ok(()) => tracing::info!("saída do PC desmutada (o agente anterior caiu com ela muda)"),
        Err(e) => tracing::warn!(error = %e, "não consegui desmutar a saída do PC"),
    }
    let _ = std::fs::remove_file(mute_marker());
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
