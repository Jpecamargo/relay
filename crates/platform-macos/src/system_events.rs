//! dono: M3
//! Eventos de sistema (sleep, wake, rede). Stub do F2.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    WillSleep,
    DidWake,
    NetworkChanged,
}

#[derive(Debug, Default)]
pub struct SystemEvents {}

impl SystemEvents {
    pub fn start(&mut self, _callback: Box<dyn Fn(SystemEvent) + Send + Sync>) {}

    pub fn stop(&mut self) {}
}
