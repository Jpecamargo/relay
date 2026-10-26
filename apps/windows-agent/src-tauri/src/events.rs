//! dono: WI1
//! Eventos core→UI do agente (nomes do glossário) emitidos pelo Tauri.

use relay_core::types::{DisplaysPayload, LatencyStats, LayoutBook, PairingEvent, StatusSnapshot};
use tauri::{AppHandle, Emitter};

use crate::core::UiEvents;

pub struct TauriEvents(pub AppHandle);

impl UiEvents for TauriEvents {
    fn status(&self, snapshot: &StatusSnapshot) {
        let _ = self.0.emit("relay:status", snapshot);
    }

    fn latency(&self, stats: &LatencyStats) {
        let _ = self.0.emit("relay:latency", stats);
    }

    fn pairing(&self, event: &PairingEvent) {
        let _ = self.0.emit("relay:pairing", event);
    }

    fn layout(&self, book: &LayoutBook) {
        let _ = self.0.emit("relay:layout", book);
    }

    fn displays(&self, displays: &DisplaysPayload) {
        let _ = self.0.emit("relay:displays", displays);
    }

    fn update_requested(&self, _version: &str) {
        crate::tray::show_main_window(&self.0);
        let _ = self.0.emit_to("main", crate::tray::OPEN_UPDATE_EVENT, ());
    }
}
