//! Controles do par. O PC controla o Mac pelo controller reverso (R1): trava, pausa e pulo de
//! tela têm o mesmo efeito do Mac. O que o PC não faz na 0.3 devolve `Err("unsupported")` (D45,
//! D46); a capacidade do par vem do snapshot (par 0.2 sem `peer_caps` vale o baseline do SO).

use relay_core::types::{Caps, CommandResult, Os, RemoteCommand, StatusSnapshot, error_code};
use tauri::State;

use crate::backend::AgentBackend;
use crate::state::AgentState;

pub(crate) fn unsupported() -> String {
    error_code::UNSUPPORTED.into()
}

/// Capacidades do par; sem `peer_caps` (0.2.x ou sem sessão) vale o baseline do SO dele.
fn peer_caps(status: &StatusSnapshot) -> Caps {
    status
        .peer_caps
        .unwrap_or_else(|| Caps::baseline(status.peer_os.unwrap_or(Os::Mac)))
}

/// `Err("unsupported")` se o par não tem `bit`.
fn require_peer(status: &StatusSnapshot, bit: u32) -> Result<(), String> {
    if peer_caps(status).has(bit) {
        Ok(())
    } else {
        Err(unsupported())
    }
}

pub(crate) fn run_command(backend: &dyn AgentBackend, _command: RemoteCommand) -> CommandResult {
    if let Err(e) = require_peer(&backend.status(), Caps::COMMANDS) {
        return CommandResult::err(0, e);
    }
    // O envio PC -> Mac ainda não existe: o Mac não executa comandos na 0.3 (D46).
    CommandResult::err(0, unsupported())
}

pub(crate) fn run_wake_peer(backend: &dyn AgentBackend) -> Result<(), String> {
    require_peer(&backend.status(), Caps::WOL_TARGET)?;
    // O PC não acorda o Mac na 0.3.
    Err(unsupported())
}

/// O PC pedir arquivos ao Mac (FILE_PULL no Windows) fica fora da 0.3 (D45).
pub(crate) fn run_receive_remote_files(backend: &dyn AgentBackend) -> Result<(), String> {
    if !backend.status().local_caps.has(Caps::FILE_PULL) {
        return Err(unsupported());
    }
    Err(unsupported())
}

#[tauri::command(rename_all = "snake_case")]
pub fn send_command(state: State<'_, AgentState>, command: RemoteCommand) -> CommandResult {
    run_command(&*state.backend, command)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_cursor_lock(state: State<'_, AgentState>, locked: bool) -> Result<(), String> {
    state.backend.set_locked(locked).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_sharing_paused(state: State<'_, AgentState>, paused: bool) -> Result<(), String> {
    state.backend.set_paused(paused).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn jump_to(state: State<'_, AgentState>, n: u8) -> Result<(), String> {
    state.backend.jump_to(n).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn clipboard_send_now(state: State<'_, AgentState>) -> Result<(), String> {
    state
        .backend
        .clipboard_send_now()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn receive_remote_files(state: State<'_, AgentState>) -> Result<(), String> {
    run_receive_remote_files(&*state.backend)
}

#[tauri::command(rename_all = "snake_case")]
pub fn wake_peer(state: State<'_, AgentState>) -> Result<(), String> {
    run_wake_peer(&*state.backend)
}

#[tauri::command(rename_all = "snake_case")]
pub fn reconnect(state: State<'_, AgentState>) -> Result<(), String> {
    state.backend.reconnect().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    fn with_peer_caps(features: Option<u32>) -> MockBackend {
        let b = MockBackend::default();
        b.edit_status(|s| {
            s.peer_caps = features.map(|features| Caps {
                os: Os::Mac,
                features,
            })
        });
        b
    }

    #[test]
    fn send_command_is_unsupported_without_the_commands_cap() {
        for b in [with_peer_caps(None), with_peer_caps(Some(Caps::VOLUME))] {
            let r = run_command(&b, RemoteCommand::GetMetrics);
            assert!(!r.ok);
            assert_eq!(r.error.as_deref(), Some("unsupported"));
        }
    }

    #[test]
    fn wake_peer_is_unsupported_without_wol_target() {
        for b in [with_peer_caps(None), with_peer_caps(Some(Caps::COMMANDS))] {
            assert_eq!(run_wake_peer(&b), Err("unsupported".to_string()));
        }
    }

    #[test]
    fn a_peer_without_caps_gets_the_baseline_of_its_os() {
        let mut s = MockBackend::default().status();
        s.peer_caps = None;
        s.peer_os = Some(Os::Windows);
        assert!(peer_caps(&s).has(Caps::COMMANDS));
        s.peer_os = None;
        assert!(!peer_caps(&s).has(Caps::COMMANDS));
    }

    #[test]
    fn receive_remote_files_is_unsupported_without_file_pull() {
        let b = MockBackend::default();
        assert!(!b.status().local_caps.has(Caps::FILE_PULL));
        assert_eq!(run_receive_remote_files(&b), Err("unsupported".to_string()));
    }

    #[test]
    fn lock_and_pause_reach_the_status() {
        let b = MockBackend::default();
        b.set_locked(true).unwrap();
        b.set_paused(true).unwrap();
        let s = b.status();
        assert!(s.locked && s.paused);
    }
}
