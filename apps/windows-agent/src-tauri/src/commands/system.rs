//! dono: WA1
use tauri::AppHandle;
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
