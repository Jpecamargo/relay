//! dono: WI1
//! `AgentCore`: o núcleo do agente Windows (D10, D11, D12, D27). Genérico sobre as traits de
//! plataforma (tudo entra por `AgentDeps`), então roda no Mac com mocks.
//!
//! Uma tarefa tokio (o "pump") é dona da `SessionListener`, do `Receiver` e do guard de precisão
//! do ponteiro, e serializa tudo o que chega: eventos da sessão, canal rápido (UDP), comandos da
//! UI, mudança de telas, app em primeiro plano e o relógio do `Receiver` (≤ 10 ms enquanto o Mac
//! controla este lado). A UI (`AgentBackend`) só enfileira comandos e lê o snapshot.

mod pump;
pub mod real;
mod reverse;
mod routed;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use relay_core::crypto::{
    LocalDevice, device_id_of, generate_pairing_code, load_or_create_identity, load_peer,
};
use relay_core::discovery::Advertiser;
use relay_core::engine::EventHandler;
use relay_core::net::{ListenerConfig, PAIRING_CODE_TTL, SessionListener};
use relay_core::platform::{
    CommandExecutor, CursorControl, DisplaySource, InputInjector, KeyStore,
};
use relay_core::types::{
    AudioState, Config, ConnectionStatus, LatencyStats, Machine, Message, NewPairingCode,
    PairingEvent, ServiceStatus, StatusSnapshot,
};
use tokio::sync::mpsc;

use crate::backend::{AgentBackend, AgentError, AgentResult};
pub use reverse::{ReverseDeps, ReverseHandle};
pub use routed::{BoxCursor, PipeInjector, RoutedInjector, SharedPipe};

/// Conta do KeyStore com o `device_id` do Mac pareado (o trait não lista contas).
pub(crate) const ACTIVE_PEER_ACCOUNT: &str = "active-peer";

/// Eventos core→UI (nomes do glossário: `relay:status`, `relay:latency`, `relay:pairing`).
pub trait UiEvents: Send + Sync {
    fn status(&self, snapshot: &StatusSnapshot);
    /// No máximo 2 Hz.
    fn latency(&self, stats: &LatencyStats);
    fn pairing(&self, event: &PairingEvent);
}

/// Observador do app em primeiro plano (`ForegroundWatcher` no Windows).
pub trait ForegroundSource: Send {
    /// Chama `on_change` com o exe sempre que o app muda (e uma vez com o atual).
    fn start(&mut self, on_change: Box<dyn Fn(String) + Send + Sync>);
}

/// Gancho para mensagens que o núcleo não trata (clipboard, arquivos, áudio: WI3). Devolve as
/// mensagens a enviar de volta.
pub type MessageHook = Box<dyn FnMut(&Message) -> Vec<Message> + Send>;

/// Tudo o que o núcleo precisa da plataforma.
pub struct AgentDeps {
    pub keystore: Arc<dyn KeyStore>,
    /// Injeção local, usada quando o pipe do serviço não está disponível.
    pub injector: Box<dyn InputInjector>,
    pub pipe: Option<SharedPipe>,
    pub cursor: Box<dyn CursorControl>,
    pub displays: Box<dyn DisplaySource>,
    pub foreground: Option<Box<dyn ForegroundSource>>,
    /// Executor de comandos (WI2). Sem ele o `Receiver` responde `unsupported`.
    pub executor: Option<Box<dyn CommandExecutor>>,
    /// Liga a precisão do ponteiro desligada; o valor devolvido restaura ao ser descartado (D27).
    pub pointer_guard: Box<dyn Fn() -> Box<dyn Send> + Send + Sync>,
    /// `helper_active` → estado do serviço.
    pub service_probe: Box<dyn Fn(bool) -> ServiceStatus + Send + Sync>,
    pub device_name: String,
    pub mac_address: [u8; 6],
    /// Onde persistir o Config (`None` = só em memória).
    pub config_path: Option<PathBuf>,
    pub bind_ip: IpAddr,
    /// Anuncia `_relay._tcp` por mDNS.
    pub advertise: bool,
    pub message_hook: Option<MessageHook>,
    pub engine_events: Option<EventHandler>,
}

pub(crate) enum Cmd {
    OpenPairing { code: String, expires_at_ms: u64 },
    Revoke,
    SetPaused(bool),
    DisplaysChanged,
    Foreground(String),
    Fast(relay_core::net::FastEvent),
    ConfigChanged,
    Shutdown,
}

/// Estado compartilhado entre a UI (threads do Tauri) e o pump.
pub(crate) struct Shared {
    pub snapshot: Mutex<StatusSnapshot>,
    pub config: Mutex<Config>,
    pub tx: mpsc::UnboundedSender<Cmd>,
    pub pipe: Option<SharedPipe>,
    pub service_probe: Box<dyn Fn(bool) -> ServiceStatus + Send + Sync>,
    pub config_path: Option<PathBuf>,
}

impl Shared {
    pub fn snapshot(&self) -> MutexGuard<'_, StatusSnapshot> {
        self.snapshot.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn save_config(&self, config: &Config) {
        if let Some(path) = &self.config_path {
            if let Err(e) = relay_core::config::save(path, config) {
                tracing::warn!(error = %e, "falha ao gravar o config");
            }
        }
    }
}

pub struct AgentCore {
    shared: Arc<Shared>,
    control_addr: SocketAddr,
    input_addr: SocketAddr,
    device_id: String,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn initial_snapshot(config: &Config) -> StatusSnapshot {
    StatusSnapshot {
        status: ConnectionStatus::Disconnected,
        peer_name: None,
        focus: Machine::Windows,
        locked: false,
        paused: false,
        secure_input: false,
        clipboard_sync: config.clipboard.enabled,
        audio: AudioState::default(),
    }
}

fn start_err(what: &str, e: impl std::fmt::Display) -> AgentError {
    AgentError::Backend(format!("{what}: {e}"))
}

impl AgentCore {
    /// Sobe o listener (portas do Config), o anúncio mDNS e o pump numa thread própria com um
    /// runtime tokio de thread única (o `Receiver` e a sessão não são `Sync`). Devolve depois que
    /// as portas estão abertas, ou com o erro de quem falhou.
    pub fn start(deps: AgentDeps, config: Config, events: Arc<dyn UiEvents>) -> AgentResult<Self> {
        Self::start_with_reverse(deps, config, events, None)
    }

    /// Como `start`, com o sentido inverso (R1): com `reverse`, o mouse e o teclado do PC podem
    /// controlar o Mac pela borda ligada a ele, desde que o Mac não esteja controlando o PC (D24).
    pub fn start_with_reverse(
        deps: AgentDeps,
        config: Config,
        events: Arc<dyn UiEvents>,
        reverse: Option<ReverseDeps>,
    ) -> AgentResult<Self> {
        let (tx, cmds) = mpsc::unbounded_channel();
        let mut deps = deps;
        // O probe do serviço (consulta o SCM) fica do lado da UI, não no pump.
        let probe = std::mem::replace(
            &mut deps.service_probe,
            Box::new(|_| ServiceStatus::default()),
        );
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(initial_snapshot(&config)),
            pipe: deps.pipe.clone(),
            config_path: deps.config_path.clone(),
            config: Mutex::new(config),
            tx,
            service_probe: probe,
        });

        let (init_tx, init_rx) = std::sync::mpsc::channel();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("relay-agent".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = init_tx.send(Err(start_err("runtime", e)));
                        return;
                    }
                };
                rt.block_on(async move {
                    match setup(deps, thread_shared, events, reverse).await {
                        Ok((pump, addrs, device_id)) => {
                            let _ = init_tx.send(Ok((addrs, device_id)));
                            pump.run(cmds).await;
                        }
                        Err(e) => {
                            let _ = init_tx.send(Err(e));
                        }
                    }
                });
            })
            .map_err(|e| start_err("thread", e))?;
        let (addrs, device_id) = init_rx
            .recv()
            .map_err(|_| AgentError::Backend("o agente não iniciou".into()))??;
        Ok(Self {
            shared,
            control_addr: addrs.0,
            input_addr: addrs.1,
            device_id,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Endereços reais de escuta (TCP de controle, UDP de input): úteis com portas efêmeras.
    pub fn local_addrs(&self) -> (SocketAddr, SocketAddr) {
        (self.control_addr, self.input_addr)
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Encerra o pump (solta teclas, restaura o ponteiro, fecha as portas) e espera a thread.
    pub fn shutdown(&self) {
        let _ = self.shared.tx.send(Cmd::Shutdown);
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = thread {
            let _ = t.join();
        }
    }

    fn send(&self, cmd: Cmd) -> AgentResult<()> {
        self.shared
            .tx
            .send(cmd)
            .map_err(|_| AgentError::Backend("agente encerrado".into()))
    }
}

impl Drop for AgentCore {
    fn drop(&mut self) {
        let _ = self.shared.tx.send(Cmd::Shutdown);
    }
}

impl AgentBackend for AgentCore {
    fn status(&self) -> StatusSnapshot {
        self.shared.snapshot().clone()
    }

    fn set_paused(&self, paused: bool) -> AgentResult<()> {
        self.send(Cmd::SetPaused(paused))
    }

    /// Código de 6 dígitos válido por 120 s e de uso único (consumido pela primeira tentativa).
    fn new_pairing_code(&self) -> AgentResult<NewPairingCode> {
        let code = generate_pairing_code();
        let expires_at = now_ms() + PAIRING_CODE_TTL.as_millis() as u64;
        self.send(Cmd::OpenPairing {
            code: code.clone(),
            expires_at_ms: expires_at,
        })?;
        Ok(NewPairingCode { code, expires_at })
    }

    fn revoke_pair(&self) -> AgentResult<()> {
        self.send(Cmd::Revoke)
    }

    fn service_status(&self) -> ServiceStatus {
        let helper_active = self
            .shared
            .pipe
            .as_ref()
            .is_some_and(|p| p.lock().unwrap_or_else(|e| e.into_inner()).is_available());
        (self.shared.service_probe)(helper_active)
    }

    fn config(&self) -> Config {
        self.shared.config().clone()
    }

    fn save_config(&self, config: Config) -> AgentResult<()> {
        self.shared.save_config(&config);
        *self.shared.config() = config;
        self.send(Cmd::ConfigChanged)
    }
}

type Setup = (pump::Pump, (SocketAddr, SocketAddr), String);

/// Parte assíncrona do start, dentro do runtime do agente.
async fn setup(
    mut deps: AgentDeps,
    shared: Arc<Shared>,
    events: Arc<dyn UiEvents>,
    reverse: Option<ReverseDeps>,
) -> AgentResult<Setup> {
    let identity =
        load_or_create_identity(&*deps.keystore).map_err(|e| start_err("identidade", e))?;
    let device_id = device_id_of(&identity.public);
    let peers: Vec<_> = match deps.keystore.get(ACTIVE_PEER_ACCOUNT) {
        Ok(Some(id)) => {
            let id = String::from_utf8_lossy(&id).into_owned();
            load_peer(&*deps.keystore, &id)
                .ok()
                .flatten()
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    };
    let device = LocalDevice {
        name: deps.device_name.clone(),
        public_key: identity.public,
        mac_address: deps.mac_address,
    };
    let ports = shared.config().network.ports.clone();
    let mut lcfg = ListenerConfig::new(identity, device, peers.clone(), ports);
    lcfg.bind_ip = deps.bind_ip;

    let fast_tx = shared.tx.clone();
    let listener = SessionListener::start(
        lcfg,
        Box::new(move |ev| {
            let _ = fast_tx.send(Cmd::Fast(ev));
        }),
    )
    .await
    .map_err(|e| start_err("listener", e))?;
    let addrs = (listener.local_control_addr(), listener.local_input_addr());

    let advertiser = if deps.advertise {
        Advertiser::start(&device_id)
            .map_err(|e| tracing::warn!(error = %e, "mDNS indisponível; só pelo endereço manual"))
            .ok()
    } else {
        None
    };

    // Telas e app em primeiro plano chegam ao pump como comandos.
    let disp_tx = shared.tx.clone();
    deps.displays.set_change_callback(Box::new(move || {
        let _ = disp_tx.send(Cmd::DisplaysChanged);
    }));
    if let Some(fg) = deps.foreground.as_mut() {
        let fg_tx = shared.tx.clone();
        fg.start(Box::new(move |exe| {
            let _ = fg_tx.send(Cmd::Foreground(exe));
        }));
    }

    let pump = pump::Pump::new(deps, shared, listener, advertiser, events, peers, reverse);
    Ok((pump, addrs, device_id))
}
