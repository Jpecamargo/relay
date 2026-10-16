//! dono: DIST-M
//! Atualização automática via `tauri-plugin-updater` (endpoint e chave em `tauri.conf.json`).
//! `check_updates` guarda a atualização encontrada para `install_update` e emite
//! `relay:update-available`.
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::events;

/// Última atualização encontrada por `check_updates`, consumida por `install_update`.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

/// Versão disponível, ou `None` se o app já está atualizado.
#[tauri::command(rename_all = "snake_case")]
pub async fn check_updates(app: AppHandle) -> Result<Option<String>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    let version = found.as_ref().map(|u| u.version.clone());
    if let Some(v) = &version {
        events::emit_update_available(&app, v.clone());
    }
    let pending = app.state::<PendingUpdate>();
    *pending.0.lock().map_err(|e| e.to_string())? = found;
    Ok(version)
}

/// Baixa e instala a atualização achada por `check_updates`, depois reinicia o app.
#[tauri::command(rename_all = "snake_case")]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<PendingUpdate>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or("nenhuma atualização pendente: rode check_updates antes")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart()
}
