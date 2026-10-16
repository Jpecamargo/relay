//! dono: MI1
//! Fronteira do AppCore com o mundo: plataforma (traits do core + o que é só do Mac), saída de
//! eventos para a UI e ganchos da mídia (MI3). Nada aqui conhece o Tauri.

use std::any::Any;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use relay_core::platform::{
    CaptureError, CursorControl, DisplaySource, InjectError, InputCapture, InputInjector,
    KeyRepeat, KeyStore, PlatformError,
};
use relay_core::types::{
    CapturedEvent, CommandResult, DiscoveredPeer, DisplaysPayload, DockInfo, HotCorners,
    InputEvent, LatencyStats, Message, PairingEvent, PermissionKind, PermissionsStatus,
    StatusSnapshot,
};

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
}

/// Ganchos da mídia (áudio, clipboard, arquivos), implementados pelo MI3. O AppCore só chama.
pub trait MediaHooks: Send + Sync + 'static {
    /// Troca de foco com clipboard em `on_switch`.
    fn clipboard_switch(&self) {}
    /// Atalho `ClipboardSendNow` ou botão do painel.
    fn clipboard_send_now(&self) {}
    fn receive_remote_files(&self) {}
    fn set_audio_streams(&self, _pc_to_mac: bool, _mac_to_pc: bool) {}
    /// Mensagem do PC que o AppCore não trata (Clipboard*, File*, DropFiles, Audio*, Metrics).
    fn on_message(&self, _msg: &Message) {}
    /// Sessão estabelecida (`true`) ou perdida (`false`).
    fn link_changed(&self, _connected: bool) {}
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
