//! dono: MI2 (casco: MAC1)
//! Ícone no menu bar: clique esquerdo alterna o painel (sob o ícone), clique direito abre o menu.

use relay_core::types::{ConnectionStatus, RemoteCommand, StatusSnapshot};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_positioner::{Position, WindowExt};

use crate::commands::system::show_window;
use crate::events::emit_status;
use crate::state::AppState;

/// Itens do menu que refletem o estado.
pub struct TrayHandles {
    status: MenuItem<Wry>,
    lock: CheckMenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
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

fn on_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let backend = &state.backend;
    let power = |action| RemoteCommand::Power {
        action,
        confirm: false,
    };
    match id {
        "lock" => {
            let locked = !backend.get_status().locked;
            let _ = backend.set_cursor_lock(locked);
            emit_status(app, &backend.get_status());
        }
        "pause" => {
            let paused = !backend.get_status().paused;
            let _ = backend.set_sharing_paused(paused);
            emit_status(app, &backend.get_status());
        }
        "clipboard" => {
            let _ = backend.clipboard_send_now();
        }
        "pc_lock" => {
            backend.send_command(power(relay_core::types::PowerAction::Lock));
        }
        "pc_suspend" => {
            backend.send_command(power(relay_core::types::PowerAction::Suspend));
        }
        "pc_wake" => {
            let _ = backend.wake_pc();
        }
        "editor" => {
            let _ = show_window(app, "editor");
        }
        "settings" => {
            let _ = show_window(app, "settings");
        }
        "quit" => app.exit(0),
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
            &PredefinedMenuItem::separator(app)?,
            &item("editor", "Editor de telas…")?,
            &item("settings", "Configurações…")?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Sair")?,
        ],
    )?;
    app.manage(TrayHandles {
        status,
        lock,
        pause,
    });

    TrayIconBuilder::with_id("tray")
        .icon(tauri::include_image!("icons/tray-template@2x.png"))
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
}
