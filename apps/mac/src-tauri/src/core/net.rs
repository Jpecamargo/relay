//! dono: MI1
//! Gerente de rede (tokio): descoberta mDNS, endereço manual, pareamento, discagem e
//! reconexão. É o dono da `Session`: uma bomba por sessão leva as mensagens entre ela e a
//! thread do core e publica status e latência.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use relay_core::crypto::{
    LocalDevice, StaticKeypair, device_id_of, load_active_peer, load_or_create_identity,
    save_active_peer,
};
use relay_core::discovery::parse_manual_address;
use relay_core::net::{DialerConfig, FastEvent, SessionDialer, SessionEvent};
use relay_core::platform::KeyStore;
use relay_core::types::{
    ConnectionStatus, DiscoveredPeer, Message, PairedPeer, PairingEvent, PairingState, Ports,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout};

use super::engine::CoreCmd;
use super::platform::{AudioKeys, Platform, SessionCtx};
use super::{ACTIVE_PEER_ACCOUNT, Inner};

const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TICK: Duration = Duration::from_secs(1);
/// Tempo para o `Message::Unpair` sair antes de derrubar a sessão.
const UNPAIR_FLUSH: Duration = Duration::from_millis(150);

pub(crate) enum NetCmd {
    Pair {
        code: String,
        address: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Derruba a sessão (o peer já foi esquecido por `Inner::forget_peer`).
    StopSession {
        notify: bool,
    },
    Discovered(DiscoveredPeer),
    /// Endereço manual ou portas mudaram.
    ConfigChanged,
    /// Acordou, rede mudou ou o usuário pediu: refaz a sessão e a busca.
    Reconnect,
    Shutdown,
}

enum PumpCmd {
    Reconnect,
    Stop { notify: bool },
}

struct Active {
    ctl: UnboundedSender<PumpCmd>,
    target: (IpAddr, u16),
    join: JoinHandle<()>,
}

struct Manager<P: Platform> {
    inner: Arc<Inner<P>>,
    identity: Option<StaticKeypair>,
    active: Option<Active>,
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
        active: None,
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
            _ = tick.tick() => m.maybe_connect().await,
        }
    }
    m.stop.store(true, Ordering::Relaxed);
    m.stop_session(false).await;
}

impl<P: Platform> Manager<P> {
    /// Identidade e peer pareado do Keychain; depois liga a descoberta.
    async fn load(&mut self) {
        let ks = self.inner.platform.key_store();
        let loaded = tokio::task::spawn_blocking(move || load_identity(&*ks)).await;
        match loaded {
            Ok(Ok((identity, peer))) => {
                self.inner.mutate(|s| s.peer = peer);
                self.searching_since = Instant::now();
                self.start_discovery(device_id_of(&identity.public));
                self.identity = Some(identity);
            }
            Ok(Err(e)) => tracing::error!("Keychain indisponível: {e}"),
            Err(e) => tracing::error!("falha ao ler o Keychain: {e}"),
        }
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
            NetCmd::StopSession { notify } => {
                self.stop_session(notify).await;
                self.searching_since = Instant::now();
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
                if let Some(a) = &self.active {
                    let _ = a.ctl.send(PumpCmd::Reconnect);
                }
            }
            NetCmd::Shutdown => {}
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
        if let Some(old) = old.filter(|o| o.device_id != peer.device_id) {
            let _ = relay_core::crypto::remove_peer(&*ks, &old.device_id);
        }
        self.hint = Some((addr.ip(), addr.port()));
        // Já sabemos onde ele está: não espera o mDNS.
        self.searching_since = Instant::now()
            .checked_sub(self.inner.opts.manual_grace)
            .unwrap_or_else(Instant::now);
        self.inner.mutate(|s| {
            s.peer = Some(peer);
            s.session = None;
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
        let (peer, pairing) = {
            let st = self.inner.st();
            (st.peer.clone(), st.pairing)
        };
        let Some(peer) = peer.filter(|_| !pairing && self.identity.is_some()) else {
            return;
        };
        let Some(target) = self.pick_target(&peer).await else {
            return;
        };
        match &self.active {
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
        let mut dial = DialerConfig::new(
            identity.clone(),
            peer.clone(),
            target.0,
            remote_ports.clone(),
            local_ports.clone(),
        );
        dial.bind_ip = self.inner.opts.bind_ip;
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
        self.epoch += 1;
        let epoch = self.epoch;
        let media = MediaLink {
            identity: identity.clone(),
            peer: peer.clone(),
            peer_ip: target.0,
            remote_ports: remote_ports.clone(),
            local_ports: local_ports.clone(),
            bind_ip: self.inner.opts.bind_ip,
        };
        let (out_tx, out_rx) = unbounded_channel();
        let (ctl_tx, ctl_rx) = unbounded_channel();
        self.inner.mutate(|s| {
            s.session_gen = epoch;
            s.session = Some(ConnectionStatus::Connecting);
        });
        let to_pump = out_tx.clone();
        self.inner.core(CoreCmd::SessionUp { fast, out: out_tx });
        // A `Session` não é `Sync`: a bomba (que a empresta entre awaits) não pode ir para o
        // pool de tarefas; roda numa thread bloqueante dirigindo o futuro no runtime.
        let (inner, rt) = (self.inner.clone(), tokio::runtime::Handle::current());
        let join = tokio::task::spawn_blocking(move || {
            rt.block_on(pump(inner, dialer, (out_rx, to_pump), ctl_rx, epoch, media));
        });
        self.active = Some(Active {
            ctl: ctl_tx,
            target,
            join,
        });
    }

    async fn stop_session(&mut self, notify: bool) {
        if let Some(a) = self.active.take() {
            let _ = a.ctl.send(PumpCmd::Stop { notify });
            let _ = timeout(Duration::from_secs(3), a.join).await;
        }
        self.inner.mutate(|s| {
            s.session_gen += 1;
            s.session = None;
        });
        self.inner.core(CoreCmd::SessionDown);
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

/// Dona da `Session`: leva mensagens entre ela e a thread do core, e publica status e latência
/// (≤ 1 Hz).
/// O que a mídia precisa saber da sessão além do que a `Session` expõe.
struct MediaLink {
    identity: StaticKeypair,
    peer: PairedPeer,
    peer_ip: IpAddr,
    remote_ports: Ports,
    local_ports: Ports,
    bind_ip: IpAddr,
}

async fn pump<P: Platform>(
    inner: Arc<Inner<P>>,
    mut dialer: SessionDialer,
    (mut out_rx, out_tx): (UnboundedReceiver<Message>, UnboundedSender<Message>),
    mut ctl_rx: UnboundedReceiver<PumpCmd>,
    epoch: u64,
    media: MediaLink,
) {
    let mut status = dialer.status();
    let stats = dialer.stats();
    let mut last_up = false;
    let mut lat = interval(Duration::from_secs(1));
    let publish = |s: ConnectionStatus, last_up: &mut bool| {
        let current = inner.mutate(|st| {
            let ok = st.session_gen == epoch;
            if ok {
                st.session = Some(s);
            }
            ok
        });
        if !current {
            return;
        }
        let up = s == ConnectionStatus::Connected;
        if up != *last_up {
            *last_up = up;
            inner.core(CoreCmd::Link(up));
        }
    };
    let announce = |dialer: &SessionDialer, s: ConnectionStatus, last_up: bool| {
        if s != ConnectionStatus::Connected || last_up || inner.st().session_gen != epoch {
            return;
        }
        let audio_keys = dialer.audio_link().map(|l| AudioKeys {
            pc_to_mac: l.pc_to_mac,
            mac_to_pc: l.mac_to_pc,
        });
        inner.core(CoreCmd::MediaUp(Box::new(SessionCtx {
            send: out_tx.clone(),
            peer_ip: media.peer_ip,
            remote_ports: media.remote_ports.clone(),
            local_ports: media.local_ports.clone(),
            bind_ip: media.bind_ip,
            identity: media.identity.clone(),
            peer: media.peer.clone(),
            audio_keys,
        })));
    };
    let first = *status.borrow_and_update();
    announce(&dialer, first, last_up);
    publish(first, &mut last_up);
    let mut notify = false;
    loop {
        tokio::select! {
            ev = dialer.recv_event() => match ev {
                None => break,
                Some(SessionEvent::Message(m)) => inner.core(CoreCmd::Peer(m)),
                Some(_) => {}
            },
            Some(m) = out_rx.recv() => {
                let _ = dialer.send(m).await;
            }
            r = status.changed() => {
                if r.is_err() { break; }
                let s = *status.borrow_and_update();
                announce(&dialer, s, last_up);
                publish(s, &mut last_up);
            }
            _ = lat.tick() => {
                if last_up {
                    let l = *stats.borrow();
                    inner.mutate(|s| s.latency = l);
                    inner.events.latency(&l);
                }
            }
            cmd = ctl_rx.recv() => match cmd {
                Some(PumpCmd::Reconnect) => dialer.force_reconnect(),
                Some(PumpCmd::Stop { notify: n }) => { notify = n; break; }
                None => break,
            },
        }
    }
    if notify {
        let _ = dialer.send(Message::Unpair).await;
        sleep(UNPAIR_FLUSH).await;
    }
    dialer.shutdown().await;
}
