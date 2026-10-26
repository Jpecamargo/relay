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

/// Intervalo do `GetMetrics` enquanto o painel ou a janela principal está visível.
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
pub fn wake_peer(state: State<'_, AppState>) -> Result<(), String> {
    state.backend.wake_peer()
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

fn window_visible(app: &AppHandle, label: &str) -> bool {
    app.get_webview_window(label)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Há alguém olhando as métricas: o painel ou a janela principal (que esconde o painel).
fn metrics_wanted(panel: bool, main: bool, connected: bool) -> bool {
    (panel || main) && connected
}

/// `GetMetrics` a cada 2 s → `relay:metrics`, só com o painel ou a janela principal visível e o PC conectado.
pub fn spawn_metrics_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(METRICS_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let backend = app.state::<AppState>().backend.clone();
            let connected = backend.get_status().status == ConnectionStatus::Connected;
            if !metrics_wanted(
                window_visible(&app, "panel"),
                window_visible(&app, "main"),
                connected,
            ) {
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
    fn metrics_wanted_with_panel_or_main_window_while_connected() {
        assert!(metrics_wanted(true, false, true));
        assert!(metrics_wanted(false, true, true));
        assert!(!metrics_wanted(false, false, true));
        assert!(!metrics_wanted(true, true, false));
    }

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

/// Pulo de tela pelo menu do painel (mesmo efeito do atalho ⌃⌥n).
#[tauri::command(rename_all = "snake_case", async)]
pub fn jump_to(state: State<'_, AppState>, n: u8) -> Result<(), String> {
    state.backend.jump_to(n)
}
