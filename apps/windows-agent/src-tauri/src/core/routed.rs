//! dono: WI1
//! Injeção roteada (D10): com o serviço presente toda injeção passa pelo pipe do helper (UAC, tela
//! de bloqueio, janelas de administrador); sem ele, o agente injeta direto.

use std::sync::{Arc, Mutex};

use relay_core::platform::{CursorControl, InjectError, InputInjector, KeyRepeat, PlatformError};
use relay_core::types::InputEvent;

/// Cliente do pipe do serviço (`InjectPipeClient` no Windows; mock nos testes).
pub trait PipeInjector: InputInjector {
    /// `true` se o helper do serviço está conectado (tenta reconectar no máximo 1x/s).
    fn is_available(&self) -> bool;
}

/// Pipe compartilhado entre o injetor do `Receiver` e o status do serviço.
pub type SharedPipe = Arc<Mutex<Box<dyn PipeInjector>>>;

/// Injetor do `Receiver`: pipe quando disponível, injeção local caso contrário (ou se o pipe
/// falhar no meio do evento).
pub struct RoutedInjector {
    local: Box<dyn InputInjector>,
    pipe: Option<SharedPipe>,
}

impl RoutedInjector {
    pub fn new(local: Box<dyn InputInjector>, pipe: Option<SharedPipe>) -> Self {
        Self { local, pipe }
    }

    fn via_pipe(
        &self,
        send: impl FnOnce(&mut dyn PipeInjector) -> Result<(), InjectError>,
    ) -> bool {
        let Some(pipe) = &self.pipe else { return false };
        let mut guard = pipe.lock().unwrap_or_else(|e| e.into_inner());
        send(guard.as_mut()).is_ok()
    }
}

impl InputInjector for RoutedInjector {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        if self.via_pipe(|p| p.inject(event)) {
            return Ok(());
        }
        self.local.inject(event)
    }

    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        if self.via_pipe(|p| p.type_text(text)) {
            return Ok(());
        }
        self.local.type_text(text)
    }

    fn key_repeat(&self) -> KeyRepeat {
        self.local.key_repeat()
    }
}

/// Adapta `Box<dyn CursorControl>` ao parâmetro genérico do `Receiver`.
pub struct BoxCursor(pub Box<dyn CursorControl>);

impl CursorControl for BoxCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        self.0.hide()
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        self.0.show()
    }
    fn associate(&mut self, associated: bool) -> Result<(), PlatformError> {
        self.0.associate(associated)
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        self.0.warp(x, y)
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        self.0.position()
    }
}
