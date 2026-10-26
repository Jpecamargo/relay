//! dono: WI1
//! Backend que sobrevive à falha do `AgentCore::start` (porta ocupada, Credential Manager): o app
//! sobe com a bandeja em estado de erro, os comandos devolvem a mensagem e o início é refeito
//! sozinho a cada `RETRY_EVERY` e a cada pedido de pareamento.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use relay_core::types::{
    Caps, Config, ConnectionStatus, Host, NewPairingCode, Os, ServiceStatus, StatusSnapshot,
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

    fn new_pairing_code(&self) -> AgentResult<NewPairingCode> {
        self.retry();
        self.core()?.new_pairing_code()
    }

    fn revoke_pair(&self) -> AgentResult<()> {
        self.core()?.revoke_pair()
    }

    fn service_status(&self) -> ServiceStatus {
        lock(&self.core)
            .as_ref()
            .map(|c| c.service_status())
            .unwrap_or_default()
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
