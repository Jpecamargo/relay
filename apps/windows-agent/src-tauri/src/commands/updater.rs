//! Updater do agente (D32): consulta o `latest.json` do release e instala a atualização.
use relay_core::types::UpdateAvailable;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Procura atualização; se houver, emite `relay:update-available` e devolve a versão.
#[tauri::command(rename_all = "snake_case")]
pub async fn agent_check_updates(app: AppHandle) -> Result<Option<UpdateAvailable>, String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(update.map(|u| {
        let payload = UpdateAvailable { version: u.version };
        let _ = app.emit("relay:update-available", payload.clone());
        payload
    }))
}

/// Baixa, instala (o NSIS reinicia o app) e encerra. Sem atualização pendente, não faz nada.
#[tauri::command(rename_all = "snake_case")]
pub async fn agent_install_update(app: AppHandle) -> Result<(), String> {
    let Some(update) = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}
