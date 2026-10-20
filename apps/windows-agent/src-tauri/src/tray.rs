//! dono: WA1
//! Ícone da bandeja e menu: Estado (label), Mostrar código de pareamento, Abrir janela, Pausar, Sair.

use relay_core::types::{ConnectionStatus, StatusSnapshot};
use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, Manager, Runtime,
    image::Image,
    include_image,
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

use crate::state::AgentState;

const ID_STATUS: &str = "status";
const ID_PAIRING: &str = "pairing";
const ID_OPEN: &str = "open";
const ID_PAUSE: &str = "pause";
const ID_UPDATE: &str = "update";
const ID_QUIT: &str = "quit";

/// Evento que pede o modal da atualização à janela (mesmo nome de `@relay/ui`).
pub(crate) const OPEN_UPDATE_EVENT: &str = "relay:open-update";

/// Itens que mudam com o estado; guardados para atualizar o texto.
struct TrayItems<R: Runtime> {
    status: MenuItem<R>,
    pause: MenuItem<R>,
    menu: Menu<R>,
    /// Item "Atualização disponível"; só fica no menu enquanto `update_shown`.
    update: MenuItem<R>,
    update_shown: Mutex<bool>,
}

pub fn status_label(s: &StatusSnapshot) -> String {
    let text = match s.status {
        ConnectionStatus::Disconnected => "desconectado".to_string(),
        ConnectionStatus::Discovering => "procurando".to_string(),
        ConnectionStatus::Connecting => "conectando".to_string(),
        ConnectionStatus::Pairing => "pareando".to_string(),
        ConnectionStatus::Connected => match &s.peer_name {
            Some(n) => format!("conectado a {n}"),
            None => "conectado".to_string(),
        },
        ConnectionStatus::Reconnecting => "reconectando".to_string(),
        ConnectionStatus::IncompatibleVersion { .. } => "versão incompatível".to_string(),
    };
    if s.paused {
        format!("Estado: {text} (pausado)")
    } else {
        format!("Estado: {text}")
    }
}

pub fn pause_label(paused: bool) -> &'static str {
    if paused { "Retomar" } else { "Pausar" }
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Mostra (com a versão) ou tira do menu o item "Atualização disponível".
pub fn set_update_available<R: Runtime>(app: &AppHandle<R>, version: Option<&str>) {
    let Some(items) = app.try_state::<TrayItems<R>>() else {
        return;
    };
    let Ok(mut shown) = items.update_shown.lock() else {
        return;
    };
    match version {
        Some(v) => {
            let _ = items.update.set_text(update_label(v));
            if !*shown
                && items
                    .menu
                    .insert(&items.update as &dyn IsMenuItem<R>, 2)
                    .is_ok()
            {
                *shown = true;
            }
        }
        None => {
            if *shown
                && items
                    .menu
                    .remove(&items.update as &dyn IsMenuItem<R>)
                    .is_ok()
            {
                *shown = false;
            }
        }
    }
}

pub fn update_label(version: &str) -> String {
    format!("Atualização {version} disponível")
}

/// Reflete o estado atual do backend no menu.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let (Some(state), Some(items)) = (
        app.try_state::<AgentState>(),
        app.try_state::<TrayItems<R>>(),
    ) else {
        return;
    };
    let snap = state.backend.status();
    let label = match state.backend.error() {
        Some(e) => format!("Erro: {e} (tentando de novo)"),
        None => status_label(&snap),
    };
    let _ = items.status.set_text(label);
    let _ = items.pause.set_text(pause_label(snap.paused));
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let snap = app.state::<AgentState>().backend.status();
    let status = MenuItem::with_id(app, ID_STATUS, status_label(&snap), false, None::<&str>)?;
    let pairing = MenuItem::with_id(
        app,
        ID_PAIRING,
        "Mostrar código de pareamento",
        true,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, ID_OPEN, "Abrir janela", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, ID_PAUSE, pause_label(snap.paused), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Sair", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep, &pairing, &open, &pause, &sep2, &quit])?;
    let update = MenuItem::with_id(app, ID_UPDATE, update_label(""), true, None::<&str>)?;
    app.manage(TrayItems {
        status,
        pause,
        menu: menu.clone(),
        update,
        update_shown: Mutex::new(false),
    });

    let icon: Image<'static> = include_image!("icons/icon.ico");
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .tooltip("Relay Agent")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            // A janela gera o código (agent_new_pairing_code); o menu só a traz à frente.
            ID_PAIRING | ID_OPEN => show_main_window(app),
            ID_UPDATE => {
                show_main_window(app);
                let _ = app.emit_to("main", OPEN_UPDATE_EVENT, ());
            }
            ID_PAUSE => {
                let backend = &app.state::<AgentState>().backend;
                let paused = backend.status().paused;
                if let Err(e) = backend.set_paused(!paused) {
                    tracing::warn!("falha ao alternar pausa: {e}");
                }
                refresh(app);
            }
            ID_QUIT => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AgentBackend, MockBackend};

    #[test]
    fn label_reflete_conexao_e_pausa() {
        let b = MockBackend::default();
        assert_eq!(status_label(&b.status()), "Estado: desconectado");
        let mut s = b.status();
        s.status = ConnectionStatus::Connected;
        s.peer_name = Some("MacBook".into());
        s.paused = true;
        assert_eq!(status_label(&s), "Estado: conectado a MacBook (pausado)");
        assert_eq!(pause_label(true), "Retomar");
        assert_eq!(pause_label(false), "Pausar");
    }

    #[test]
    fn item_de_atualizacao_traz_a_versao() {
        assert_eq!(update_label("0.2.0"), "Atualização 0.2.0 disponível");
    }
}
