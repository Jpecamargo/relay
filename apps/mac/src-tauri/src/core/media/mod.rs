//! dono: MI3
//! Mídia do Mac (D13, D20): áudio PC↔Mac, clipboard, arquivos e arraste. `RealMedia` implementa
//! os `MediaHooks` do AppCore; tudo que toca o sistema (dispositivos de áudio, pasteboard,
//! Finder) passa por `MediaPlatform`, para os testes rodarem com mocks.
//!
//! Threads: áudio tem as suas (UDP + reprodução/captura); o clipboard tem um trabalhador que é
//! dono do `ClipboardBackend` e do `ClipboardSync`; transferências rodam no runtime tokio. Os
//! ganchos chamados pela thread do core só enfileiram trabalho.

mod audio;
mod clipboard;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(test)]
mod tests;
mod transfer;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use relay_core::platform::{AudioCapture, AudioPlayback, ClipboardBackend};
use relay_core::types::{AudioDevices, Config, FileEntry, Message};
use tokio::runtime::Handle;

use super::{CoreEvents, MediaHooks, SessionCtx};
use audio::AudioRun;
use clipboard::ClipCmd;
use transfer::Files;

#[cfg(target_os = "macos")]
pub use mac::MacMedia;

/// Tudo o que a mídia pede ao sistema.
pub trait MediaPlatform: Send + Sync + 'static {
    fn playback(&self) -> Box<dyn AudioPlayback>;
    fn capture(&self) -> Box<dyn AudioCapture>;
    fn clipboard(&self) -> Box<dyn ClipboardBackend>;
    fn audio_devices(&self) -> AudioDevices;
    /// Arquivos do pasteboard de arraste (vazio se não há arraste de arquivos).
    fn drag_files(&self) -> Vec<PathBuf>;
    /// O Finder é o app em primeiro plano.
    fn finder_frontmost(&self) -> bool;
    fn reveal(&self, paths: &[PathBuf]);
    /// Destino dos arquivos vindos do PC (`~/Downloads/Relay`).
    fn downloads_dir(&self) -> PathBuf;
}

/// Estado compartilhado entre os ganchos, o trabalhador do clipboard e as tarefas.
pub(crate) struct Sh {
    pub platform: Arc<dyn MediaPlatform>,
    pub events: Arc<dyn CoreEvents>,
    pub rt: Handle,
    pub files: Files,
    st: Mutex<St>,
}

#[derive(Default)]
pub(crate) struct St {
    pub cfg: Config,
    pub session: Option<Arc<SessionCtx>>,
    /// Streams pedidos pelo usuário (PC→Mac, Mac→PC); valem de sessão em sessão.
    pub want: (bool, bool),
    pub audio: AudioRun,
    /// Arquivos que o PC ofereceu e ainda não foram baixados.
    pub pending: Option<Vec<FileEntry>>,
    /// Oferta do arraste Mac→Dell à espera do mouse up remoto.
    pub armed: Option<u32>,
    pub next_offer: u32,
}

impl Sh {
    pub fn st(&self) -> MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Manda uma mensagem pelo canal confiável da sessão atual (descarta sem sessão).
    pub fn send(&self, msg: Message) {
        if let Some(ctx) = self.st().session.clone() {
            let _ = ctx.send.send(msg);
        }
    }

    pub fn session(&self) -> Option<Arc<SessionCtx>> {
        self.st().session.clone()
    }
}

/// Ganchos reais da mídia.
pub struct RealMedia {
    sh: Arc<Sh>,
    clip: Mutex<Sender<ClipCmd>>,
}

impl RealMedia {
    pub fn new(
        platform: Arc<dyn MediaPlatform>,
        events: Arc<dyn CoreEvents>,
        rt: Handle,
    ) -> Arc<Self> {
        let sh = Arc::new(Sh {
            platform,
            events,
            rt,
            files: Files::default(),
            st: Mutex::new(St::default()),
        });
        let clip = clipboard::spawn(sh.clone());
        Arc::new(Self {
            sh,
            clip: Mutex::new(clip),
        })
    }

    fn clip(&self, cmd: ClipCmd) {
        let _ = self
            .clip
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(cmd);
    }

    /// Endereço do servidor de transferência (testes, que usam porta efêmera).
    pub fn transfer_addr(&self) -> Option<SocketAddr> {
        self.sh.files.addr()
    }

    /// Endereço UDP em que o áudio PC→Mac está escutando.
    pub fn audio_rx_addr(&self) -> Option<SocketAddr> {
        self.sh.st().audio.rx_addr()
    }

    /// Há arquivos do PC esperando para serem baixados.
    pub fn has_pending_files(&self) -> bool {
        self.sh.st().pending.is_some()
    }
}

impl Drop for RealMedia {
    fn drop(&mut self) {
        self.clip(ClipCmd::Quit);
        let audio = std::mem::take(&mut self.sh.st().audio);
        drop(audio);
        self.sh.files.shutdown();
    }
}

impl MediaHooks for RealMedia {
    fn clipboard_switch(&self) {
        self.clip(ClipCmd::Switch);
    }

    fn clipboard_send_now(&self) {
        self.clip(ClipCmd::SendNow);
    }

    fn receive_remote_files(&self) {
        transfer::receive_files(&self.sh);
    }

    fn set_audio_streams(&self, pc_to_mac: bool, mac_to_pc: bool) {
        audio::set_streams(&self.sh, pc_to_mac, mac_to_pc);
    }

    fn on_message(&self, msg: &Message) {
        match msg {
            Message::ClipboardOffer { .. }
            | Message::ClipboardChunk { .. }
            | Message::ClipboardSkipped { .. } => self.clip(ClipCmd::Msg(msg.clone())),
            // Estado real do áudio no PC: o que ele recusou não liga aqui também.
            Message::AudioControl {
                pc_to_mac,
                mac_to_pc,
            } => audio::pc_state(&self.sh, *pc_to_mac, *mac_to_pc),
            Message::CommandResult { id: 0, result } => {
                tracing::warn!("áudio no PC: {:?}", result.error);
            }
            Message::FileOffer { files, .. } => {
                self.sh.st().pending = Some(files.clone());
            }
            _ => {}
        }
    }

    fn link_changed(&self, connected: bool) {
        if !connected {
            transfer::session_down(&self.sh);
            audio::session_down(&self.sh);
            let mut st = self.sh.st();
            st.session = None;
            st.armed = None;
            st.pending = None;
        }
    }

    fn session_up(&self, ctx: SessionCtx) {
        let ctx = Arc::new(ctx);
        self.sh.st().session = Some(ctx.clone());
        self.sh.files.listen(&self.sh, &ctx);
        audio::session_up(&self.sh);
    }

    fn focus_changed(&self, remote: bool, left_down: bool) {
        transfer::focus_changed(&self.sh, remote, left_down);
    }

    fn left_button_up(&self) {
        transfer::left_button_up(&self.sh);
    }

    fn paste_requested(&self) {
        transfer::paste_requested(&self.sh);
    }

    fn config_changed(&self, config: &Config) {
        self.sh.st().cfg = config.clone();
        self.clip(ClipCmd::Config(config.clipboard.clone()));
        audio::config_changed(&self.sh);
    }

    fn audio_devices(&self) -> AudioDevices {
        self.sh.platform.audio_devices()
    }
}
