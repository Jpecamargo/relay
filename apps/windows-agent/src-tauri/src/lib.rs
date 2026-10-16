//! dono: WA1
//! Casco do agente Windows: bandeja, janela oculta, instância única, início com o Windows e
//! registro dos comandos `agent_*` contra o `AgentBackend` real (`core::AgentCore`).

pub mod backend;
pub mod commands;
pub mod core;
pub mod events;
pub mod logging;
pub mod state;
pub mod tray;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::core::{AgentCore, ResilientBackend, ReverseDeps, real};
use crate::events::TauriEvents;
use crate::state::AgentState;

pub fn run() {
    let _ = logging::init();
    // Antes de qualquer janela: per-monitor v2 para que as coordenadas sejam pixels físicos.
    relay_platform_windows::dpi::set_dpi_awareness();
    tauri::Builder::default()
        // Segunda execução só traz a janela da instância existente à frente.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::status::agent_get_status,
            commands::status::agent_get_service_status,
            commands::pairing::agent_new_pairing_code,
            commands::pairing::agent_revoke_pair,
            commands::config::agent_get_config,
            commands::config::agent_save_config,
            commands::system::agent_set_autostart,
            commands::system::agent_set_update_available,
        ])
        .setup(|app| {
            let config = relay_core::config::load_or_default(&relay_core::config::config_path())
                .unwrap_or_default();
            let events = Arc::new(TauriEvents(app.handle().clone()));
            let (start_events, start_config) = (events, config.clone());
            let refresh = app.handle().clone();
            // Falha ao subir (porta ocupada, Credential Manager) não derruba o app: bandeja em
            // estado de erro, nova tentativa a cada 5 s e a cada pedido de pareamento.
            let backend = ResilientBackend::new(
                Box::new(move || {
                    AgentCore::start_with_reverse(
                        real::real_deps(),
                        start_config.clone(),
                        start_events.clone(),
                        Some(ReverseDeps::real()),
                    )
                }),
                config,
                Box::new(move || tray::refresh(&refresh)),
                true,
            );
            app.manage(AgentState::new(backend));
            tray::setup(app.handle())?;
            Ok(())
        })
        // Fechar a janela só a esconde: o agente continua na bandeja.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("erro ao executar o app Tauri");
}
