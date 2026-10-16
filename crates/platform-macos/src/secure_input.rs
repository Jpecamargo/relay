//! dono: M2
//! Detecção de Secure Input (IsSecureEventInputEnabled a cada 500 ms). Stub do F2.

/// Consulta periódica; chama o callback só quando o valor muda.
#[derive(Debug, Default)]
pub struct SecureInputWatcher {}

impl SecureInputWatcher {
    pub fn start(&mut self, _on_change: Box<dyn Fn(bool) + Send + Sync>) {}

    pub fn stop(&mut self) {}
}
