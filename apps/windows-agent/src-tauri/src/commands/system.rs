//! dono: WA1
use relay_core::types::{PermissionKind, PermissionState, PermissionsStatus};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use super::controls::unsupported;
use crate::backend::AgentBackend;
use crate::state::AgentState;
use crate::tray;

/// O Windows não pede permissão de acessibilidade nem de monitoramento de entrada.
fn windows_permissions() -> PermissionsStatus {
    PermissionsStatus {
        accessibility: PermissionState::Granted,
        input_monitoring: PermissionState::Granted,
    }
}

/// Liga o início com o Windows e só então grava o Config; se o autostart falha, o Config fica.
fn apply_launch_at_login(
    backend: &dyn AgentBackend,
    enabled: bool,
    autostart: impl FnOnce(bool) -> Result<(), String>,
) -> Result<(), String> {
    autostart(enabled)?;
    let mut config = backend.config();
    config.general.launch_at_login = enabled;
    backend.save_config(config).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn check_permissions() -> PermissionsStatus {
    windows_permissions()
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_permission_settings(_kind: PermissionKind) -> Result<(), String> {
    Err(unsupported())
}

/// Início com o Windows (HKCU\...\Run, via plugin autostart).
#[tauri::command(rename_all = "snake_case")]
pub fn set_launch_at_login(
    app: AppHandle,
    state: State<'_, AgentState>,
    enabled: bool,
) -> Result<(), String> {
    apply_launch_at_login(&*state.backend, enabled, |on| {
        let launcher = app.autolaunch();
        if on {
            launcher.enable()
        } else {
            launcher.disable()
        }
        .map_err(|e| e.to_string())
    })
}

/// Labels aceitos: `panel` e `main`.
#[tauri::command(rename_all = "snake_case")]
pub fn open_window(app: AppHandle, label: String) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("janela desconhecida: {label}"))?;
    let _ = window.unminimize();
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// A janela avisa que há (ou deixou de haver) atualização: a bandeja mostra "Atualização
/// disponível", que traz a janela à frente com o modal (a checagem e a instalação são do front).
#[tauri::command(rename_all = "snake_case")]
pub fn set_update_available(app: AppHandle, version: Option<String>) {
    tray::set_update_available(&app, version.as_deref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn check_permissions_grants_both() {
        let p = windows_permissions();
        assert_eq!(p.accessibility, PermissionState::Granted);
        assert_eq!(p.input_monitoring, PermissionState::Granted);
    }

    #[test]
    fn launch_at_login_enables_the_autostart_and_saves_the_config() {
        let b = MockBackend::default();
        let mut calls = Vec::new();
        apply_launch_at_login(&b, true, |on| {
            calls.push(on);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, [true]);
        assert!(b.config().general.launch_at_login);
        apply_launch_at_login(&b, false, |_| Ok(())).unwrap();
        assert!(!b.config().general.launch_at_login);
    }

    #[test]
    fn launch_at_login_keeps_the_config_when_the_autostart_fails() {
        let b = MockBackend::default();
        let r = apply_launch_at_login(&b, true, |_| Err("registro".into()));
        assert_eq!(r, Err("registro".to_string()));
        assert!(!b.config().general.launch_at_login);
    }
}
