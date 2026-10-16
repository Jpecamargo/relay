//! dono: WA1
use tauri::AppHandle;

use crate::tray;
use tauri_plugin_autostart::ManagerExt;

/// Liga/desliga o início com o Windows (HKCU\...\Run, via plugin autostart).
#[tauri::command(rename_all = "snake_case")]
pub fn agent_set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())
}

/// A janela avisa que há (ou deixou de haver) atualização: a bandeja mostra "Atualização
/// disponível", que traz a janela à frente com o modal (a checagem e a instalação são do front).
#[tauri::command(rename_all = "snake_case")]
pub fn agent_set_update_available(app: AppHandle, version: Option<String>) {
    tray::set_update_available(&app, version.as_deref());
}
