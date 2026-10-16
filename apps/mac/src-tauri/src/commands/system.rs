//! dono: MI2 (casco: MAC1)
use relay_core::types::{PermissionKind, PermissionsStatus};
use tauri::{ActivationPolicy, AppHandle, Emitter, Manager, State};

use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn check_permissions(state: State<'_, AppState>) -> PermissionsStatus {
    state.backend.check_permissions()
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_permission_settings(
    state: State<'_, AppState>,
    kind: PermissionKind,
) -> Result<(), String> {
    state.backend.open_permission_settings(kind)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_launch_at_login(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.backend.set_launch_at_login(enabled)
}

/// Labels aceitos: `panel` e `main`. A janela principal traz o app ao Dock enquanto estiver
/// aberta (ver `hide_main`).
pub fn show_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("janela desconhecida: {label}"))?;
    if label == "main" {
        let _ = app.set_activation_policy(ActivationPolicy::Regular);
        let _ = window.unminimize();
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

/// Abre a janela principal; `section` (id de seção da UI) é enviado em `relay:navigate`.
pub fn open_main(app: &AppHandle, section: Option<&str>) -> Result<(), String> {
    show_window(app, "main")?;
    if let Some(section) = section {
        app.emit_to("main", "relay:navigate", section)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Esconde a janela principal e devolve o app ao modo só-menu-bar (sem Dock, D30).
pub fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let _ = app.set_activation_policy(ActivationPolicy::Accessory);
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_window(app: AppHandle, label: String) -> Result<(), String> {
    show_window(&app, &label)
}

#[tauri::command(rename_all = "snake_case")]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
