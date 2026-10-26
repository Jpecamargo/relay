//! dono: WA1
//! Contrato entre o casco Tauri e o núcleo do agente. O casco só conhece este trait; o `AgentCore`
//! real (WI1) o implementa e o `MockBackend` (só `#[cfg(test)]`) serve aos testes.

use relay_core::types::{
    Config, DiscoveredPeer, Host, NewPairingCode, ServiceStatus, StatusSnapshot,
};

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("{0}")]
    Backend(String),
}

pub type AgentResult<T> = Result<T, AgentError>;

pub trait AgentBackend: Send + Sync {
    fn status(&self) -> StatusSnapshot;
    /// Pausa ou retoma o compartilhamento (menu da bandeja).
    fn set_paused(&self, paused: bool) -> AgentResult<()>;
    /// Trava o cursor no lado atual (controller reverso).
    fn set_locked(&self, locked: bool) -> AgentResult<()>;
    /// Pula o cursor para a tela de número `n` (controller reverso).
    fn jump_to(&self, n: u8) -> AgentResult<()>;
    fn clipboard_send_now(&self) -> AgentResult<()>;
    /// Derruba e refaz a sessão com o par.
    fn reconnect(&self) -> AgentResult<()>;
    /// Esta máquina mostra o código e escuta como Listener (120 s).
    fn new_pairing_code(&self) -> AgentResult<NewPairingCode>;
    /// Esta máquina disca como Dialer com o código que o outro lado mostra. `address` é `host`,
    /// `host:porta` ou `None` (o único par descoberto); `layout_host` é a escolha do D44.
    fn pair_with_code(
        &self,
        code: String,
        address: Option<String>,
        layout_host: Host,
    ) -> AgentResult<()>;
    /// Esquece o par (avisa o outro lado com `Unpair`).
    fn unpair(&self) -> AgentResult<()>;
    /// Pares vistos pelo mDNS agora.
    fn list_discovered(&self) -> Vec<DiscoveredPeer>;
    fn service_status(&self) -> ServiceStatus;
    fn config(&self) -> Config;
    fn save_config(&self, config: Config) -> AgentResult<()>;
    /// Mensagem de erro quando o núcleo não subiu (bandeja em estado de erro); `None` = ok.
    fn error(&self) -> Option<String> {
        None
    }
}

/// Backend em memória, só para os testes do casco.
#[cfg(test)]
pub use mock::MockBackend;

#[cfg(test)]
mod mock {
    use std::sync::Mutex;

    use super::{AgentBackend, AgentResult};
    use relay_core::types::{
        Caps, Config, ConnectionStatus, DiscoveredPeer, Host, NewPairingCode, Os, ServiceStatus,
        StatusSnapshot,
    };

    /// Validade do código de pareamento (D5).
    const PAIRING_CODE_TTL_MS: u64 = 120_000;

    pub struct MockBackend {
        inner: Mutex<MockInner>,
    }

    struct MockInner {
        status: StatusSnapshot,
        config: Config,
        paired: bool,
    }

    impl Default for MockBackend {
        fn default() -> Self {
            Self {
                inner: Mutex::new(MockInner {
                    status: StatusSnapshot {
                        status: ConnectionStatus::Disconnected,
                        peer_name: None,
                        focus: Host::Local,
                        locked: false,
                        paused: false,
                        secure_input: false,
                        peer_unreachable: false,
                        peer_version: None,
                        version_sync: Default::default(),
                        clipboard_sync: true,
                        audio: Default::default(),
                        local_os: Os::Windows,
                        peer_os: None,
                        local_caps: Caps::baseline(Os::Windows),
                        peer_caps: None,
                        layout_host: Host::Peer,
                        layout_editable: false,
                    },
                    config: Config::default(),
                    paired: false,
                }),
            }
        }
    }

    impl MockBackend {
        /// Ajusta o snapshot (caps do par, SO) nos testes dos comandos.
        pub fn edit_status(&self, f: impl FnOnce(&mut StatusSnapshot)) {
            f(&mut self.lock().status);
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, MockInner> {
            self.inner.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl AgentBackend for MockBackend {
        fn status(&self) -> StatusSnapshot {
            self.lock().status.clone()
        }

        fn set_paused(&self, paused: bool) -> AgentResult<()> {
            self.lock().status.paused = paused;
            Ok(())
        }

        fn set_locked(&self, locked: bool) -> AgentResult<()> {
            self.lock().status.locked = locked;
            Ok(())
        }

        fn jump_to(&self, _n: u8) -> AgentResult<()> {
            Ok(())
        }

        fn clipboard_send_now(&self) -> AgentResult<()> {
            Ok(())
        }

        fn reconnect(&self) -> AgentResult<()> {
            Ok(())
        }

        fn new_pairing_code(&self) -> AgentResult<NewPairingCode> {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            let mut g = self.lock();
            g.status.status = ConnectionStatus::Pairing;
            g.paired = true;
            Ok(NewPairingCode {
                code: "123456".into(),
                expires_at: now + PAIRING_CODE_TTL_MS,
            })
        }

        fn pair_with_code(
            &self,
            _code: String,
            _address: Option<String>,
            _layout_host: Host,
        ) -> AgentResult<()> {
            self.lock().paired = true;
            Ok(())
        }

        fn list_discovered(&self) -> Vec<DiscoveredPeer> {
            Vec::new()
        }

        fn unpair(&self) -> AgentResult<()> {
            let mut g = self.lock();
            g.paired = false;
            g.status.status = ConnectionStatus::Disconnected;
            g.status.peer_name = None;
            Ok(())
        }

        fn service_status(&self) -> ServiceStatus {
            ServiceStatus::default()
        }

        fn config(&self) -> Config {
            self.lock().config.clone()
        }

        fn save_config(&self, config: Config) -> AgentResult<()> {
            self.lock().config = config;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_core::types::ConnectionStatus;

    #[test]
    fn pause_e_pareamento_mudam_o_estado() {
        let b = MockBackend::default();
        assert!(!b.status().paused);
        b.set_paused(true).unwrap();
        assert!(b.status().paused);
        let code = b.new_pairing_code().unwrap();
        assert_eq!(code.code.len(), 6);
        assert_eq!(b.status().status, ConnectionStatus::Pairing);
        b.unpair().unwrap();
        assert_eq!(b.status().status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn config_salva_e_relida() {
        let b = MockBackend::default();
        let mut c = b.config();
        c.general.launch_at_login = !c.general.launch_at_login;
        b.save_config(c.clone()).unwrap();
        assert_eq!(b.config(), c);
    }
}
