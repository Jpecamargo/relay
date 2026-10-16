//! dono: MI2 (casco: MAC1)
//! Ícone no menu bar: clique esquerdo alterna o painel (sob o ícone), clique direito abre o menu.

use relay_core::types::{
    CommandResult, ConnectionStatus, PowerAction, RemoteCommand, StatusSnapshot,
};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_positioner::{Position, WindowExt};

use crate::backend::AppBackend;
use crate::commands::system::open_main;
use crate::events::{emit_command_result, emit_status};
use crate::state::AppState;

/// Itens do menu que refletem o estado.
pub struct TrayHandles {
    status: MenuItem<Wry>,
    lock: CheckMenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
    tray: TrayIcon<Wry>,
}

/// Ícone cheio só com o PC conectado; qualquer outro estado usa a variante tracejada.
pub fn tray_connected(s: &StatusSnapshot) -> bool {
    matches!(s.status, ConnectionStatus::Connected)
}

pub fn status_label(s: &StatusSnapshot) -> String {
    let text = match s.status {
        ConnectionStatus::Disconnected => "Desconectado",
        ConnectionStatus::Discovering => "Procurando o PC",
        ConnectionStatus::Connecting => "Conectando",
        ConnectionStatus::Pairing => "Pareando",
        ConnectionStatus::Connected => "Conectado",
        ConnectionStatus::Reconnecting => "Reconectando",
        ConnectionStatus::IncompatibleVersion { .. } => "Versão incompatível",
    };
    match (&s.status, &s.peer_name) {
        (ConnectionStatus::Connected, Some(peer)) => format!("Estado: {text} a {peer}"),
        _ => format!("Estado: {text}"),
    }
}

impl TrayHandles {
    pub fn sync(&self, s: &StatusSnapshot) {
        let _ = self.status.set_text(status_label(s));
        let _ = self.lock.set_checked(s.locked);
        let _ = self.pause.set_checked(s.paused);
        let icon = if tray_connected(s) {
            tauri::include_image!("icons/tray-template@2x.png")
        } else {
            tauri::include_image!("icons/tray-template-off@2x.png")
        };
        let _ = self.tray.set_icon(Some(icon));
        let _ = self.tray.set_icon_as_template(true);
    }
}

fn toggle_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    let _ = panel.move_window(Position::TrayBottomCenter);
    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Ação do menu que fala com o backend (as demais abrem janela ou saem).
fn perform(backend: &dyn AppBackend, id: &str) -> Option<CommandResult> {
    let power = |action| RemoteCommand::Power {
        action,
        confirm: false,
    };
    match id {
        "lock" => {
            let _ = backend.set_cursor_lock(!backend.get_status().locked);
        }
        "pause" => {
            let _ = backend.set_sharing_paused(!backend.get_status().paused);
        }
        "clipboard" => {
            let _ = backend.clipboard_send_now();
        }
        "pc_wake" => {
            let _ = backend.wake_pc();
        }
        "reconnect" => {
            let _ = backend.reconnect();
        }
        "pc_lock" => return Some(backend.send_command(power(PowerAction::Lock))),
        "pc_suspend" => return Some(backend.send_command(power(PowerAction::Suspend))),
        _ => {}
    }
    None
}

const BACKEND_ACTIONS: [&str; 7] = [
    "lock",
    "pause",
    "clipboard",
    "pc_lock",
    "pc_suspend",
    "pc_wake",
    "reconnect",
];

/// Tudo que fala com o backend roda fora da thread principal: `send_command` espera o PC
/// (até 7 s) e o menu não pode travar.
fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "editor" => {
            let _ = open_main(app, Some("screens"));
        }
        "settings" => {
            let _ = open_main(app, Some("general"));
        }
        "quit" => app.exit(0),
        id if BACKEND_ACTIONS.contains(&id) => {
            let (app, id) = (app.clone(), id.to_owned());
            let backend = app.state::<AppState>().backend.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let result = perform(&*backend, &id);
                emit_status(&app, &backend.get_status());
                if let Some(r) = result {
                    emit_command_result(&app, &r);
                }
            });
        }
        _ => {}
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let snapshot = app.state::<AppState>().backend.get_status();
    let status = MenuItem::with_id(app, "status", status_label(&snapshot), false, None::<&str>)?;
    let lock = CheckMenuItem::with_id(
        app,
        "lock",
        "Travar cursor",
        true,
        snapshot.locked,
        None::<&str>,
    )?;
    let pause = CheckMenuItem::with_id(
        app,
        "pause",
        "Pausar compartilhamento",
        true,
        snapshot.paused,
        None::<&str>,
    )?;
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &lock,
            &pause,
            &item("clipboard", "Enviar clipboard")?,
            &PredefinedMenuItem::separator(app)?,
            &item("pc_lock", "Bloquear PC")?,
            &item("pc_suspend", "Suspender PC")?,
            &item("pc_wake", "Ligar PC")?,
            &item("reconnect", "Reconectar")?,
            &PredefinedMenuItem::separator(app)?,
            &item("editor", "Telas…")?,
            &item("settings", "Ajustes…")?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Sair")?,
        ],
    )?;
    let tray = TrayIconBuilder::with_id("tray")
        .icon(if tray_connected(&snapshot) {
            tauri::include_image!("icons/tray-template@2x.png")
        } else {
            tauri::include_image!("icons/tray-template-off@2x.png")
        })
        .icon_as_template(true)
        .tooltip("Relay")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_panel(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayHandles {
        status,
        lock,
        pause,
        tray,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AppBackend, MockBackend};

    #[test]
    fn status_label_names_the_peer_when_connected() {
        let s = MockBackend::default().get_status();
        assert_eq!(status_label(&s), "Estado: Conectado a DESKTOP-RELAY");
        let mut s = s;
        s.status = ConnectionStatus::Reconnecting;
        assert_eq!(status_label(&s), "Estado: Reconectando");
    }

    #[test]
    fn tray_icon_is_solid_only_when_connected() {
        let mut s = MockBackend::default().get_status();
        assert!(tray_connected(&s));
        s.status = ConnectionStatus::Reconnecting;
        assert!(!tray_connected(&s));
        s.status = ConnectionStatus::Disconnected;
        assert!(!tray_connected(&s));
    }

    #[test]
    fn menu_actions_toggle_state_and_send_power_commands() {
        let b = MockBackend::default();
        assert!(perform(&b, "lock").is_none());
        assert!(b.get_status().locked);
        perform(&b, "lock");
        assert!(!b.get_status().locked);
        perform(&b, "pause");
        assert!(b.get_status().paused);
        assert!(perform(&b, "pc_lock").is_some_and(|r| r.ok));
        assert!(perform(&b, "pc_suspend").is_some_and(|r| r.ok));
        for id in ["clipboard", "pc_wake", "reconnect", "desconhecido"] {
            assert!(perform(&b, id).is_none());
        }
    }
}
