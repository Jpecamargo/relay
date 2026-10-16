//! dono: MI2 (casco: MAC1)
use relay_core::types::{PermissionKind, PermissionsStatus};
use tauri::{AppHandle, Manager, State};

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

/// Labels aceitos: `panel`, `settings` e `editor`.
pub fn show_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("janela desconhecida: {label}"))?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_window(app: AppHandle, label: String) -> Result<(), String> {
    show_window(&app, &label)
}
