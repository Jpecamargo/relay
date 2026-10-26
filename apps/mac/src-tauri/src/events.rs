//! dono: MAC1
//! Helpers `emit_*` core→UI, um por evento do glossário. `relay:latency` no máximo 2 Hz e
//! `relay:transfer-progress` no máximo 10 Hz: a limitação é de quem chama.

use relay_core::types::{
    CommandResult, DisplaysPayload, LatencyStats, LayoutBook, PairingEvent, PermissionsStatus,
    SecureInputPayload, StatusSnapshot, SystemMetrics, TransferProgress, UpdateAvailable,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::tray::TrayHandles;

fn emit<S: Serialize + Clone>(app: &AppHandle, name: &str, payload: S) {
    if let Err(e) = app.emit(name, payload) {
        tracing::warn!("falha ao emitir {name}: {e}");
    }
}

/// Também atualiza o menu da bandeja (label de estado e checks).
pub fn emit_status(app: &AppHandle, s: &StatusSnapshot) {
    emit(app, "relay:status", s);
    if let Some(tray) = tauri::Manager::try_state::<TrayHandles>(app) {
        tray.sync(s);
    }
}
pub fn emit_latency(app: &AppHandle, s: &LatencyStats) {
    emit(app, "relay:latency", s);
}
pub fn emit_displays(app: &AppHandle, d: &DisplaysPayload) {
    emit(app, "relay:displays", d);
}
pub fn emit_layout(app: &AppHandle, book: &LayoutBook) {
    emit(app, "relay:layout", book);
}
pub fn emit_command_result(app: &AppHandle, r: &CommandResult) {
    emit(app, "relay:command-result", r);
}
pub fn emit_transfer_progress(app: &AppHandle, p: &TransferProgress) {
    emit(app, "relay:transfer-progress", p);
}
pub fn emit_secure_input(app: &AppHandle, active: bool) {
    emit(app, "relay:secure-input", SecureInputPayload { active });
}
pub fn emit_permissions(app: &AppHandle, p: &PermissionsStatus) {
    emit(app, "relay:permissions", p);
}
pub fn emit_metrics(app: &AppHandle, m: &SystemMetrics) {
    emit(app, "relay:metrics", m);
}
pub fn emit_pairing(app: &AppHandle, p: &PairingEvent) {
    emit(app, "relay:pairing", p);
}
pub fn emit_update_available(app: &AppHandle, version: &str) {
    emit(
        app,
        "relay:update-available",
        UpdateAvailable {
            version: version.to_owned(),
        },
    );
}
