//! dono: MAC1 (MI1: trait sem os métodos de atualização; o app usa o `AppCore` real)
//! `AppBackend`: a fronteira entre os comandos Tauri e o core. O `MockBackend` fica para
//! desenvolver a UI (`RELAY_MOCK_BACKEND=1`) e para testes.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use relay_core::types::{
    AudioDevice, AudioDevices, AudioState, CommandOutput, CommandResult, Config, ConnectionStatus,
    DiscoveredPeer, DisplayInfo, DisplaysPayload, LatencyStats, LayoutProfile, Machine,
    PermissionKind, PermissionState, PermissionsStatus, PlacedDisplay, ProfileKey, RectMm,
    RemoteCommand, SaveLayoutResult, StatusSnapshot,
};

pub type BackendResult<T> = Result<T, String>;

/// Futuro `'static` para os comandos que esperam a rede (pareamento).
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub trait AppBackend: Send + Sync {
    fn get_status(&self) -> StatusSnapshot;
    fn list_discovered(&self) -> Vec<DiscoveredPeer>;
    /// `address` é `host`, `host:porta` ou `None` (usa o PC descoberto).
    fn pair_with_code(&self, code: String, address: Option<String>)
    -> BoxFuture<BackendResult<()>>;
    fn unpair(&self) -> BackendResult<()>;
    fn set_manual_address(&self, address: Option<String>) -> BackendResult<()>;
    fn get_displays(&self) -> DisplaysPayload;
    fn get_layout(&self) -> LayoutProfile;
    fn save_layout(&self, profile: LayoutProfile) -> BackendResult<SaveLayoutResult>;
    fn get_config(&self) -> Config;
    fn save_config(&self, config: Config) -> BackendResult<()>;
    fn send_command(&self, command: RemoteCommand) -> CommandResult;
    fn set_cursor_lock(&self, locked: bool) -> BackendResult<()>;
    fn set_sharing_paused(&self, paused: bool) -> BackendResult<()>;
    fn clipboard_send_now(&self) -> BackendResult<()>;
    fn set_clipboard_sync(&self, enabled: bool) -> BackendResult<()>;
    fn receive_remote_files(&self) -> BackendResult<()>;
    fn wake_pc(&self) -> BackendResult<()>;
    /// Derruba e refaz a sessão com o PC (também sai de `IncompatibleVersion`).
    fn reconnect(&self) -> BackendResult<()>;
    fn list_audio_devices(&self) -> AudioDevices;
    fn set_audio_streams(&self, pc_to_mac: bool, mac_to_pc: bool) -> BackendResult<()>;
    fn set_audio_devices(
        &self,
        output_id: Option<String>,
        input_id: Option<String>,
    ) -> BackendResult<()>;
    fn set_stream_volume(&self, volume: f32) -> BackendResult<()>;
    fn check_permissions(&self) -> PermissionsStatus;
    fn open_permission_settings(&self, kind: PermissionKind) -> BackendResult<()>;
    fn set_launch_at_login(&self, enabled: bool) -> BackendResult<()>;
    /// Amostra atual de latência (o emissor a 2 Hz chama isto).
    fn latency(&self) -> LatencyStats;
}

struct MockState {
    status: StatusSnapshot,
    config: Config,
    layout: LayoutProfile,
}

/// Dados plausíveis para desenvolver a UI sem rede nem permissões.
pub struct MockBackend {
    state: Mutex<MockState>,
}

fn display(
    id: &str,
    name: &str,
    machine: Machine,
    size_px: (u32, u32),
    scale: f32,
    size_mm: (f32, f32),
    builtin: bool,
) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: name.into(),
        machine,
        origin_px: (0, 0),
        size_px,
        scale,
        size_mm: Some(size_mm),
        is_primary: true,
        is_builtin: builtin,
    }
}

pub fn mock_displays() -> DisplaysPayload {
    DisplaysPayload {
        mac: vec![display(
            "mac-builtin",
            "MacBook Pro",
            Machine::Mac,
            (1512, 982),
            2.0,
            (302.0, 196.0),
            true,
        )],
        windows: vec![
            display(
                "win-dell-1",
                "Dell U2723QE",
                Machine::Windows,
                (3840, 2160),
                1.5,
                (597.0, 336.0),
                false,
            ),
            display(
                "win-dell-2",
                "Dell U2723QE",
                Machine::Windows,
                (3840, 2160),
                1.5,
                (597.0, 336.0),
                false,
            ),
        ],
    }
}

fn mock_layout() -> LayoutProfile {
    let placed = |id: &str, machine, x, y, w, h| PlacedDisplay {
        display_id: id.into(),
        machine,
        rect_mm: RectMm { x, y, w, h },
        sensitivity: 1.0,
        rotation: None,
    };
    LayoutProfile {
        key: ProfileKey(vec![
            "mac-builtin".into(),
            "win-dell-1".into(),
            "win-dell-2".into(),
        ]),
        name: "Mesa".into(),
        displays: vec![
            placed("mac-builtin", Machine::Mac, 0.0, 140.0, 302.0, 196.0),
            placed("win-dell-1", Machine::Windows, 302.0, 0.0, 597.0, 336.0),
            placed("win-dell-2", Machine::Windows, 899.0, 0.0, 597.0, 336.0),
        ],
        windows_detached: false,
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            state: Mutex::new(MockState {
                status: StatusSnapshot {
                    status: ConnectionStatus::Connected,
                    peer_name: Some("DESKTOP-RELAY".into()),
                    focus: Machine::Mac,
                    locked: false,
                    paused: false,
                    secure_input: false,
                    clipboard_sync: true,
                    audio: AudioState {
                        pc_to_mac: false,
                        mac_to_pc: false,
                    },
                },
                config: Config::default(),
                layout: mock_layout(),
            }),
        }
    }
}

impl MockBackend {
    fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        f(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

fn device(id: &str, name: &str, is_default: bool) -> AudioDevice {
    AudioDevice {
        id: id.into(),
        name: name.into(),
        is_default,
    }
}

impl AppBackend for MockBackend {
    fn get_status(&self) -> StatusSnapshot {
        self.with(|s| s.status.clone())
    }
    fn list_discovered(&self) -> Vec<DiscoveredPeer> {
        vec![DiscoveredPeer {
            name: "DESKTOP-RELAY".into(),
            addrs: vec!["192.168.0.20".into()],
            port: relay_core::types::consts::PORT_CONTROL,
            device_id: "a1b2c3d4e5f60718".into(),
            protocol_version: relay_core::types::consts::PROTOCOL_VERSION,
        }]
    }
    fn pair_with_code(
        &self,
        _code: String,
        _address: Option<String>,
    ) -> BoxFuture<BackendResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn unpair(&self) -> BackendResult<()> {
        self.with(|s| {
            s.status.status = ConnectionStatus::Disconnected;
            s.status.peer_name = None;
        });
        Ok(())
    }
    fn set_manual_address(&self, _address: Option<String>) -> BackendResult<()> {
        Ok(())
    }
    fn get_displays(&self) -> DisplaysPayload {
        mock_displays()
    }
    fn get_layout(&self) -> LayoutProfile {
        self.with(|s| s.layout.clone())
    }
    fn save_layout(&self, profile: LayoutProfile) -> BackendResult<SaveLayoutResult> {
        self.with(|s| s.layout = profile);
        Ok(SaveLayoutResult::default())
    }
    fn get_config(&self) -> Config {
        self.with(|s| s.config.clone())
    }
    fn save_config(&self, config: Config) -> BackendResult<()> {
        self.with(|s| s.config = config);
        Ok(())
    }
    fn send_command(&self, command: RemoteCommand) -> CommandResult {
        let output = match command {
            RemoteCommand::ListOutputDevices => Some(CommandOutput::Devices(vec![
                device("speakers", "Alto-falantes", true),
                device("headset", "Headset", false),
            ])),
            RemoteCommand::GetMetrics => Some(CommandOutput::Metrics(Default::default())),
            _ => None,
        };
        CommandResult::ok(0, output)
    }
    fn set_cursor_lock(&self, locked: bool) -> BackendResult<()> {
        self.with(|s| s.status.locked = locked);
        Ok(())
    }
    fn set_sharing_paused(&self, paused: bool) -> BackendResult<()> {
        self.with(|s| s.status.paused = paused);
        Ok(())
    }
    fn clipboard_send_now(&self) -> BackendResult<()> {
        Ok(())
    }
    fn set_clipboard_sync(&self, enabled: bool) -> BackendResult<()> {
        self.with(|s| s.status.clipboard_sync = enabled);
        Ok(())
    }
    fn receive_remote_files(&self) -> BackendResult<()> {
        Ok(())
    }
    fn wake_pc(&self) -> BackendResult<()> {
        Ok(())
    }
    fn reconnect(&self) -> BackendResult<()> {
        Ok(())
    }
    fn list_audio_devices(&self) -> AudioDevices {
        AudioDevices {
            outputs: vec![
                device("mac-speakers", "Alto-falantes do MacBook", true),
                device("mac-headphones", "Fones", false),
            ],
            inputs: vec![device("mac-mic", "Microfone do MacBook", true)],
        }
    }
    fn set_audio_streams(&self, pc_to_mac: bool, mac_to_pc: bool) -> BackendResult<()> {
        self.with(|s| {
            s.status.audio = AudioState {
                pc_to_mac,
                mac_to_pc,
            }
        });
        Ok(())
    }
    fn set_audio_devices(
        &self,
        output_id: Option<String>,
        input_id: Option<String>,
    ) -> BackendResult<()> {
        self.with(|s| {
            s.config.audio.mac_output_id = output_id;
            s.config.audio.mac_input_id = input_id;
        });
        Ok(())
    }
    fn set_stream_volume(&self, volume: f32) -> BackendResult<()> {
        self.with(|s| s.config.audio.stream_volume = volume);
        Ok(())
    }
    fn check_permissions(&self) -> PermissionsStatus {
        PermissionsStatus {
            accessibility: PermissionState::Granted,
            input_monitoring: PermissionState::Granted,
        }
    }
    fn open_permission_settings(&self, _kind: PermissionKind) -> BackendResult<()> {
        Ok(())
    }
    fn set_launch_at_login(&self, enabled: bool) -> BackendResult<()> {
        self.with(|s| s.config.general.launch_at_login = enabled);
        Ok(())
    }
    fn latency(&self) -> LatencyStats {
        LatencyStats {
            rtt_ms: 0.8,
            jitter_ms: 0.1,
            loss_pct: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_has_three_displays_and_connected_status() {
        let b = MockBackend::default();
        let d = b.get_displays();
        let all: Vec<_> = d.mac.iter().chain(d.windows.iter()).collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].size_mm, Some((302.0, 196.0)));
        assert_eq!(all[1].size_mm, Some((597.0, 336.0)));
        assert_eq!(all[2].size_mm, Some((597.0, 336.0)));
        assert_eq!(b.get_status().status, ConnectionStatus::Connected);
        assert_eq!(b.latency().rtt_ms, 0.8);
        assert_eq!(b.get_layout().displays.len(), 3);
    }

    #[test]
    fn mock_toggles_are_reflected_in_status() {
        let b = MockBackend::default();
        b.set_cursor_lock(true).unwrap();
        b.set_sharing_paused(true).unwrap();
        let s = b.get_status();
        assert!(s.locked && s.paused);
    }
}
