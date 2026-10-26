//! dono: WI1
//! Peças reais do Windows ligadas ao `AgentCore`. Compila em qualquer target (os backends de
//! `relay-platform-windows` são neutros fora do Windows), mas só faz efeito no Windows.

use std::sync::{Arc, Mutex};

use relay_core::crypto::SystemKeyStore;
use relay_core::platform::KeyStore;
use relay_core::types::ServiceStatus;
use relay_platform_windows::clipboard::WindowsClipboard;
use relay_platform_windows::displays::WindowsDisplays;
use relay_platform_windows::foreground::ForegroundWatcher;
use relay_platform_windows::inject::WindowsInjector;
use relay_platform_windows::mouse_accel::PointerPrecisionGuard;
use relay_platform_windows::pipe::InjectPipeClient;
use relay_platform_windows::service::SERVICE_NAME;

use super::media::platform::{
    WindowsAudioHost, detect_vbcable, recover_local_mute, windows_drop_target,
};
use super::media::{CaptureSource, Media, MediaDeps};
use super::routed::{PipeInjector, SharedPipe};
use super::{AgentDeps, ForegroundSource, PeerBrowse};

impl PipeInjector for InjectPipeClient {
    fn is_available(&self) -> bool {
        InjectPipeClient::is_available(self)
    }
}

impl ForegroundSource for ForegroundWatcher {
    fn start(&mut self, on_change: Box<dyn Fn(String) + Send + Sync>) {
        ForegroundWatcher::start(self, on_change);
    }
}

/// A busca mDNS do core como `PeerBrowse`.
struct RealBrowse(relay_core::discovery::Browser);

impl PeerBrowse for RealBrowse {
    fn next_peer(
        &mut self,
        timeout: std::time::Duration,
    ) -> Option<relay_core::types::DiscoveredPeer> {
        self.0.next_peer(timeout)
    }
}

/// Estado do `RelayService` no SCM (`sc query`) + se o helper respondeu no pipe.
pub fn query_service(helper_active: bool) -> ServiceStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("sc")
            .args(["query", SERVICE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            return ServiceStatus {
                installed: o.status.success(),
                running: o.status.success() && text.contains("RUNNING"),
                helper_active,
                vbcable_detected: detect_vbcable(),
            };
        }
    }
    let _ = SERVICE_NAME;
    ServiceStatus {
        installed: helper_active,
        running: helper_active,
        helper_active,
        vbcable_detected: detect_vbcable(),
    }
}

/// Fila de PNGs do `CaptureScreen` (WI2) que a mídia oferece ao Mac como clipboard.
fn real_captures() -> CaptureSource {
    Box::new(super::executor::take_captures)
}

/// Mídia de produção: clipboard do Windows, Core Audio/VB-CABLE e Explorer.
pub fn real_media(keystore: Arc<dyn KeyStore>) -> Media {
    recover_local_mute();
    let config =
        relay_core::config::load_or_default(&relay_core::config::config_path()).unwrap_or_default();
    Media::start(
        MediaDeps {
            keystore,
            clipboard: Box::new(WindowsClipboard::default()),
            audio: Arc::new(WindowsAudioHost),
            drop_target: windows_drop_target(),
            temp_root: std::env::temp_dir(),
            captures: real_captures(),
        },
        &config,
    )
}

/// Dependências de produção: executor de comandos (WI2) e mídia (WI3) já ligados.
pub fn real_deps() -> AgentDeps {
    let pipe: SharedPipe = Arc::new(Mutex::new(Box::new(InjectPipeClient::default())));
    let keystore: Arc<dyn KeyStore> = Arc::new(SystemKeyStore);
    let media = real_media(keystore.clone());
    AgentDeps {
        message_hook: Some(media.hook()),
        engine_events: Some(media.engine_events()),
        media: Some(Box::new(media)),
        keystore,
        injector: Box::new(WindowsInjector::default()),
        pipe: Some(pipe),
        cursor: Box::new(relay_platform_windows::cursor::WindowsCursor::default()),
        displays: Box::new(WindowsDisplays::default()),
        foreground: Some(Box::new(ForegroundWatcher::default())),
        executor: Some(super::executor::windows_executor()),
        pointer_guard: Box::new(|| Box::new(PointerPrecisionGuard::engage())),
        service_probe: Box::new(query_service),
        device_name: relay_core::discovery::local_hostname(),
        mac_address: relay_platform_windows::net::primary_mac_address().unwrap_or([0; 6]),
        config_path: Some(relay_core::config::config_path()),
        bind_ip: [0, 0, 0, 0].into(),
        advertise: true,
        app_version: None,
        browse: Some(Box::new(|| {
            relay_core::discovery::Browser::start()
                .map(|b| Box::new(RealBrowse(b)) as Box<dyn PeerBrowse>)
                .map_err(|e| e.to_string())
        })),
    }
}
