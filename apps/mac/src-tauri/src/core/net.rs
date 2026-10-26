//! dono: MI1
//! Gerente de rede (tokio): descoberta mDNS, anúncio, endereço manual, pareamento, escuta,
//! discagem e reconexão. A sessão é simétrica: o Mac sempre escuta (`SessionListener`, também
//! para mostrar o código de pareamento) e disca (`SessionDialer`) quando `dial_decision` manda.
//! Cada transporte tem uma bomba que leva as mensagens entre a sessão e a thread do core e
//! publica status e latência; só uma sessão vale por vez (`Inner::claim`).

use std::any::Any;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use relay_core::compat::v02::legacy_peer_os;
use relay_core::crypto::{
    LocalDevice, StaticKeypair, device_id_of, load_active_peer, load_or_create_identity,
    save_active_peer,
};
use relay_core::discovery::parse_manual_address;
use relay_core::net::{
    DialerConfig, FastEvent, FastSender, ListenerConfig, Session, SessionDialer, SessionEvent,
    SessionListener,
};
use relay_core::node::session::{DIAL_FALLBACK, DialDecision, caps_message, dial_decision};
use relay_core::platform::KeyStore;
use relay_core::types::{
    Caps, ConnectionStatus, DiscoveredPeer, Message, Os, PairedPeer, PairingEvent, PairingState,
    Ports, Role, VersionSync,
};
use relay_core::version::{APP_VERSION, peer_is_symmetric, update_request, version_sync};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout};

use super::engine::CoreCmd;
use super::platform::{AudioKeys, Platform, SessionCtx};
use super::{ACTIVE_PEER_ACCOUNT, Inner};

const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TICK: Duration = Duration::from_secs(1);
/// Tempo para o `Message::Unpair` sair antes de derrubar a sessão.
const UNPAIR_FLUSH: Duration = Duration::from_millis(150);
/// Entre duas tentativas de abrir a escuta quando a porta está ocupada.
const LISTEN_RETRY: Duration = Duration::from_secs(5);

pub(crate) enum NetCmd {
    Pair {
        code: String,
        address: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Mostra um código de pareamento (Mac como Listener).
    OpenPairing {
        code: String,
        ttl: Duration,
        expires_at: u64,
    },
    /// Derruba as sessões (o peer já foi esquecido por `Inner::forget_peer`).
    StopSession {
        notify: bool,
    },
    /// O peer deixou de ser pareado: a escuta para de aceitá-lo.
    PeerForgotten(String),
    Discovered(DiscoveredPeer),
    /// Endereço manual ou portas mudaram.
    ConfigChanged,
    /// Acordou, rede mudou ou o usuário pediu: refaz a sessão, o anúncio e a busca.
    Reconnect,
    /// A bomba da época `epoch` terminou.
    PumpEnded {
        epoch: u64,
    },
    Shutdown,
}

pub(crate) enum PumpCmd {
    Reconnect,
    Stop {
        notify: bool,
    },
    /// Outra sessão ficou com o lugar (ou esta foi recusada): sai.
    Drop,
    /// Encerra a bomba e libera as portas.
    Exit,
    OpenPairing {
        code: String,
        ttl: Duration,
    },
    AddPeer(PairedPeer),
    RemovePeer(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dialer,
    Listener,
}

struct DialerSlot {
    ctl: UnboundedSender<PumpCmd>,
    target: (IpAddr, u16),
    join: JoinHandle<()>,
    epoch: u64,
}

struct ListenerSlot {
    ctl: UnboundedSender<PumpCmd>,
    join: JoinHandle<()>,
    epoch: u64,
}

struct Manager<P: Platform> {
    inner: Arc<Inner<P>>,
    identity: Option<StaticKeypair>,
    dialer: Option<DialerSlot>,
    listener: Option<ListenerSlot>,
    /// Anúncio mDNS: vale enquanto o guarda viver.
    advert: Option<std::sync::Mutex<Box<dyn Any + Send>>>,
    listen_retry: Instant,
    /// Portas reais de escuta (controle, input), para refazer o anúncio.
    listen_ports: Option<(u16, u16)>,
    epoch: u64,
    /// Quando a busca por este peer começou (a espera pelo mDNS vale a partir daqui).
    searching_since: Instant,
    /// Endereço em que o pareamento acabou de funcionar.
    hint: Option<(IpAddr, u16)>,
    discovery_epoch: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

pub(crate) async fn manager<P: Platform>(inner: Arc<Inner<P>>, mut rx: UnboundedReceiver<NetCmd>) {
    let mut m = Manager {
        searching_since: Instant::now(),
        inner,
        identity: None,
        dialer: None,
        listener: None,
        advert: None,
        listen_retry: Instant::now(),
        listen_ports: None,
        epoch: 0,
        hint: None,
        discovery_epoch: Arc::new(AtomicU64::new(0)),
        stop: Arc::new(AtomicBool::new(false)),
    };
    m.load().await;
    let mut tick = interval(CONNECT_TICK);
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None | Some(NetCmd::Shutdown) => break,
                Some(cmd) => m.on_cmd(cmd).await,
            },
            _ = tick.tick() => m.on_tick().await,
        }
    }
    m.stop.store(true, Ordering::Relaxed);
    m.stop_session(false).await;
    m.stop_listener().await;
}

impl<P: Platform> Manager<P> {
    /// Identidade e peer pareado do Keychain; depois liga a descoberta e a escuta.
    async fn load(&mut self) {
        let ks = self.inner.platform.key_store();
        let loaded = tokio::task::spawn_blocking(move || load_identity(&*ks)).await;
        match loaded {
            Ok(Ok((identity, peer))) => {
                self.inner.mutate(|s| s.peer = peer);
                self.searching_since = Instant::now();
                self.start_discovery(device_id_of(&identity.public));
                self.identity = Some(identity);
                self.ensure_listener().await;
            }
            Ok(Err(e)) => tracing::error!("Keychain indisponível: {e}"),
            Err(e) => tracing::error!("falha ao ler o Keychain: {e}"),
        }
    }

    fn next_epoch(&mut self) -> u64 {
        self.epoch += 1;
        self.epoch
    }

    fn start_discovery(&self, own_id: String) {
        let platform = self.inner.platform.clone();
        let tx = self.inner.net_tx.clone();
        let (epoch, stop) = (self.discovery_epoch.clone(), self.stop.clone());
        let spawned = std::thread::Builder::new()
            .name("relay-mdns".into())
            .spawn(move || discovery_loop(&*platform, &tx, &own_id, &epoch, &stop));
        if let Err(e) = spawned {
            tracing::warn!("busca mDNS não iniciou: {e}");
        }
    }

    async fn on_tick(&mut self) {
        let expired = self
            .inner
            .st()
            .pairing_until
            .is_some_and(|t| Instant::now() >= t);
        if expired {
            self.inner.mutate(|s| s.pairing_until = None);
        }
        self.ensure_listener().await;
        self.maybe_connect().await;
    }

    async fn on_cmd(&mut self, cmd: NetCmd) {
        match cmd {
            NetCmd::Pair {
                code,
                address,
                reply,
            } => {
                let r = self.pair(code, address).await;
                let _ = reply.send(r);
            }
            NetCmd::OpenPairing {
                code,
                ttl,
                expires_at,
            } => {
                let Some(l) = &self.listener else { return };
                let _ = l.ctl.send(PumpCmd::OpenPairing {
                    code: code.clone(),
                    ttl,
                });
                self.inner
                    .mutate(|s| s.pairing_until = Some(Instant::now() + ttl));
                self.inner.events.pairing(&PairingEvent {
                    state: PairingState::Code,
                    code: Some(code),
                    expires_at: Some(expires_at),
                });
            }
            NetCmd::StopSession { notify } => {
                self.stop_session(notify).await;
                self.searching_since = Instant::now();
            }
            NetCmd::PeerForgotten(id) => {
                if let Some(l) = &self.listener {
                    let _ = l.ctl.send(PumpCmd::RemovePeer(id));
                }
            }
            NetCmd::Discovered(peer) => {
                self.inner.mutate(|s| {
                    s.discovered.retain(|p| p.device_id != peer.device_id);
                    s.discovered.push(peer);
                });
                self.maybe_connect().await;
            }
            NetCmd::ConfigChanged => self.maybe_connect().await,
            NetCmd::Reconnect => {
                self.searching_since = Instant::now();
                self.discovery_epoch.fetch_add(1, Ordering::Relaxed);
                // A rede mudou: o anúncio refaz com os endereços novos.
                self.advert = None;
                self.advertise();
                if let Some(a) = &self.dialer {
                    let _ = a.ctl.send(PumpCmd::Reconnect);
                }
                if let Some(l) = &self.listener {
                    let _ = l.ctl.send(PumpCmd::Reconnect);
                }
            }
            NetCmd::PumpEnded { epoch } => self.on_pump_ended(epoch),
            NetCmd::Shutdown => {}
        }
    }

    // ---- escuta ----

    /// Abre a escuta e o anúncio (e os refaz se a bomba caiu). Porta ocupada: avisa e tenta de novo.
    async fn ensure_listener(&mut self) {
        if self.listener.is_some() || Instant::now() < self.listen_retry {
            return;
        }
        let Some(identity) = self.identity.clone() else {
            return;
        };
        let (cfg, peer) = {
            let st = self.inner.st();
            (st.config.clone(), st.peer.clone())
        };
        let ports = self
            .inner
            .opts
            .local_ports
            .clone()
            .unwrap_or_else(|| cfg.network.ports.clone());
        let device = LocalDevice {
            name: self.inner.platform.device_name(),
            public_key: identity.public,
            mac_address: [0; 6],
        };
        let mut lcfg =
            ListenerConfig::new(identity.clone(), device, peer.into_iter().collect(), ports);
        lcfg.bind_ip = self.inner.opts.bind_ip;
        if let Some(v) = &self.inner.opts.app_version {
            lcfg.app_version = v.clone();
        }
        let fast_inner = self.inner.clone();
        let fast_sink = Box::new(move |ev: FastEvent| fast_inner.core(CoreCmd::Fast(ev.event)));
        let mut listener = match SessionListener::start(lcfg, fast_sink).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("escuta não iniciou: {e}");
                self.listen_retry = Instant::now() + LISTEN_RETRY;
                return;
            }
        };
        let fast = listener.take_input_producer();
        self.advertise_with(&listener);
        let epoch = self.next_epoch();
        let media = MediaLink {
            identity,
            peer: None,
            peer_ip: None,
            remote_ports: cfg.network.ports.clone(),
            local_ports: self
                .inner
                .opts
                .local_ports
                .clone()
                .unwrap_or_else(|| cfg.network.ports.clone()),
            bind_ip: self.inner.opts.bind_ip,
        };
        let (ctl, join) =
            self.spawn_pump(epoch, Kind::Listener, Link::Listener(listener), fast, media);
        self.listener = Some(ListenerSlot { ctl, join, epoch });
        self.inner.mutate(|s| s.listening = true);
    }

    fn advertise_with(&mut self, listener: &SessionListener) {
        let Some(identity) = &self.identity else {
            return;
        };
        let id = device_id_of(&identity.public);
        let (control, input) = (
            listener.local_control_addr().port(),
            listener.local_input_addr().port(),
        );
        self.listen_ports = Some((control, input));
        match self.inner.platform.advertise(&id, control, input) {
            Ok(guard) => self.advert = Some(std::sync::Mutex::new(guard)),
            Err(e) => tracing::warn!("mDNS indisponível; só pelo endereço manual: {e}"),
        }
    }

    /// Refaz o anúncio com as portas da escuta atual (rede nova).
    fn advertise(&mut self) {
        let Some(identity) = &self.identity else {
            return;
        };
        if self.listener.is_none() {
            return;
        }
        let id = device_id_of(&identity.public);
        let Some((control, input)) = self.listen_ports else {
            return;
        };
        match self.inner.platform.advertise(&id, control, input) {
            Ok(guard) => self.advert = Some(std::sync::Mutex::new(guard)),
            Err(e) => tracing::warn!("mDNS indisponível; só pelo endereço manual: {e}"),
        }
    }

    async fn stop_listener(&mut self) {
        self.advert = None;
        if let Some(l) = self.listener.take() {
            let _ = l.ctl.send(PumpCmd::Exit);
            let _ = timeout(Duration::from_secs(3), l.join).await;
        }
        self.inner.mutate(|s| s.listening = false);
    }

    fn spawn_pump(
        &mut self,
        epoch: u64,
        kind: Kind,
        link: Link,
        fast: Option<FastSender>,
        media: MediaLink,
    ) -> (UnboundedSender<PumpCmd>, JoinHandle<()>) {
        let (out_tx, out_rx) = unbounded_channel();
        let (ctl_tx, ctl_rx) = unbounded_channel();
        let own_id = self
            .identity
            .as_ref()
            .map(|i| device_id_of(&i.public))
            .unwrap_or_default();
        let status = link.session().status();
        let pump = Pump {
            inner: self.inner.clone(),
            link,
            status,
            epoch,
            kind,
            own_id,
            out_tx,
            fast,
            media,
            granted: false,
            last_up: false,
            peer: None,
            renew: false,
        };
        self.inner.st().pumps.push((epoch, ctl_tx.clone()));
        // A `Session` não é `Sync`: a bomba (que a empresta entre awaits) não pode ir para o
        // pool de tarefas; roda numa thread bloqueante dirigindo o futuro no runtime.
        let rt = tokio::runtime::Handle::current();
        let join = tokio::task::spawn_blocking(move || {
            rt.block_on(pump.run(out_rx, ctl_rx));
        });
        (ctl_tx, join)
    }

    fn on_pump_ended(&mut self, epoch: u64) {
        if self.listener.as_ref().is_some_and(|l| l.epoch == epoch) {
            self.listener = None;
            self.advert = None;
            self.inner.mutate(|s| s.listening = false);
        }
        if self.dialer.as_ref().is_some_and(|d| d.epoch == epoch) {
            self.dialer = None;
            self.cleanup_dialer(epoch, false);
        }
    }

    /// Solta o que a sessão discada `epoch` deixou: estado e Controller, se eram dela.
    fn cleanup_dialer(&mut self, epoch: u64, force: bool) {
        let fresh = self.next_epoch();
        let attached = self.inner.mutate(|s| {
            let attached = s.attached == Some(epoch);
            if attached {
                s.attached = None;
            }
            // Com outra sessão de pé o estado já é dela.
            if force || s.active.is_none() {
                s.session_gen = fresh;
                s.dialing_since = None;
                s.session = None;
            }
            attached
        });
        if attached {
            self.inner.core(CoreCmd::SessionDown);
        }
    }

    // ---- pareamento ----

    async fn pair(&mut self, code: String, address: Option<String>) -> Result<(), String> {
        let pairing_event = |state| PairingEvent {
            state,
            code: None,
            expires_at: None,
        };
        let result = self.pair_inner(&code, address).await;
        self.inner.mutate(|s| s.pairing = false);
        match result {
            Ok(()) => {
                self.inner
                    .events
                    .pairing(&pairing_event(PairingState::Paired));
                self.searching_since = Instant::now();
                self.maybe_connect().await;
                Ok(())
            }
            Err(e) => {
                self.inner.mutate(|s| s.pending_layout_host = None);
                self.inner
                    .events
                    .pairing(&pairing_event(PairingState::Failed));
                Err(e)
            }
        }
    }

    async fn pair_inner(&mut self, code: &str, address: Option<String>) -> Result<(), String> {
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err("O código deve ter 6 dígitos.".into());
        }
        let identity = self
            .identity
            .clone()
            .ok_or("Identidade indisponível (Keychain).")?;
        let addr = self.pair_target(address).await?;
        self.inner.mutate(|s| s.pairing = true);
        let me = LocalDevice {
            name: self.inner.platform.device_name(),
            public_key: identity.public,
            mac_address: [0; 6],
        };
        let peer = timeout(PAIR_TIMEOUT, SessionDialer::pair(addr, code, &me))
            .await
            .map_err(|_| "O PC não respondeu a tempo.".to_string())?
            .map_err(|e| e.to_string())?;
        // Reparear (mesmo PC ou outro) invalida a sessão atual.
        self.stop_session(false).await;
        let ks = self.inner.platform.key_store();
        let old = self.inner.st().peer.clone();
        save_active_peer(&*ks, ACTIVE_PEER_ACCOUNT, &peer).map_err(|e| e.to_string())?;
        if let Some(l) = &self.listener {
            let _ = l.ctl.send(PumpCmd::AddPeer(peer.clone()));
        }
        if let Some(old) = old.filter(|o| o.device_id != peer.device_id) {
            let _ = relay_core::crypto::remove_peer(&*ks, &old.device_id);
            if let Some(l) = &self.listener {
                let _ = l.ctl.send(PumpCmd::RemovePeer(old.device_id));
            }
        }
        self.hint = Some((addr.ip(), addr.port()));
        // Já sabemos onde ele está: não espera o mDNS.
        self.searching_since = Instant::now()
            .checked_sub(self.inner.opts.manual_grace)
            .unwrap_or_else(Instant::now);
        self.inner.mutate(|s| {
            s.peer = Some(peer);
            s.peer_live = None;
            s.session = None;
            s.idle_since = Instant::now();
        });
        Ok(())
    }

    /// `address` (com ou sem porta) ou, sem ele, o único PC descoberto / o endereço do config.
    async fn pair_target(&self, address: Option<String>) -> Result<SocketAddr, String> {
        let (host, port) = match address
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
        {
            Some(a) => parse_manual_address(&a).map_err(|e| e.to_string())?,
            None => {
                let (found, manual) = {
                    let st = self.inner.st();
                    (
                        st.discovered.clone(),
                        st.config.network.manual_address.clone(),
                    )
                };
                match (found.as_slice(), manual) {
                    ([p], _) => (
                        p.addrs
                            .first()
                            .cloned()
                            .ok_or("O PC não informou endereço.")?,
                        p.port,
                    ),
                    (_, Some(m)) => parse_manual_address(&m).map_err(|e| e.to_string())?,
                    ([], None) => return Err("Nenhum PC encontrado: informe o endereço.".into()),
                    _ => return Err("Mais de um PC encontrado: escolha um.".into()),
                }
            }
        };
        resolve(&host, port)
            .await
            .ok_or_else(|| format!("Não consegui resolver {host}."))
    }

    // ---- sessão ----

    /// Destino da sessão: PC descoberto; senão, depois da espera, o endereço manual / de pareamento.
    async fn pick_target(&self, peer: &PairedPeer) -> Option<(IpAddr, u16)> {
        let (found, manual) = {
            let st = self.inner.st();
            (
                st.discovered
                    .iter()
                    .find(|d| d.device_id == peer.device_id)
                    .cloned(),
                st.config.network.manual_address.clone(),
            )
        };
        if let Some(d) = found {
            let mut ips: Vec<IpAddr> = d.addrs.iter().filter_map(|a| a.parse().ok()).collect();
            ips.sort_by_key(|ip| (!ip.is_ipv4(), is_link_local(ip)));
            if let Some(ip) = ips.first() {
                return Some((*ip, d.port));
            }
        }
        if self.searching_since.elapsed() < self.inner.opts.manual_grace {
            return None;
        }
        if let Some(m) = manual {
            let (host, port) = parse_manual_address(&m).ok()?;
            return resolve(&host, port).await.map(|a| (a.ip(), a.port()));
        }
        self.hint
    }

    async fn maybe_connect(&mut self) {
        if self.dialer.is_some() {
            // Recalcula `pc_unreachable` com o passar do tempo; só emite se o snapshot mudar.
            self.inner.mutate(|_| {});
        }
        let (peer, pairing, active, found, idle) = {
            let st = self.inner.st();
            let found = st
                .peer
                .as_ref()
                .and_then(|p| st.discovered.iter().find(|d| d.device_id == p.device_id))
                .cloned();
            (
                st.peer.clone(),
                st.pairing,
                st.active.is_some(),
                found,
                st.idle_since,
            )
        };
        // Sessão de pé (de quem for): não há o que discar.
        let Some(peer) = peer.filter(|_| !pairing && !active && self.identity.is_some()) else {
            return;
        };
        if self.dialer.is_none()
            && let (Some(d), Some(id)) = (&found, &self.identity)
        {
            match dial_decision(&device_id_of(&id.public), d) {
                DialDecision::Ignore => return,
                // O lado de id menor disca primeiro; este só entra se nada se conectou.
                DialDecision::DialAfterFallback if idle.elapsed() < DIAL_FALLBACK => return,
                _ => {}
            }
        }
        let Some(target) = self.pick_target(&peer).await else {
            return;
        };
        match &self.dialer {
            None => self.start_session(peer, target).await,
            Some(a) if a.target != target => {
                // O PC mudou de endereço: só troca se a sessão atual não está de pé.
                let connected = self.inner.st().session == Some(ConnectionStatus::Connected);
                if !connected {
                    self.stop_session(false).await;
                    self.start_session(peer, target).await;
                }
            }
            Some(_) => {}
        }
    }

    async fn start_session(&mut self, peer: PairedPeer, target: (IpAddr, u16)) {
        let Some(identity) = self.identity.clone() else {
            return;
        };
        let cfg = self.inner.st().config.clone();
        let remote_ports = Ports {
            control: target.1,
            ..cfg.network.ports.clone()
        };
        let local_ports = self
            .inner
            .opts
            .local_ports
            .clone()
            .unwrap_or_else(|| cfg.network.ports.clone());
        // O UDP de input da escuta já ocupa a porta local: o PC aprende a desta sessão pelo
        // primeiro pacote.
        let mut dial = DialerConfig::new(
            identity.clone(),
            peer.clone(),
            target.0,
            remote_ports.clone(),
            Ports {
                input: 0,
                ..local_ports.clone()
            },
        );
        dial.bind_ip = self.inner.opts.bind_ip;
        if let Some(v) = &self.inner.opts.app_version {
            dial.app_version = v.clone();
        }
        // Canal rápido do PC (movimento/rolagem do sentido inverso) vai para a thread do core.
        let fast_inner = self.inner.clone();
        let fast_sink = Box::new(move |ev: FastEvent| fast_inner.core(CoreCmd::Fast(ev.event)));
        let mut dialer = match SessionDialer::start(dial, fast_sink).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("sessão não iniciou: {e}");
                return;
            }
        };
        let Some(fast) = dialer.take_input_producer() else {
            return;
        };
        let epoch = self.next_epoch();
        let media = MediaLink {
            identity,
            peer: Some(peer.clone()),
            peer_ip: Some(target.0),
            remote_ports,
            local_ports,
            bind_ip: self.inner.opts.bind_ip,
        };
        self.inner.mutate(|s| {
            s.session_gen = epoch;
            s.dialing_since = Some(Instant::now());
            s.session = Some(ConnectionStatus::Connecting);
        });
        let (ctl, join) =
            self.spawn_pump(epoch, Kind::Dialer, Link::Dialer(dialer), Some(fast), media);
        self.dialer = Some(DialerSlot {
            ctl,
            target,
            join,
            epoch,
        });
    }

    /// Derruba a sessão discada e a que o PC abriu na escuta (a escuta continua aberta).
    async fn stop_session(&mut self, notify: bool) {
        if let Some(l) = &self.listener {
            let _ = l.ctl.send(PumpCmd::Stop { notify });
        }
        if let Some(a) = self.dialer.take() {
            let _ = a.ctl.send(PumpCmd::Stop { notify });
            let _ = timeout(Duration::from_secs(3), a.join).await;
            self.cleanup_dialer(a.epoch, true);
        } else {
            let fresh = self.next_epoch();
            self.inner.mutate(|s| {
                s.session_gen = fresh;
                s.dialing_since = None;
                s.session = None;
            });
        }
    }
}

type Loaded = (StaticKeypair, Option<PairedPeer>);

fn load_identity(ks: &dyn KeyStore) -> Result<Loaded, String> {
    let identity = load_or_create_identity(ks).map_err(|e| e.to_string())?;
    let peer = load_active_peer(ks, ACTIVE_PEER_ACCOUNT).map_err(|e| e.to_string())?;
    Ok((identity, peer))
}

fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

async fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .ok()?
        .collect::<Vec<_>>();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs.into_iter().next()
}

/// Thread da busca mDNS: reinicia o `Browse` quando `epoch` muda (rede nova) e ignora o próprio
/// anúncio comparando o `device_id`.
fn discovery_loop<P: Platform>(
    platform: &P,
    tx: &UnboundedSender<NetCmd>,
    own_id: &str,
    epoch: &AtomicU64,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        let mine = epoch.load(Ordering::Relaxed);
        match platform.browse() {
            Ok(browse) => {
                while !stop.load(Ordering::Relaxed) && epoch.load(Ordering::Relaxed) == mine {
                    if let Some(p) = browse.next_peer(Duration::from_millis(500))
                        && p.device_id != own_id
                        && tx.send(NetCmd::Discovered(p)).is_err()
                    {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("mDNS indisponível: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

/// O que a mídia precisa saber da sessão além do que a `Session` expõe. Na escuta, o peer e o
/// endereço dele só se conhecem com a sessão de pé.
struct MediaLink {
    identity: StaticKeypair,
    peer: Option<PairedPeer>,
    peer_ip: Option<IpAddr>,
    remote_ports: Ports,
    local_ports: Ports,
    bind_ip: IpAddr,
}

/// O transporte de uma bomba: discado por nós ou aceito na escuta.
enum Link {
    Dialer(SessionDialer),
    Listener(SessionListener),
}

impl Link {
    fn session(&self) -> &Session {
        match self {
            Link::Dialer(d) => d,
            Link::Listener(l) => l,
        }
    }

    fn session_mut(&mut self) -> &mut Session {
        match self {
            Link::Dialer(d) => d,
            Link::Listener(l) => l,
        }
    }

    async fn shutdown(self) {
        match self {
            Link::Dialer(d) => d.shutdown().await,
            Link::Listener(l) => l.shutdown().await,
        }
    }
}

/// Dona da `Session`: leva mensagens entre ela e a thread do core, e publica status e latência
/// (≤ 1 Hz). Só a sessão que passou por `Inner::claim` escreve no estado e fala com o core.
struct Pump<P: Platform> {
    inner: Arc<Inner<P>>,
    link: Link,
    status: watch::Receiver<ConnectionStatus>,
    epoch: u64,
    kind: Kind,
    own_id: String,
    out_tx: UnboundedSender<Message>,
    /// Fila do canal rápido, até o Controller a levar (`SessionUp`).
    fast: Option<FastSender>,
    media: MediaLink,
    /// Esta sessão está de pé e é a que vale.
    granted: bool,
    last_up: bool,
    peer: Option<PairedPeer>,
    /// A escuta perdeu o canal rápido para o Controller de outra sessão: refaz-se.
    renew: bool,
}

impl<P: Platform> Pump<P> {
    async fn run(
        mut self,
        mut out_rx: UnboundedReceiver<Message>,
        mut ctl_rx: UnboundedReceiver<PumpCmd>,
    ) {
        let mut status = self.status.clone();
        let stats = self.link.session().stats();
        let mut lat = interval(Duration::from_secs(1));
        let mut notify = false;
        let first = *status.borrow_and_update();
        let mut go = self.on_status(first);
        while go {
            tokio::select! {
                ev = self.link.session_mut().recv_event() => match ev {
                    None => break,
                    Some(ev) => go = self.on_event(ev),
                },
                Some(m) = out_rx.recv() => {
                    let _ = self.link.session().send(m).await;
                }
                r = status.changed() => {
                    if r.is_err() { break; }
                    let s = *status.borrow_and_update();
                    go = self.on_status(s);
                }
                _ = lat.tick() => {
                    if self.last_up {
                        let l = *stats.borrow();
                        self.inner.mutate(|s| s.latency = l);
                        self.inner.events.latency(&l);
                    }
                }
                cmd = ctl_rx.recv() => match cmd {
                    Some(PumpCmd::Stop { notify: n }) if self.kind == Kind::Dialer => {
                        notify = n;
                        break;
                    }
                    Some(PumpCmd::Exit) | Some(PumpCmd::Drop) if self.kind == Kind::Dialer => break,
                    Some(PumpCmd::Exit) | None => break,
                    Some(cmd) => self.on_cmd(cmd).await,
                },
            }
        }
        if notify {
            let _ = self.link.session().send(Message::Unpair).await;
            sleep(UNPAIR_FLUSH).await;
        }
        let Pump {
            inner,
            link,
            epoch,
            granted,
            ..
        } = self;
        link.shutdown().await;
        {
            let mut st = inner.st();
            st.pumps.retain(|(e, _)| *e != epoch);
        }
        if granted {
            inner.release(epoch);
        }
        inner.net(NetCmd::PumpEnded { epoch });
    }

    /// Comandos que não encerram a bomba. Devolve nada: quem encerra decide no `select`.
    async fn on_cmd(&mut self, cmd: PumpCmd) {
        match cmd {
            PumpCmd::Reconnect => match self.kind {
                Kind::Dialer => self.link.session().force_reconnect(),
                Kind::Listener => self.drop_session(false).await,
            },
            PumpCmd::Stop { notify } => self.drop_session(notify).await,
            PumpCmd::Drop => {
                // Outra sessão ficou com o lugar: a nossa sai sem mexer no estado.
                if self.granted {
                    self.granted = false;
                    self.drop_session(false).await;
                }
            }
            PumpCmd::OpenPairing { code, ttl } => {
                if let Link::Listener(l) = &self.link {
                    l.open_pairing(code, ttl);
                }
            }
            PumpCmd::AddPeer(p) => {
                if let Link::Listener(l) = &self.link {
                    l.add_peer(p);
                }
            }
            PumpCmd::RemovePeer(id) => {
                if let Link::Listener(l) = &self.link {
                    l.remove_peer(&id);
                }
            }
            PumpCmd::Exit => {}
        }
    }

    /// Derruba a sessão aceita na escuta (a escuta continua); sem sessão de pé não faz nada, para
    /// o pedido de reconexão não ficar guardado para a próxima.
    async fn drop_session(&mut self, notify: bool) {
        if !self.granted && *self.status.borrow() != ConnectionStatus::Connected {
            return;
        }
        if notify {
            let _ = self.link.session().send(Message::Unpair).await;
            sleep(UNPAIR_FLUSH).await;
        }
        self.link.session().force_reconnect();
    }

    /// `false`: a bomba deve encerrar.
    fn on_status(&mut self, s: ConnectionStatus) -> bool {
        if s == ConnectionStatus::Connected && self.kind == Kind::Dialer && !self.granted {
            self.peer = self.media.peer.clone();
            if !self.on_up() {
                return false;
            }
        }
        if s != ConnectionStatus::Connected && self.granted {
            self.granted = false;
            self.inner.release(self.epoch);
        }
        self.publish(s);
        true
    }

    fn on_event(&mut self, ev: SessionEvent) -> bool {
        match ev {
            SessionEvent::Message(m) => {
                // O `select!` pode entregar a mensagem antes da mudança de status que a precede.
                if !self.granted && self.kind == Kind::Dialer {
                    let s = *self.status.borrow();
                    if !self.on_status(s) {
                        return false;
                    }
                }
                if self.granted {
                    self.on_message(m);
                }
            }
            SessionEvent::Connected(peer) => {
                if self.kind == Kind::Listener {
                    self.peer = Some(peer);
                    if !self.on_up() {
                        return false;
                    }
                    let s = *self.status.borrow();
                    self.publish(s);
                }
            }
            SessionEvent::Paired(peer) => self.on_paired(peer),
            SessionEvent::PairFailed(reason) => {
                tracing::warn!(%reason, "pareamento falhou");
                self.inner.mutate(|s| s.pairing_until = None);
                self.inner
                    .events
                    .pairing(&pairing_event(PairingState::Failed));
            }
            SessionEvent::Refused => tracing::warn!("conexão de peer não pareado recusada"),
        }
        true
    }

    /// A sessão ficou de pé. `false`: esta bomba não deve continuar.
    fn on_up(&mut self) -> bool {
        let dialer_id = match (self.kind, &self.peer) {
            (Kind::Dialer, _) => self.own_id.clone(),
            (Kind::Listener, Some(p)) => p.device_id.clone(),
            (Kind::Listener, None) => return true,
        };
        if !self.inner.claim(self.epoch, &dialer_id) {
            // Já há outra sessão com este par e ela vence: esta cai (Bye) e o par a abandona.
            return match self.kind {
                Kind::Dialer => false,
                Kind::Listener => {
                    self.link.session().force_reconnect();
                    true
                }
            };
        }
        self.granted = true;
        // A escuta pode ter sessão trocada pela mesma conexão (Replaced): Link recomeça.
        if self.last_up {
            self.last_up = false;
            self.inner.core(CoreCmd::Link(false));
        }
        let attached = self.inner.st().attached == Some(self.epoch);
        if !attached {
            match self.fast.take() {
                Some(fast) => {
                    self.inner.mutate(|s| s.attached = Some(self.epoch));
                    self.inner.core(CoreCmd::SessionUp {
                        fast,
                        out: self.out_tx.clone(),
                    });
                }
                None => {
                    // O canal rápido já foi para outra sessão: refaz-se o transporte.
                    self.granted = false;
                    self.inner.release(self.epoch);
                    self.renew = true;
                    return false;
                }
            }
        }
        self.announce();
        true
    }

    /// Pedido de atualização, Caps e contexto da mídia da sessão que acabou de subir.
    fn announce(&mut self) {
        let session = self.link.session();
        let peer_version = session.peer_version();
        let symmetric = peer_is_symmetric(peer_version.as_deref());
        let peer_os = if symmetric {
            self.inner.st().status.peer_os.unwrap_or(Os::Windows)
        } else {
            // 0.2 não anuncia o SO: quem disca era o Mac e quem escutava, o PC.
            let role = match self.kind {
                Kind::Dialer => Role::Listener,
                Kind::Listener => Role::Dialer,
            };
            let os = legacy_peer_os(role);
            self.inner.mutate(|s| {
                if s.session_gen == self.epoch {
                    s.peer_live = Some(Caps::baseline(os));
                }
            });
            os
        };
        if let Some(request) = update_request(APP_VERSION, peer_version.as_deref(), peer_os) {
            let _ = self.out_tx.send(request);
        }
        let local = self.inner.st().status.local_caps;
        if let Some(caps) = caps_message(local, peer_version.as_deref()) {
            let _ = self.out_tx.send(caps);
        }
        let audio_keys = session.audio_link().map(|l| AudioKeys {
            mac_to_pc: l.mac_to_pc,
        });
        let peer_ip = match self.kind {
            Kind::Dialer => self.media.peer_ip,
            Kind::Listener => session.audio_link().map(|l| l.remote_ip),
        };
        let (Some(peer_ip), Some(peer)) = (peer_ip, self.peer.clone()) else {
            tracing::warn!("sessão sem endereço do par: mídia desligada");
            return;
        };
        self.inner.core(CoreCmd::MediaUp(Box::new(SessionCtx {
            send: self.out_tx.clone(),
            peer_ip,
            remote_ports: self.media.remote_ports.clone(),
            local_ports: self.media.local_ports.clone(),
            bind_ip: self.media.bind_ip,
            identity: self.media.identity.clone(),
            peer,
            audio_keys,
        })));
    }

    fn publish(&mut self, s: ConnectionStatus) {
        let peer_version = self.link.session().peer_version();
        let epoch = self.epoch;
        let current = self.inner.mutate(|st| {
            let ok = st.session_gen == epoch;
            if ok {
                st.session = Some(s);
                st.peer_version = peer_version;
            }
            ok
        });
        if !current {
            return;
        }
        let up = s == ConnectionStatus::Connected;
        if up != self.last_up {
            self.last_up = up;
            self.inner.core(CoreCmd::Link(up));
        }
    }

    fn on_message(&mut self, m: Message) {
        match m {
            Message::Caps(caps) => self.inner.mutate(|s| s.peer_live = Some(caps)),
            // O pedido só vale se o PC está de fato numa versão mais nova que esta.
            Message::RequestUpdate { version }
                if version_sync(APP_VERSION, Some(&version)) == VersionSync::LocalBehind =>
            {
                self.inner.events.update_available(&version);
            }
            Message::RequestUpdate { .. } => {}
            m => self.inner.core(CoreCmd::Peer(m)),
        }
    }

    /// Pareamento por código mostrado: o PC digitou o código e virou par.
    fn on_paired(&mut self, peer: PairedPeer) {
        let Link::Listener(l) = &self.link else {
            return;
        };
        let ks = self.inner.platform.key_store();
        if let Err(e) = save_active_peer(&*ks, ACTIVE_PEER_ACCOUNT, &peer) {
            tracing::error!("não consegui guardar o peer pareado: {e}");
            l.remove_peer(&peer.device_id);
            self.inner.mutate(|s| s.pairing_until = None);
            self.inner
                .events
                .pairing(&pairing_event(PairingState::Failed));
            return;
        }
        let old = self.inner.st().peer.clone();
        let replaced = old.filter(|o| o.device_id != peer.device_id);
        if let Some(old) = &replaced {
            let _ = relay_core::crypto::remove_peer(&*ks, &old.device_id);
            l.remove_peer(&old.device_id);
        }
        self.inner.mutate(|s| {
            s.peer = Some(peer);
            s.peer_live = None;
            s.pairing_until = None;
            s.idle_since = Instant::now();
        });
        self.inner
            .events
            .pairing(&pairing_event(PairingState::Paired));
        if replaced.is_some() {
            // Outro PC: a sessão com o anterior não vale mais.
            self.inner.net(NetCmd::StopSession { notify: false });
        }
    }
}

fn pairing_event(state: PairingState) -> PairingEvent {
    PairingEvent {
        state,
        code: None,
        expires_at: None,
    }
}
