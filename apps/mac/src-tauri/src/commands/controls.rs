//! dono: MI2 (casco: MAC1)
//! Controles remotos. `send_command` bloqueia até a resposta do PC (ou o timeout de 5 s), então
//! roda fora da thread principal; os demais comandos também, para nunca travar a UI nem o menu.

use std::time::Duration;

use relay_core::types::{
    CommandOutput, CommandResult, ConnectionStatus, RemoteCommand, SystemMetrics,
};
use tauri::{AppHandle, Manager, State};

use crate::backend::AppBackend;
use crate::events::{emit_command_result, emit_metrics, emit_status};
use crate::state::AppState;

/// Intervalo do `GetMetrics` enquanto o painel está visível.
pub const METRICS_INTERVAL: Duration = Duration::from_secs(2);

/// Envia o comando e espera o resultado; roda numa thread de bloqueio.
pub async fn run_command(app: &AppHandle, command: RemoteCommand) -> CommandResult {
    let backend = app.state::<AppState>().backend.clone();
    let result = tauri::async_runtime::spawn_blocking(move || backend.send_command(command))
        .await
        .unwrap_or_else(|_| CommandResult::err(0, "interno"));
    emit_command_result(app, &result);
    result
}

#[tauri::command(rename_all = "snake_case")]
pub async fn send_command(app: AppHandle, command: RemoteCommand) -> CommandResult {
    run_command(&app, command).await
}

#[tauri::command(rename_all = "snake_case", async)]
pub fn set_cursor_lock(
    app: AppHandle,
    state: State<'_, AppState>,
    locked: bool,
) -> Result<(), String> {
    state.backend.set_cursor_lock(locked)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case", async)]
pub fn set_sharing_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<(), String> {
    state.backend.set_sharing_paused(paused)?;
    emit_status(&app, &state.backend.get_status());
    Ok(())
}

#[tauri::command(rename_all = "snake_case", async)]
pub fn wake_pc(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.wake_pc()
}

#[tauri::command(rename_all = "snake_case", async)]
pub fn reconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.reconnect()
}

/// Uma amostra de métricas do PC (`None` se o comando falhou).
pub fn fetch_metrics(backend: &dyn AppBackend) -> Option<SystemMetrics> {
    metrics_of(backend.send_command(RemoteCommand::GetMetrics))
}

fn metrics_of(r: CommandResult) -> Option<SystemMetrics> {
    match r.output {
        Some(CommandOutput::Metrics(m)) if r.ok => Some(m),
        _ => None,
    }
}

fn panel_visible(app: &AppHandle) -> bool {
    app.get_webview_window("panel")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// `GetMetrics` a cada 2 s → `relay:metrics`, só com o painel visível e o PC conectado.
pub fn spawn_metrics_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(METRICS_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let backend = app.state::<AppState>().backend.clone();
            if !panel_visible(&app) || backend.get_status().status != ConnectionStatus::Connected {
                continue;
            }
            let sample = tauri::async_runtime::spawn_blocking(move || fetch_metrics(&*backend))
                .await
                .ok()
                .flatten();
            if let Some(m) = sample {
                emit_metrics(&app, &m);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn fetch_metrics_reads_the_metrics_output() {
        let b = MockBackend::default();
        assert_eq!(fetch_metrics(&b), Some(SystemMetrics::default()));
    }

    #[test]
    fn metrics_of_ignores_failures_and_other_outputs() {
        assert_eq!(metrics_of(CommandResult::err(1, "timeout")), None);
        assert_eq!(metrics_of(CommandResult::ok(1, None)), None);
        let text = CommandResult::ok(1, Some(CommandOutput::Text("x".into())));
        assert_eq!(metrics_of(text), None);
    }
}
