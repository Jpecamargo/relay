//! dono: MAC1
//! Casco do app Mac: só menu bar (D30), janelas `panel`/`settings`/`editor` de um único build
//! Vite, comandos do glossário contra `AppBackend`.

pub mod backend;
pub mod commands;
pub mod core;
pub mod events;
pub mod sink;
pub mod state;
pub mod tray;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, WindowEvent};

use backend::{AppBackend, MockBackend};
use commands::{audio, clipboard, config, controls, layout, pairing, status, system, updater};
use core::media::{MacMedia, RealMedia};
use core::{AppCore, CoreOptions, MacPlatform};
use sink::TauriSink;
use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = system::show_window(app, "settings");
        }))
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            status::get_status,
            status::list_discovered,
            pairing::pair_with_code,
            pairing::unpair,
            pairing::set_manual_address,
            layout::get_displays,
            layout::get_layout,
            layout::save_layout,
            config::get_config,
            config::save_config,
            controls::send_command,
            controls::set_cursor_lock,
            controls::set_sharing_paused,
            controls::wake_pc,
            controls::reconnect,
            clipboard::clipboard_send_now,
            clipboard::set_clipboard_sync,
            clipboard::receive_remote_files,
            audio::list_audio_devices,
            audio::set_audio_streams,
            audio::set_audio_devices,
            audio::set_stream_volume,
            system::check_permissions,
            system::open_permission_settings,
            system::set_launch_at_login,
            updater::check_updates,
            updater::install_update,
            system::open_window,
        ])
        .on_window_event(|window, event| match (window.label(), event) {
            ("panel", WindowEvent::Focused(false)) => {
                let _ = window.hide();
            }
            // Fechar settings/editor só esconde: a janela existe de novo ao reabrir.
            ("settings" | "editor", WindowEvent::CloseRequested { api, .. }) => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .setup(|app| {
            // D30: sem ícone no Dock.
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            // RELAY_MOCK_BACKEND=1: UI com dados de mentira, sem rede nem permissões.
            let mock = std::env::var_os("RELAY_MOCK_BACKEND").is_some();
            let backend: Arc<dyn AppBackend> = if mock {
                Arc::new(MockBackend::default())
            } else {
                let runtime = tauri::async_runtime::handle().inner().clone();
                let sink = Arc::new(TauriSink(app.handle().clone()));
                let media = RealMedia::new(Arc::new(MacMedia), sink.clone(), runtime.clone());
                Arc::new(AppCore::start(
                    Arc::new(MacPlatform),
                    sink,
                    media,
                    CoreOptions::default(),
                    runtime,
                ))
            };
            app.manage(AppState::new(backend.clone()));
            app.manage(updater::PendingUpdate::default());
            tray::setup(app.handle())?;
            controls::spawn_metrics_poller(app.handle().clone());

            if mock {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_millis(500));
                    loop {
                        tick.tick().await;
                        events::emit_latency(&handle, &backend.latency());
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao executar o app Tauri");
}

#[cfg(test)]
mod tests {
    /// O tauri.conf.json declara as três janelas do glossário e nenhuma `main`.
    #[test]
    fn conf_declares_panel_settings_editor() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = conf["app"]["windows"].as_array().unwrap();
        let labels: Vec<_> = windows
            .iter()
            .map(|w| w["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["panel", "settings", "editor"]);
        let panel = &windows[0];
        assert_eq!(
            (panel["width"].as_u64(), panel["height"].as_u64()),
            (Some(360), Some(520))
        );
        assert_eq!(panel["decorations"], false);
        assert_eq!(panel["alwaysOnTop"], true);
        assert_eq!(panel["skipTaskbar"], true);
    }
}
