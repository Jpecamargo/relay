//! dono: MI1
//! AppCore do Mac (D29): captura, Controller, sessão, descoberta, pareamento, telas, layout e
//! eventos de status. Independente do Tauri e genérico sobre `Platform`.
//!
//! Três partes, ligadas por filas:
//! - a thread `relay-core` (`engine`) é dona do `Controller`: captura → `handle`, `on_message`,
//!   `tick` a cada ≤10 ms, `CommandDispatcher`, e esvazia a fila de controle do Controller;
//! - o gerente de rede (`net`, tokio) é dono da `Session`: descoberta, pareamento, discagem,
//!   reconexão e a bomba de mensagens entre a sessão e a thread do core;
//! - a API pública (`AppCore`) só lê o estado compartilhado e manda comandos às duas.

mod backend;
mod engine;
#[cfg(target_os = "macos")]
mod mac;
pub mod media;
mod net;
mod platform;
mod reverse;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use relay_core::config;
use relay_core::platform::{DisplaySource, KeyStore, LoopWaker};
use relay_core::types::{
    AudioState, Caps, Config, ConnectionStatus, DiscoveredPeer, DisplayInfo, DisplaysPayload, Host,
    LatencyStats, LayoutProfile, Os, PairedPeer, PermissionsStatus, Ports, SaveLayoutResult,
    StatusSnapshot,
};
use relay_core::version::{APP_VERSION, version_sync};
use relay_core::{crypto, layout};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use engine::CoreCmd;
use net::NetCmd;

#[cfg(target_os = "macos")]
pub use mac::MacPlatform;
pub use platform::{
    AudioKeys, Browse, CoreEvents, MediaHooks, NoMedia, Platform, SessionCtx, Shared, SysEvent,
};

/// Conta do KeyStore com o `device_id` do PC pareado (um só par, usuário único).
pub(crate) const ACTIVE_PEER_ACCOUNT: &str = "active_peer";

/// Ajustes que os testes mudam; o app usa o padrão.
#[derive(Debug, Clone)]
pub struct CoreOptions {
    /// IP em que a sessão abre o UDP de input.
    pub bind_ip: IpAddr,
    /// Portas locais do Mac; `None` usa `config.network.ports`.
    pub local_ports: Option<Ports>,
    /// Espera pelo mDNS antes de usar o endereço manual.
    pub manual_grace: Duration,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            bind_ip: IpAddr::from([0, 0, 0, 0]),
            local_ports: None,
            manual_grace: Duration::from_secs(3),
        }
    }
}

/// Estado lido pela UI. Mudou o que alimenta `status`, `Inner::mutate` recalcula e emite.
pub(crate) struct State {
    pub status: StatusSnapshot,
    emitted: Option<StatusSnapshot>,
    pub config: Config,
    pub peer: Option<PairedPeer>,
    /// Status da sessão discando (`None`: não há sessão).
    pub session: Option<ConnectionStatus>,
    /// Versão do app do PC, do Hello da sessão atual.
    pub peer_version: Option<String>,
    /// Geração da sessão atual: a bomba de uma sessão velha não escreve no estado.
    pub session_gen: u64,
    /// Quando a sessão atual começou a discar (`None`: não há sessão).
    pub dialing_since: Option<Instant>,
    pub pairing: bool,
    pub local: Vec<DisplayInfo>,
    pub remote: Vec<DisplayInfo>,
    pub profile: Option<LayoutProfile>,
    pub discovered: Vec<DiscoveredPeer>,
    pub latency: LatencyStats,
}

/// Status mostrado ao usuário a partir do que o AppCore sabe.
pub(crate) fn derive_status(
    paired: bool,
    pairing: bool,
    session: Option<ConnectionStatus>,
) -> ConnectionStatus {
    if pairing {
        return ConnectionStatus::Pairing;
    }
    if !paired {
        return ConnectionStatus::Disconnected;
    }
    match session {
        None => ConnectionStatus::Discovering,
        // A sessão nasce `Disconnected` até a primeira tentativa.
        Some(ConnectionStatus::Disconnected) => ConnectionStatus::Connecting,
        Some(s) => s,
    }
}

/// Quanto o primeiro TCP de controle pode ficar sem resposta antes de culparmos o PC/firewall.
pub(crate) const UNREACHABLE_AFTER: Duration = Duration::from_secs(10);

/// Só vale para a primeira conexão (`Connecting`): em `Reconnecting` o PC pode simplesmente ter
/// sido desligado. Sem sessão (`Discovering`: PC fora do mDNS) nunca acusa.
pub(crate) fn pc_unreachable(status: ConnectionStatus, dialing_for: Option<Duration>) -> bool {
    status == ConnectionStatus::Connecting && dialing_for.is_some_and(|d| d >= UNREACHABLE_AFTER)
}

impl State {
    fn derive(&mut self) {
        self.status.status = derive_status(self.peer.is_some(), self.pairing, self.session);
        self.status.peer_unreachable =
            pc_unreachable(self.status.status, self.dialing_since.map(|t| t.elapsed()));
        self.status.peer_name = self.peer.as_ref().map(|p| p.name.clone());
        let connected = self.status.status == ConnectionStatus::Connected;
        self.status.peer_version = self.peer_version.clone().filter(|_| connected);
        self.status.version_sync = version_sync(APP_VERSION, self.status.peer_version.as_deref());
    }
}

pub(crate) struct Inner<P: Platform> {
    pub platform: Arc<P>,
    pub events: Arc<dyn CoreEvents>,
    pub media: Arc<dyn MediaHooks>,
    pub opts: CoreOptions,
    pub config_path: PathBuf,
    pub state: Mutex<State>,
    emit_lock: Mutex<()>,
    pub core_tx: mpsc::Sender<CoreCmd>,
    /// Acorda o laço do core: comandos e eventos da captura.
    pub waker: LoopWaker,
    pub net_tx: UnboundedSender<NetCmd>,
}

impl<P: Platform> Inner<P> {
    pub fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Muda o estado e emite `relay:status` se o snapshot mudou.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let r = {
            let mut st = self.st();
            let r = f(&mut st);
            st.derive();
            r
        };
        self.flush_status();
        r
    }

    /// Emite o snapshot mais novo, em ordem e sem segurar o lock do estado (o sink pode
    /// esperar a thread principal, que por sua vez lê o estado). Quem não consegue o lock de
    /// emissão sai: o dono dele relê o estado antes de soltá-lo.
    fn flush_status(&self) {
        loop {
            if let Ok(_emitting) = self.emit_lock.try_lock() {
                loop {
                    let snap = {
                        let mut st = self.st();
                        if st.emitted.as_ref() == Some(&st.status) {
                            break;
                        }
                        st.emitted = Some(st.status.clone());
                        st.status.clone()
                    };
                    self.events.status(&snap);
                }
            } else {
                return;
            }
            let settled = {
                let st = self.st();
                st.emitted.as_ref() == Some(&st.status)
            };
            if settled {
                return;
            }
        }
    }

    pub fn persist_config(&self, cfg: &Config) {
        if let Err(e) = config::save(&self.config_path, cfg) {
            tracing::warn!("não consegui gravar o config: {e}");
        }
        self.media.config_changed(cfg);
    }

    pub fn core(&self, cmd: CoreCmd) {
        let _ = self.core_tx.send(cmd);
        self.waker.wake();
    }

    pub fn net(&self, cmd: NetCmd) {
        let _ = self.net_tx.send(cmd);
    }

    /// Esquece o PC pareado (Keychain e estado). A sessão é derrubada por `NetCmd::StopSession`.
    pub fn forget_peer(&self) {
        let peer = self.mutate(|s| {
            s.session = None;
            s.peer.take()
        });
        if let Some(peer) = peer {
            let ks: Arc<dyn KeyStore> = self.platform.key_store();
            if let Err(e) = crypto::forget_active_peer(&*ks, ACTIVE_PEER_ACCOUNT, &peer.device_id) {
                tracing::warn!("não consegui remover o peer do Keychain: {e}");
            }
        }
    }

    /// Acorda o PC pareado (Wake-on-LAN).
    pub fn wake_pc(&self) -> Result<(), String> {
        let mac = self
            .st()
            .peer
            .as_ref()
            .map(|p| p.mac_address)
            .ok_or("nenhum PC pareado")?;
        relay_core::wol::send_wol(mac).map_err(|e| e.to_string())
    }
}

/// O núcleo do app Mac. Barato de clonar o `Arc` interno; `Drop` encerra as threads.
pub struct AppCore<P: Platform> {
    pub(crate) inner: Arc<Inner<P>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Assinaturas de sistema; valem enquanto vivem.
    _guards: Mutex<Vec<Box<dyn Any + Send>>>,
}

impl<P: Platform> AppCore<P> {
    /// Carrega config e identidade, confere permissões, sobe a thread de captura e o gerente de
    /// rede (no `runtime`). Chame da thread principal: o callback de telas do Mac precisa do
    /// run loop dela.
    pub fn start(
        platform: Arc<P>,
        events: Arc<dyn CoreEvents>,
        media: Arc<dyn MediaHooks>,
        opts: CoreOptions,
        runtime: Handle,
    ) -> Self {
        let config_path = platform.config_path();
        let config = config::load_or_default(&config_path).unwrap_or_else(|e| {
            tracing::warn!("config ilegível, usando padrões: {e}");
            Config::default()
        });
        let mut displays = platform.new_displays();
        let local = displays.list().unwrap_or_else(|e| {
            tracing::warn!("não consegui listar as telas: {e}");
            Vec::new()
        });
        let (core_tx, core_rx) = mpsc::channel();
        let (net_tx, net_rx) = unbounded_channel();
        let tx = core_tx.clone();
        let waker = LoopWaker::default();
        let displays_waker = waker.clone();
        displays.set_change_callback(Box::new(move || {
            let _ = tx.send(CoreCmd::DisplaysChanged);
            displays_waker.wake();
        }));

        let status = StatusSnapshot {
            status: ConnectionStatus::Disconnected,
            peer_name: None,
            focus: Host::Local,
            locked: false,
            paused: false,
            secure_input: false,
            peer_unreachable: false,
            peer_version: None,
            version_sync: Default::default(),
            clipboard_sync: config.clipboard.enabled,
            audio: AudioState::default(),
            local_os: Os::Mac,
            peer_os: None,
            local_caps: Caps::baseline(Os::Mac),
            peer_caps: None,
            layout_host: Host::Local,
            layout_editable: true,
        };
        let inner = Arc::new(Inner {
            platform: platform.clone(),
            events,
            media,
            opts,
            config_path,
            state: Mutex::new(State {
                status,
                emitted: None,
                config,
                peer: None,
                session: None,
                peer_version: None,
                session_gen: 0,
                dialing_since: None,
                pairing: false,
                local,
                remote: Vec::new(),
                profile: None,
                discovered: Vec::new(),
                latency: LatencyStats::default(),
            }),
            emit_lock: Mutex::new(()),
            core_tx,
            waker,
            net_tx,
        });

        inner.media.config_changed(&inner.st().config.clone());
        let thread = engine::spawn(inner.clone(), core_rx, displays);
        runtime.spawn(net::manager(inner.clone(), net_rx));

        let i = inner.clone();
        let sys = platform.watch_system(Box::new(move |e| {
            if matches!(e, SysEvent::DidWake | SysEvent::NetworkChanged) {
                i.net(NetCmd::Reconnect);
            }
        }));
        let i = inner.clone();
        let secure = platform.watch_secure_input(Box::new(move |active| {
            i.mutate(|s| s.status.secure_input = active);
            i.events.secure_input(active);
        }));
        Self {
            inner,
            thread: Mutex::new(Some(thread)),
            _guards: Mutex::new(vec![sys, secure]),
        }
    }

    pub fn status(&self) -> StatusSnapshot {
        self.inner.st().status.clone()
    }

    pub fn discovered(&self) -> Vec<DiscoveredPeer> {
        self.inner.st().discovered.clone()
    }

    pub fn displays(&self) -> DisplaysPayload {
        let st = self.inner.st();
        DisplaysPayload {
            local: st.local.clone(),
            peer: st.remote.clone(),
        }
    }

    pub fn layout(&self) -> LayoutProfile {
        let st = self.inner.st();
        st.profile.clone().unwrap_or_else(|| LayoutProfile {
            key: layout::profile_key(&st.local),
            name: String::new(),
            displays: Vec::new(),
            windows_detached: false,
        })
    }

    /// Valida, grava em `config.layout.profiles` (um por nome), torna-o o perfil em uso, aplica no
    /// Controller e manda `LayoutSync`. Devolve os avisos de Dock e Hot Corner.
    pub fn save_layout(&self, mut profile: LayoutProfile) -> Result<SaveLayoutResult, String> {
        layout::validate(&profile).map_err(|e| e.to_string())?;
        profile.name = profile.name.trim().to_owned();
        if profile.name.is_empty() {
            profile.name = layout::DEFAULT_PROFILE_NAME.to_owned();
        }
        let (dock, corners) = self.inner.platform.dock();
        let cfg = self.inner.mutate(|s| {
            let l = &mut s.config.layout;
            match l.profiles.iter_mut().find(|p| p.name == profile.name) {
                Some(p) => *p = profile.clone(),
                None => l.profiles.push(profile.clone()),
            }
            l.active = Some(profile.name.clone());
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        self.inner.core(CoreCmd::ProfilesChanged);
        let warnings =
            layout::dock_warnings(&profile, dock, corners, cfg.transition.corner_dead_zone_mm);
        Ok(SaveLayoutResult { warnings })
    }

    pub fn config(&self) -> Config {
        self.inner.st().config.clone()
    }

    /// Troca a lista de perfis (criar, renomear, excluir) e o perfil em uso, vindos do editor.
    pub fn set_layout_profiles(
        &self,
        profiles: Vec<LayoutProfile>,
        active: String,
    ) -> Result<(), String> {
        let mut names = std::collections::HashSet::new();
        for p in &profiles {
            let name = p.name.trim();
            if name.is_empty() {
                return Err("perfil sem nome".into());
            }
            if !names.insert(name) {
                return Err(format!("já existe um perfil chamado {name}"));
            }
        }
        if !names.contains(active.trim()) {
            return Err(format!("perfil {active} não existe"));
        }
        let cfg = self.inner.mutate(|s| {
            s.config.layout.profiles = profiles
                .into_iter()
                .map(|p| LayoutProfile {
                    name: p.name.trim().to_owned(),
                    ..p
                })
                .collect();
            s.config.layout.active = Some(active.trim().to_owned());
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        self.inner.core(CoreCmd::ProfilesChanged);
        Ok(())
    }

    /// Grava o config vindo da UI. Os perfis de layout só mudam por `save_layout`.
    pub fn save_config(&self, mut new: Config) -> Result<(), String> {
        let manual = new.network.manual_address.clone();
        if let Some(a) = &manual {
            relay_core::discovery::parse_manual_address(a).map_err(|e| e.to_string())?;
        }
        let cfg = self.inner.mutate(|s| {
            new.layout = s.config.layout.clone();
            s.status.clipboard_sync = new.clipboard.enabled;
            s.config = new;
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        self.inner.core(CoreCmd::ApplyConfig(Box::new(cfg)));
        self.inner.net(NetCmd::ConfigChanged);
        Ok(())
    }

    pub fn set_manual_address(&self, address: Option<String>) -> Result<(), String> {
        let address = address
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty());
        if let Some(a) = &address {
            relay_core::discovery::parse_manual_address(a).map_err(|e| e.to_string())?;
        }
        let cfg = self.inner.mutate(|s| {
            s.config.network.manual_address = address;
            s.config.clone()
        });
        self.inner.persist_config(&cfg);
        self.inner.net(NetCmd::ConfigChanged);
        Ok(())
    }

    /// Esquece o PC: remove o peer do Keychain e avisa o agente (`Message::Unpair`).
    pub fn unpair(&self) -> Result<(), String> {
        self.inner.forget_peer();
        self.inner.net(NetCmd::StopSession { notify: true });
        Ok(())
    }

    /// Derruba e refaz a sessão (também sai de `IncompatibleVersion`).
    pub fn reconnect(&self) {
        self.inner.net(NetCmd::Reconnect);
    }

    pub fn set_cursor_lock(&self, locked: bool) {
        self.inner.mutate(|s| s.status.locked = locked);
        self.inner.core(CoreCmd::SetLocked(locked));
    }

    pub fn jump_to(&self, n: u8) {
        self.inner.core(CoreCmd::JumpTo(n));
    }

    pub fn set_sharing_paused(&self, paused: bool) {
        self.inner.mutate(|s| s.status.paused = paused);
        self.inner.core(CoreCmd::SetPaused(paused));
    }

    pub fn latency(&self) -> LatencyStats {
        self.inner.st().latency
    }

    pub fn permissions(&self) -> PermissionsStatus {
        self.inner.platform.check_permissions()
    }

    /// Encerra a thread do core e o gerente de rede.
    pub fn shutdown(&self) {
        self.inner.core(CoreCmd::Shutdown);
        self.inner.net(NetCmd::Shutdown);
        let t = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = t {
            let _ = t.join();
        }
    }
}

impl<P: Platform> Drop for AppCore<P> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
