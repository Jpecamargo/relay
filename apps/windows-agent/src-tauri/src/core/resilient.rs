//! dono: WI1
//! Backend que sobrevive à falha do `AgentCore::start` (porta ocupada, Credential Manager): o app
//! sobe com a bandeja em estado de erro, os comandos devolvem a mensagem e o início é refeito
//! sozinho a cada `RETRY_EVERY` e a cada pedido de pareamento.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use relay_core::types::{
    Caps, Config, ConnectionStatus, DiscoveredPeer, DisplaysPayload, Host, LayoutBook,
    LayoutProfile, NewPairingCode, Os, SaveLayoutResult, ServiceStatus, StatusSnapshot,
};

use super::AgentCore;
use crate::backend::{AgentBackend, AgentError, AgentResult};

pub const RETRY_EVERY: Duration = Duration::from_secs(5);

type Factory = Box<dyn Fn() -> AgentResult<AgentCore> + Send + Sync>;

pub struct ResilientBackend {
    core: Mutex<Option<Arc<AgentCore>>>,
    error: Mutex<Option<String>>,
    config: Mutex<Config>,
    factory: Factory,
    /// Chamado quando o estado muda (núcleo subiu): atualiza a bandeja.
    on_change: Box<dyn Fn() + Send + Sync>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ResilientBackend {
    /// Tenta subir agora; em falha guarda o erro e, se `auto_retry`, refaz a cada `RETRY_EVERY`.
    pub fn new(
        factory: Factory,
        config: Config,
        on_change: Box<dyn Fn() + Send + Sync>,
        auto_retry: bool,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            core: Mutex::new(None),
            error: Mutex::new(None),
            config: Mutex::new(config),
            factory,
            on_change,
        });
        this.retry();
        if auto_retry {
            let weak: Weak<Self> = Arc::downgrade(&this);
            let _ = std::thread::Builder::new()
                .name("relay-agent-retry".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(RETRY_EVERY);
                        let Some(this) = weak.upgrade() else { return };
                        if lock(&this.core).is_some() {
                            return;
                        }
                        this.retry();
                    }
                });
        }
        this
    }

    /// Tenta subir o núcleo se ainda não subiu. `true` se está rodando.
    pub fn retry(&self) -> bool {
        if lock(&self.core).is_some() {
            return true;
        }
        match (self.factory)() {
            Ok(core) => {
                *lock(&self.core) = Some(Arc::new(core));
                *lock(&self.error) = None;
                (self.on_change)();
                true
            }
            Err(e) => {
                let msg = e.to_string();
                tracing::error!(error = %msg, "o agente não iniciou");
                if lock(&self.error).replace(msg).is_none() {
                    (self.on_change)();
                }
                false
            }
        }
    }

    fn core(&self) -> AgentResult<Arc<AgentCore>> {
        lock(&self.core).clone().ok_or_else(|| {
            AgentError::Backend(format!(
                "agente indisponível: {}",
                lock(&self.error).clone().unwrap_or_default()
            ))
        })
    }
}

impl AgentBackend for ResilientBackend {
    fn status(&self) -> StatusSnapshot {
        match lock(&self.core).as_ref() {
            Some(c) => c.status(),
            None => StatusSnapshot {
                status: ConnectionStatus::Disconnected,
                peer_name: None,
                focus: Host::Peer,
                locked: false,
                paused: false,
                secure_input: false,
                peer_unreachable: false,
                peer_version: None,
                version_sync: Default::default(),
                clipboard_sync: lock(&self.config).clipboard.enabled,
                audio: Default::default(),
                local_os: Os::Windows,
                peer_os: None,
                local_caps: Caps::baseline(Os::Windows),
                peer_caps: None,
                layout_host: Host::Peer,
                layout_editable: false,
            },
        }
    }

    fn set_paused(&self, paused: bool) -> AgentResult<()> {
        self.core()?.set_paused(paused)
    }

    fn set_locked(&self, locked: bool) -> AgentResult<()> {
        self.core()?.set_locked(locked)
    }

    fn jump_to(&self, n: u8) -> AgentResult<()> {
        self.core()?.jump_to(n)
    }

    fn clipboard_send_now(&self) -> AgentResult<()> {
        self.core()?.clipboard_send_now()
    }

    fn reconnect(&self) -> AgentResult<()> {
        self.retry();
        self.core()?.reconnect()
    }

    fn new_pairing_code(&self) -> AgentResult<NewPairingCode> {
        self.retry();
        self.core()?.new_pairing_code()
    }

    fn pair_with_code(
        &self,
        code: String,
        address: Option<String>,
        layout_host: Host,
    ) -> AgentResult<()> {
        self.retry();
        self.core()?.pair_with_code(code, address, layout_host)
    }

    fn unpair(&self) -> AgentResult<()> {
        self.core()?.unpair()
    }

    fn list_discovered(&self) -> Vec<DiscoveredPeer> {
        lock(&self.core)
            .as_ref()
            .map(|c| c.list_discovered())
            .unwrap_or_default()
    }

    fn service_status(&self) -> ServiceStatus {
        lock(&self.core)
            .as_ref()
            .map(|c| c.service_status())
            .unwrap_or_default()
    }

    fn get_displays(&self) -> DisplaysPayload {
        lock(&self.core)
            .as_ref()
            .map(|c| c.get_displays())
            .unwrap_or_default()
    }

    fn get_layout(&self) -> LayoutProfile {
        match lock(&self.core).as_ref() {
            Some(c) => c.get_layout(),
            None => super::layout_state::active_profile(&self.get_layout_book())
                .unwrap_or_else(super::layout_state::empty_profile),
        }
    }

    fn get_layout_book(&self) -> LayoutBook {
        match lock(&self.core).as_ref() {
            Some(c) => c.get_layout_book(),
            None => super::layout_state::book_of(&lock(&self.config).layout),
        }
    }

    fn save_layout(&self, profile: LayoutProfile) -> AgentResult<SaveLayoutResult> {
        self.core()?.save_layout(profile)
    }

    fn set_layout_profiles(&self, profiles: Vec<LayoutProfile>, active: String) -> AgentResult<()> {
        self.core()?.set_layout_profiles(profiles, active)
    }

    fn set_layout_host(&self, host: Host) -> AgentResult<()> {
        self.core()?.set_layout_host(host)
    }

    fn config(&self) -> Config {
        match lock(&self.core).as_ref() {
            Some(c) => c.config(),
            None => lock(&self.config).clone(),
        }
    }

    fn save_config(&self, config: Config) -> AgentResult<()> {
        match lock(&self.core).as_ref() {
            Some(c) => c.save_config(config),
            None => {
                relay_core::config::save(&relay_core::config::config_path(), &config)
                    .map_err(|e| AgentError::Backend(e.to_string()))?;
                *lock(&self.config) = config;
                Ok(())
            }
        }
    }

    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}
