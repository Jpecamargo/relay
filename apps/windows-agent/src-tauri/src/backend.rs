//! dono: WA1
//! Contrato entre o casco Tauri e o núcleo do agente. O casco só conhece este trait; o `AgentCore`
//! real (WI1) o implementa e o `MockBackend` (só `#[cfg(test)]`) serve aos testes.

use relay_core::types::{Config, NewPairingCode, ServiceStatus, StatusSnapshot};

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
    fn new_pairing_code(&self) -> AgentResult<NewPairingCode>;
    fn revoke_pair(&self) -> AgentResult<()>;
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
        Config, ConnectionStatus, Machine, NewPairingCode, ServiceStatus, StatusSnapshot,
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
                        focus: Machine::Windows,
                        locked: false,
                        paused: false,
                        secure_input: false,
                        pc_unreachable: false,
                        peer_version: None,
                        version_sync: Default::default(),
                        clipboard_sync: true,
                        audio: Default::default(),
                    },
                    config: Config::default(),
                    paired: false,
                }),
            }
        }
    }

    impl MockBackend {
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

        fn revoke_pair(&self) -> AgentResult<()> {
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
        b.revoke_pair().unwrap();
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
