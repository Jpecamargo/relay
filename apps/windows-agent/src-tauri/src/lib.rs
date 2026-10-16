//! dono: WA1
//! Casco do agente Windows: bandeja, janela oculta, instância única, início com o Windows e
//! registro dos comandos `agent_*` contra um `AgentBackend` (por ora o `MockBackend`).

pub mod backend;
pub mod commands;
pub mod state;
pub mod tray;

use std::sync::Arc;

use tauri::WindowEvent;
use tauri_plugin_autostart::MacosLauncher;

use crate::backend::MockBackend;
use crate::state::AgentState;

pub fn run() {
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
        .manage(AgentState::new(Arc::new(MockBackend::default())))
        .invoke_handler(tauri::generate_handler![
            commands::status::agent_get_status,
            commands::status::agent_get_service_status,
            commands::pairing::agent_new_pairing_code,
            commands::pairing::agent_revoke_pair,
            commands::config::agent_get_config,
            commands::config::agent_save_config,
            commands::system::agent_set_autostart,
            commands::updater::agent_check_updates,
            commands::updater::agent_install_update,
        ])
        .setup(|app| {
            #[cfg(windows)]
            relay_platform_windows::dpi::set_dpi_awareness();
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
