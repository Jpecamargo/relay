//! dono: WI3
//! Costuras de plataforma da mídia: o que o agente usa do Windows (Core Audio, Explorer), atrás
//! de traits para rodar com mocks no Mac. A implementação real só faz efeito no Windows.

use std::path::PathBuf;

use relay_core::platform::{AudioPlayback, PlatformError};
use relay_core::types::AudioDevices;
use relay_platform_windows::audio_endpoints;
use relay_platform_windows::audio_stream::EndpointRender;
use relay_platform_windows::explorer;

/// Tudo de áudio que o agente pede ao sistema.
pub trait AudioHost: Send + Sync {
    fn devices(&self) -> Result<AudioDevices, PlatformError>;
    /// Reprodução num endpoint de render (`None` = padrão).
    fn render(&self, endpoint: Option<String>) -> Box<dyn AudioPlayback>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsAudioHost;

impl AudioHost for WindowsAudioHost {
    fn devices(&self) -> Result<AudioDevices, PlatformError> {
        audio_endpoints::list_all()
    }
    fn render(&self, endpoint: Option<String>) -> Box<dyn AudioPlayback> {
        Box::new(EndpointRender::new(endpoint))
    }
}

/// Até a 0.1.x o agente mutava a saída do PC enquanto o som ia para o Mac, com este marcador.
fn mute_marker() -> PathBuf {
    crate::logging::log_dir()
        .parent()
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("saida-silenciada")
}

/// Desfaz o mudo deixado por um agente 0.1.x que caiu com o som indo para o Mac.
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
