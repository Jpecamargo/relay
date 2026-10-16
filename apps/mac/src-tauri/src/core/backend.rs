//! dono: MI1
//! `AppBackend` real: o `AppCore` atrás dos comandos Tauri. Mídia (áudio, clipboard, arquivos)
//! só chama os ganchos do MI3.

use std::sync::mpsc;
use std::time::Duration;

use relay_core::types::{
    AudioDevices, CommandResult, Config, DiscoveredPeer, DisplaysPayload, LatencyStats,
    LayoutProfile, PermissionKind, PermissionsStatus, RemoteCommand, SaveLayoutResult,
    StatusSnapshot,
};
use tokio::sync::oneshot;

use super::engine::CoreCmd;
use super::net::NetCmd;
use super::{AppCore, Platform};
use crate::backend::{AppBackend, BackendResult, BoxFuture};

/// Um pouco mais que o timeout do dispatcher (5 s), que sempre responde antes.
const COMMAND_WAIT: Duration = Duration::from_secs(7);

impl<P: Platform> AppBackend for AppCore<P> {
    fn get_status(&self) -> StatusSnapshot {
        self.status()
    }

    fn list_discovered(&self) -> Vec<DiscoveredPeer> {
        self.discovered()
    }

    fn pair_with_code(
        &self,
        code: String,
        address: Option<String>,
    ) -> BoxFuture<BackendResult<()>> {
        let tx = self.inner.net_tx.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            tx.send(NetCmd::Pair {
                code,
                address,
                reply,
            })
            .map_err(|_| "O Relay está encerrando.".to_string())?;
            rx.await
                .map_err(|_| "O Relay está encerrando.".to_string())?
        })
    }

    fn unpair(&self) -> BackendResult<()> {
        AppCore::unpair(self)
    }

    fn set_manual_address(&self, address: Option<String>) -> BackendResult<()> {
        AppCore::set_manual_address(self, address)
    }

    fn get_displays(&self) -> DisplaysPayload {
        self.displays()
    }

    fn get_layout(&self) -> LayoutProfile {
        self.layout()
    }

    fn save_layout(&self, profile: LayoutProfile) -> BackendResult<SaveLayoutResult> {
        AppCore::save_layout(self, profile)
    }

    fn get_config(&self) -> Config {
        self.config()
    }

    fn save_config(&self, config: Config) -> BackendResult<()> {
        AppCore::save_config(self, config)
    }

    /// Bloqueia até a resposta do PC (ou `timeout`): chame fora da thread principal.
    fn send_command(&self, command: RemoteCommand) -> CommandResult {
        let (reply, rx) = mpsc::channel();
        self.inner.core(CoreCmd::Command {
            cmd: command,
            reply: Some(reply),
        });
        rx.recv_timeout(COMMAND_WAIT)
            .unwrap_or_else(|_| CommandResult::err(0, relay_core::types::error_code::TIMEOUT))
    }

    fn set_cursor_lock(&self, locked: bool) -> BackendResult<()> {
        AppCore::set_cursor_lock(self, locked);
        Ok(())
    }

    fn set_sharing_paused(&self, paused: bool) -> BackendResult<()> {
        AppCore::set_sharing_paused(self, paused);
        Ok(())
    }

    fn clipboard_send_now(&self) -> BackendResult<()> {
        self.inner.media.clipboard_send_now();
        Ok(())
    }

    fn set_clipboard_sync(&self, enabled: bool) -> BackendResult<()> {
        let cfg = self.inner.mutate(|s| {
            s.status.clipboard_sync = enabled;
            s.config.clipboard.enabled = enabled;
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        self.inner.core(CoreCmd::ApplyConfig(Box::new(cfg)));
        Ok(())
    }

    fn receive_remote_files(&self) -> BackendResult<()> {
        self.inner.media.receive_remote_files();
        Ok(())
    }

    fn wake_pc(&self) -> BackendResult<()> {
        self.inner.wake_pc()
    }

    fn reconnect(&self) -> BackendResult<()> {
        AppCore::reconnect(self);
        Ok(())
    }

    fn list_audio_devices(&self) -> AudioDevices {
        AudioDevices::default()
    }

    fn set_audio_streams(&self, pc_to_mac: bool, mac_to_pc: bool) -> BackendResult<()> {
        self.inner.mutate(|s| {
            s.status.audio = relay_core::types::AudioState {
                pc_to_mac,
                mac_to_pc,
            }
        });
        self.inner.media.set_audio_streams(pc_to_mac, mac_to_pc);
        Ok(())
    }

    fn set_audio_devices(
        &self,
        output_id: Option<String>,
        input_id: Option<String>,
    ) -> BackendResult<()> {
        let cfg = self.inner.mutate(|s| {
            s.config.audio.mac_output_id = output_id;
            s.config.audio.mac_input_id = input_id;
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        Ok(())
    }

    fn set_stream_volume(&self, volume: f32) -> BackendResult<()> {
        let cfg = self.inner.mutate(|s| {
            s.config.audio.stream_volume = volume.clamp(0.0, 1.0);
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        Ok(())
    }

    fn check_permissions(&self) -> PermissionsStatus {
        self.permissions()
    }

    fn open_permission_settings(&self, kind: PermissionKind) -> BackendResult<()> {
        self.inner.platform.open_permission_settings(kind);
        Ok(())
    }

    fn set_launch_at_login(&self, enabled: bool) -> BackendResult<()> {
        self.inner.platform.set_launch_at_login(enabled)?;
        let cfg = self.inner.mutate(|s| {
            s.config.general.launch_at_login = enabled;
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        Ok(())
    }

    fn latency(&self) -> LatencyStats {
        AppCore::latency(self)
    }
}
