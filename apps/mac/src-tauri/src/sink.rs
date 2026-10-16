//! dono: MI1
//! `CoreEvents` do app: cada evento do AppCore vira um `emit_*` do glossário.

use relay_core::types::{
    CommandResult, DisplaysPayload, LatencyStats, PairingEvent, PermissionsStatus, StatusSnapshot,
};
use tauri::AppHandle;

use crate::core::CoreEvents;
use crate::events;

pub struct TauriSink(pub AppHandle);

impl CoreEvents for TauriSink {
    fn status(&self, status: &StatusSnapshot) {
        events::emit_status(&self.0, status);
    }
    fn latency(&self, latency: &LatencyStats) {
        events::emit_latency(&self.0, latency);
    }
    fn displays(&self, displays: &DisplaysPayload) {
        events::emit_displays(&self.0, displays);
    }
    fn permissions(&self, permissions: &PermissionsStatus) {
        events::emit_permissions(&self.0, permissions);
    }
    fn secure_input(&self, active: bool) {
        events::emit_secure_input(&self.0, active);
    }
    fn pairing(&self, event: &PairingEvent) {
        events::emit_pairing(&self.0, event);
    }
    fn command_result(&self, result: &CommandResult) {
        events::emit_command_result(&self.0, result);
    }
}
