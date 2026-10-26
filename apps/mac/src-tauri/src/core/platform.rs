//! dono: MI1
//! Fronteira do AppCore com o mundo: plataforma (traits do core + o que é só do Mac), saída de
//! eventos para a UI e ganchos da mídia (MI3). Nada aqui conhece o Tauri.

use std::any::Any;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use relay_core::crypto::{StaticKeypair, UdpKeys};
use relay_core::platform::{
    CaptureError, CursorControl, DisplaySource, InjectError, InputCapture, InputInjector,
    KeyRepeat, KeyStore, LoopWaker, PlatformError,
};
use relay_core::types::{
    AudioDevices, CapturedEvent, CommandResult, Config, DiscoveredPeer, DisplaysPayload, DockInfo,
    HotCorners, InputEvent, LatencyStats, Message, PairedPeer, PairingEvent, PermissionKind,
    PermissionsStatus, Ports, StatusSnapshot, TransferProgress,
};
use std::net::IpAddr;
use tokio::sync::mpsc::UnboundedSender;

/// Eventos de sistema que o AppCore trata (espelho de `system_events::SystemEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysEvent {
    WillSleep,
    DidWake,
    NetworkChanged,
}

/// Busca de agentes na rede (mDNS no Mac).
pub trait Browse: Send {
    /// Próximo agente resolvido, ou `None` se nada chegar em `timeout`.
    fn next_peer(&self, timeout: Duration) -> Option<DiscoveredPeer>;
}

/// Tudo o que o AppCore pede à plataforma. As quatro traits de entrada/saída são as do core;
/// o resto é o que não tem trait lá (permissões, Dock, eventos de sistema, descoberta).
pub trait Platform: Send + Sync + 'static {
    type Capture: InputCapture + 'static;
    type Cursor: CursorControl + 'static;
    type Injector: InputInjector + 'static;
    type Displays: DisplaySource + 'static;

    fn new_capture(&self) -> Self::Capture;
    fn new_cursor(&self) -> Self::Cursor;
    fn new_injector(&self) -> Self::Injector;
    fn new_displays(&self) -> Self::Displays;

    fn key_store(&self) -> Arc<dyn KeyStore>;
    fn config_path(&self) -> PathBuf;
    /// Nome deste Mac, entregue ao PC no pareamento.
    fn device_name(&self) -> String;

    fn check_permissions(&self) -> PermissionsStatus;
    fn open_permission_settings(&self, kind: PermissionKind);
    fn dock(&self) -> (DockInfo, HotCorners);
    fn set_launch_at_login(&self, enabled: bool) -> Result<(), String>;

    /// Assina sleep/wake e troca de rede; a assinatura vale enquanto o guarda viver.
    fn watch_system(&self, callback: Box<dyn Fn(SysEvent) + Send + Sync>) -> Box<dyn Any + Send>;
    /// Assina o Secure Input; a assinatura vale enquanto o guarda viver.
    fn watch_secure_input(&self, callback: Box<dyn Fn(bool) + Send + Sync>) -> Box<dyn Any + Send>;
    fn browse(&self) -> Result<Box<dyn Browse>, String>;
    /// Anuncia este Mac por mDNS (TXT `id/pv/udp/os/v`) com as portas reais de escuta; o anúncio
    /// vale enquanto o guarda viver.
    fn advertise(
        &self,
        device_id: &str,
        control_port: u16,
        input_port: u16,
    ) -> Result<Box<dyn Any + Send>, String>;
}

/// Saída core→UI. O app Tauri implementa com `emit_*`; os testes, com um registro.
pub trait CoreEvents: Send + Sync + 'static {
    fn status(&self, status: &StatusSnapshot);
    fn latency(&self, latency: &LatencyStats);
    fn displays(&self, displays: &DisplaysPayload);
    fn permissions(&self, permissions: &PermissionsStatus);
    fn secure_input(&self, active: bool);
    fn pairing(&self, event: &PairingEvent);
    fn command_result(&self, result: &CommandResult);
    /// `relay:transfer-progress` (≤ 10 Hz).
    fn transfer_progress(&self, _progress: &TransferProgress) {}
    /// `relay:update-available`: o PC está numa versão mais nova e pediu que o Mac se atualize.
    fn update_available(&self, _version: &str) {}
}

/// Chaves UDP do microfone na sessão atual (derivadas do handshake Noise).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioKeys {
    pub mac_to_pc: UdpKeys,
}

/// O que a mídia precisa de uma sessão de pé: fila de saída do canal confiável, endereço do PC,
/// portas, identidade (transferência de arquivos) e chaves de áudio.
#[derive(Clone)]
pub struct SessionCtx {
    /// Fila da bomba da sessão: tudo o que entra aqui sai por `Session::send`.
    pub send: UnboundedSender<Message>,
    pub peer_ip: IpAddr,
    /// Portas do PC (`audio_mac_to_pc`, `transfer`).
    pub remote_ports: Ports,
    /// Portas do Mac (`transfer`).
    pub local_ports: Ports,
    pub bind_ip: IpAddr,
    pub identity: StaticKeypair,
    pub peer: PairedPeer,
    pub audio_keys: Option<AudioKeys>,
}

/// Ganchos da mídia (áudio, clipboard, arquivos), implementados pelo MI3. O AppCore só chama.
pub trait MediaHooks: Send + Sync + 'static {
    /// Troca de foco com clipboard em `on_switch`.
    fn clipboard_switch(&self) {}
    /// Atalho `ClipboardSendNow` ou botão do painel.
    fn clipboard_send_now(&self) {}
    fn receive_remote_files(&self) {}
    fn set_audio_streams(&self, _mac_to_pc: bool) {}
    /// Mensagem do PC que o AppCore não trata (Clipboard*, File*, DropFiles, Audio*, Metrics).
    fn on_message(&self, _msg: &Message) {}
    /// Sessão estabelecida (`true`) ou perdida (`false`).
    fn link_changed(&self, _connected: bool) {}
    /// Sessão `Connected` (antes de `link_changed(true)`; repete a cada reconexão).
    fn session_up(&self, _ctx: SessionCtx) {}
    /// O foco mudou de máquina; `left_down`: botão esquerdo pressionado nesse instante (arraste).
    fn focus_changed(&self, _remote: bool, _left_down: bool) {}
    /// O botão esquerdo foi solto (depois de o Controller tratar o evento).
    fn left_button_up(&self) {}
    /// Cmd+V com o foco no Mac.
    fn paste_requested(&self) {}
    /// Config gravada (inclusive a inicial).
    fn config_changed(&self, _config: &Config) {}
    fn audio_devices(&self) -> AudioDevices {
        AudioDevices::default()
    }
    /// Um `CaptureScreen` foi enviado ao PC: o PNG que ele devolve é ação explícita do usuário.
    fn capture_screen_requested(&self) {}
}

/// Sem mídia: o padrão até o MI3.
pub struct NoMedia;
impl MediaHooks for NoMedia {}

/// Instância compartilhada de um backend de plataforma: o Controller guarda uma cópia e o
/// AppCore outra, para reconstruir o Controller (nova sessão) sem perder captura, cursor e
/// injetor. Os locks só disputam em troca de foco, nunca no caminho de cada evento.
pub struct Shared<T>(Arc<Mutex<T>>);

impl<T> Shared<T> {
    pub fn new(inner: T) -> Self {
        Self(Arc::new(Mutex::new(inner)))
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: InputCapture> InputCapture for Shared<T> {
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        self.lock().start(sink)
    }
    fn set_waker(&mut self, waker: LoopWaker) {
        self.lock().set_waker(waker);
    }
    fn stop(&mut self) {
        self.lock().stop();
    }
    fn set_swallow(&self, swallow: bool) {
        self.lock().set_swallow(swallow);
    }
    fn is_swallowing(&self) -> bool {
        self.lock().is_swallowing()
    }
}

impl<T: CursorControl> CursorControl for Shared<T> {
    fn hide(&mut self) -> Result<(), PlatformError> {
        self.lock().hide()
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        self.lock().show()
    }
    fn associate(&mut self, associated: bool) -> Result<(), PlatformError> {
        self.lock().associate(associated)
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        self.lock().warp(x, y)
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        self.lock().position()
    }
}

impl<T: InputInjector> InputInjector for Shared<T> {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.lock().inject(event)
    }
    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        self.lock().type_text(text)
    }
    fn key_repeat(&self) -> KeyRepeat {
        self.lock().key_repeat()
    }
}
