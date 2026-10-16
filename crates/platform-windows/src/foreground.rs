//! dono: W3
//! Observador do app em primeiro plano (SetWinEventHook). Stub do F2.

/// Chama o callback com o basename do exe sempre que o app em primeiro plano muda.
#[derive(Debug, Default)]
pub struct ForegroundWatcher {}

impl ForegroundWatcher {
    pub fn start(&mut self, _on_change: Box<dyn Fn(String) + Send + Sync>) {}

    pub fn stop(&mut self) {}
}
