//! dono: MI1
//! `Platform` do macOS: junta os backends de `relay-platform-macos` e a descoberta mDNS.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use relay_core::crypto::{FileKeyStore, SystemKeyStore};
use relay_core::discovery::{Browser, local_hostname};
use relay_core::platform::{KeyStore, PlatformError};
use relay_core::types::{DiscoveredPeer, DockInfo, HotCorners, PermissionKind, PermissionsStatus};
use relay_platform_macos::capture::MacCapture;
use relay_platform_macos::cursor::MacCursor;
use relay_platform_macos::displays::MacDisplays;
use relay_platform_macos::inject::MacInjector;
use relay_platform_macos::login_item::LoginItem;
use relay_platform_macos::secure_input::SecureInputWatcher;
use relay_platform_macos::system_events::{SystemEvent, SystemEvents};
use relay_platform_macos::{cursor_bg, dock, permissions};

use super::platform::{Browse, Platform, SysEvent};

#[derive(Debug, Default)]
pub struct MacPlatform;

impl Browse for Browser {
    fn next_peer(&self, timeout: Duration) -> Option<DiscoveredPeer> {
        Browser::next_peer(self, timeout)
    }
}

impl Platform for MacPlatform {
    type Capture = MacCapture;
    type Cursor = MacCursor;
    type Injector = MacInjector;
    type Displays = MacDisplays;

    fn new_capture(&self) -> MacCapture {
        MacCapture::new()
    }

    fn new_cursor(&self) -> MacCursor {
        // Sem isto o cursor não some enquanto o Relay está em background (ACHADO M2).
        if !cursor_bg::enable() {
            tracing::warn!("SetsCursorInBackground indisponível: o cursor pode ficar visível");
        }
        MacCursor::default()
    }

    fn new_injector(&self) -> MacInjector {
        MacInjector::default()
    }

    fn new_displays(&self) -> MacDisplays {
        MacDisplays::default()
    }

    /// Uma instância só: o `FileKeyStore` guarda o arquivo em memória.
    fn key_store(&self) -> Arc<dyn KeyStore> {
        static STORE: std::sync::OnceLock<Arc<dyn KeyStore>> = std::sync::OnceLock::new();
        STORE
            .get_or_init(|| {
                let path = self.config_path().with_file_name("secrets.bin");
                match FileKeyStore::open(path, Some(Box::new(SystemKeyStore))) {
                    Ok(store) => Arc::new(store),
                    Err(e) => {
                        tracing::error!("segredos ilegíveis, usando o Keychain: {e}");
                        Arc::new(SystemKeyStore)
                    }
                }
            })
            .clone()
    }

    fn config_path(&self) -> PathBuf {
        relay_core::config::config_path()
    }

    fn device_name(&self) -> String {
        local_hostname()
    }

    fn check_permissions(&self) -> PermissionsStatus {
        permissions::check_permissions()
    }

    fn open_permission_settings(&self, kind: PermissionKind) {
        permissions::open_permission_settings(kind);
    }

    fn dock(&self) -> (DockInfo, HotCorners) {
        dock::read_dock_info()
    }

    fn set_launch_at_login(&self, enabled: bool) -> Result<(), String> {
        let item = LoginItem::default();
        let r: Result<(), PlatformError> = if enabled {
            item.register()
        } else {
            item.unregister()
        };
        r.map_err(|e| e.to_string())
    }

    fn watch_system(&self, callback: Box<dyn Fn(SysEvent) + Send + Sync>) -> Box<dyn Any + Send> {
        let mut events = SystemEvents::default();
        events.start(Box::new(move |e| {
            callback(match e {
                SystemEvent::WillSleep => SysEvent::WillSleep,
                SystemEvent::DidWake => SysEvent::DidWake,
                SystemEvent::NetworkChanged => SysEvent::NetworkChanged,
            });
        }));
        Box::new(events)
    }

    fn watch_secure_input(&self, callback: Box<dyn Fn(bool) + Send + Sync>) -> Box<dyn Any + Send> {
        let mut watcher = SecureInputWatcher::default();
        watcher.start(callback);
        Box::new(watcher)
    }

    fn browse(&self) -> Result<Box<dyn Browse>, String> {
        Browser::start()
            .map(|b| Box::new(b) as Box<dyn Browse>)
            .map_err(|e| e.to_string())
    }
}
