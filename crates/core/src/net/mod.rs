//! dono: C3
//! Sessão de rede: canal de controle TCP cifrado (Noise), canal rápido UDP em threads dedicadas
//! (`fast`, sem tokio), heartbeat, reconexão e estatísticas.
//!
//! Dois papéis, um handle:
//! - `SessionListener` (agente): escuta, aceita só peers pareados e atende pareamentos com o
//!   código aberto por `open_pairing`.
//! - `SessionDialer` (Mac): `pair` (uma tentativa) e `start` (sessão com reconexão automática).
//!
//! Os dois devolvem um `Session`: `status()`/`stats()` (watch), `send`/`recv_event` (canal
//! confiável), `force_reconnect`, `take_input_producer` (fila do canal rápido) e `shutdown`.
//! Na sessão atual, MouseMove/Scroll vão pelo ring + UDP; o resto, por `send` (TCP).
//!
//! O primeiro frame de toda conexão TCP é 1 byte: 1 = sessão Noise KK, 2 = pareamento SPAKE2.
//! Depois do Noise, os dois lados trocam `Hello` com `PROTOCOL_VERSION`; versões diferentes
//! terminam em `ConnectionStatus::IncompatibleVersion` e a sessão só é refeita por
//! `force_reconnect`.

mod control;
mod fast;
mod stats;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtrb::Producer;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::crypto::{LocalDevice, StaticKeypair, device_id_of};
use crate::types::consts::PROTOCOL_VERSION;
use crate::types::{ConnectionStatus, InputEvent, LatencyStats, Message, PairedPeer, Ports, Role};

pub use fast::{FastEvent, FastStats};
pub use stats::{Backoff, Jitter, RttTracker, now_us};

use control::{Ctx, DialerState, ListenerState, PairingOffer, Shared};
use fast::FastChannel;

/// Capacidade do ring de saída do canal rápido.
pub const FAST_RING_CAPACITY: usize = 4096;
/// Validade do código de pareamento (D5).
pub const PAIRING_CODE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("peer não pareado")]
    UnknownPeer,
    #[error("{0}")]
    Io(String),
    #[error("protocolo: {0}")]
    Protocol(String),
    #[error("cripto: {0}")]
    Crypto(String),
    #[error("pareamento: {0}")]
    Pairing(String),
    #[error("tempo esgotado")]
    Timeout,
    #[error("sem sessão ativa")]
    NotConnected,
}

/// O que a sessão entrega ao dono.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// Sessão estabelecida (Hello conferido) com este peer.
    Connected(PairedPeer),
    /// Mensagem do canal confiável (Ping/Pong/Hello/Bye são tratadas aqui dentro).
    Message(Message),
    /// Agente: um pareamento terminou; quem chama persiste com `crypto::save_peer`.
    Paired(PairedPeer),
    PairFailed(String),
    /// Agente: conexão de peer não pareado recusada.
    Refused,
}

/// Configuração do agente. Portas 0 pedem porta efêmera (testes); `local_*_addr` diz qual veio.
#[derive(Clone)]
pub struct ListenerConfig {
    pub identity: StaticKeypair,
    pub device: LocalDevice,
    pub peers: Vec<PairedPeer>,
    pub bind_ip: IpAddr,
    /// Usa `control` (TCP) e `input` (UDP).
    pub ports: Ports,
    pub app_version: String,
    pub protocol_version: u16,
}

/// Configuração do Mac.
#[derive(Clone)]
pub struct DialerConfig {
    pub identity: StaticKeypair,
    pub peer: PairedPeer,
    pub remote_ip: IpAddr,
    /// Portas do agente: `control` (TCP) e `input` (UDP).
    pub remote_ports: Ports,
    pub bind_ip: IpAddr,
    /// Porta local do UDP de input (`input`).
    pub local_ports: Ports,
    pub app_version: String,
    pub protocol_version: u16,
}

impl ListenerConfig {
    pub fn new(
        identity: StaticKeypair,
        device: LocalDevice,
        peers: Vec<PairedPeer>,
        ports: Ports,
    ) -> Self {
        Self {
            identity,
            device,
            peers,
            bind_ip: IpAddr::from([0, 0, 0, 0]),
            ports,
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

impl DialerConfig {
    pub fn new(
        identity: StaticKeypair,
        peer: PairedPeer,
        remote_ip: IpAddr,
        remote_ports: Ports,
        local_ports: Ports,
    ) -> Self {
        Self {
            identity,
            peer,
            remote_ip,
            remote_ports,
            bind_ip: IpAddr::from([0, 0, 0, 0]),
            local_ports,
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

/// Par e chaves UDP dos canais de áudio da sessão atual (derivadas do mesmo handshake Noise do
/// canal de input, separadas por canal e direção). `pc_to_mac`/`mac_to_pc` já vêm orientadas
/// para quem chama (`send`/`recv`).
#[derive(Debug, Clone)]
pub struct AudioLink {
    pub remote_ip: IpAddr,
    pub pc_to_mac: crate::crypto::UdpKeys,
    pub mac_to_pc: crate::crypto::UdpKeys,
}

/// Handle de uma sessão (dialer ou listener).
pub struct Session {
    shared: Arc<Shared>,
    out_tx: mpsc::Sender<Message>,
    events: mpsc::Receiver<SessionEvent>,
    shutdown_tx: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    fast: Option<FastChannel>,
    producer: Option<Producer<InputEvent>>,
}

impl Session {
    pub fn status(&self) -> watch::Receiver<ConnectionStatus> {
        self.shared.status.subscribe()
    }

    pub fn stats(&self) -> watch::Receiver<LatencyStats> {
        self.shared.stats.subscribe()
    }

    /// IP do par e chaves de áudio da sessão atual (`None` fora de `Connected`).
    pub fn audio_link(&self) -> Option<AudioLink> {
        if *self.shared.status.borrow() != ConnectionStatus::Connected {
            return None;
        }
        self.shared
            .audio_link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Sessões estabelecidas desde o início (sobe a cada reconexão).
    pub fn connections(&self) -> u64 {
        self.shared.connections.load(Ordering::Relaxed)
    }

    /// TCP_NODELAY da conexão atual, lido de volta do socket (`None` sem sessão).
    pub fn tcp_nodelay(&self) -> Option<bool> {
        match self.shared.nodelay.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n == 2),
        }
    }

    /// Endereço do socket UDP do canal rápido.
    pub fn fast_local_addr(&self) -> Option<SocketAddr> {
        self.fast.as_ref().map(|f| f.local_addr)
    }

    /// Jitter e perda do canal rápido na sessão atual (`None` se ainda não chegou pacote).
    pub fn fast_stats(&self) -> Option<FastStats> {
        self.shared.fast.stats()
    }

    /// (eventos não suportados no UDP descartados, pacotes sem sessão/destino descartados).
    pub fn fast_dropped(&self) -> (u64, u64) {
        self.shared.fast.dropped()
    }

    /// Fila do canal rápido (uma vez). A thread de envio lê dela; MouseMove e Scroll viajam por
    /// UDP, qualquer outro evento é descartado (e contado) — esses vão por `send`.
    pub fn take_input_producer(&mut self) -> Option<Producer<InputEvent>> {
        self.producer.take()
    }

    /// Envia pelo canal confiável. Sem sessão ativa devolve `NotConnected`.
    pub async fn send(&self, msg: Message) -> Result<(), NetError> {
        if *self.shared.status.borrow() != ConnectionStatus::Connected {
            return Err(NetError::NotConnected);
        }
        self.out_tx
            .send(msg)
            .await
            .map_err(|_| NetError::NotConnected)
    }

    /// Próximo evento (mensagem recebida, pareamento, recusa). `None` depois do `shutdown`.
    pub async fn recv_event(&mut self) -> Option<SessionEvent> {
        self.events.recv().await
    }

    /// Derruba a sessão atual e refaz.
    pub fn force_reconnect(&self) {
        self.shared.force.notify_one();
    }

    /// Encerra tudo e libera as portas.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        for t in self.tasks.drain(..) {
            let _ = t.await;
        }
        if let Some(fast) = self.fast.take() {
            let _ = tokio::task::spawn_blocking(move || fast.stop()).await;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        for t in &self.tasks {
            t.abort();
        }
        if let Some(fast) = &self.fast {
            fast.signal_stop();
        }
    }
}

fn start_fast(
    bind_ip: IpAddr,
    port: u16,
    on_fast: Box<dyn FnMut(FastEvent) + Send>,
) -> Result<(FastChannel, Producer<InputEvent>), NetError> {
    FastChannel::start(SocketAddr::new(bind_ip, port), FAST_RING_CAPACITY, on_fast)
        .map_err(|e| NetError::Io(e.to_string()))
}

struct Parts {
    ctx: Ctx,
    session: Session,
}

fn build(
    fast: FastChannel,
    producer: Producer<InputEvent>,
    role: Role,
    device_id: String,
    app_version: String,
    protocol_version: u16,
    fast_dest: Option<SocketAddr>,
) -> (Parts, mpsc::Sender<SessionEvent>) {
    let shared = Shared::new(fast.shared.clone());
    let (events_tx, events_rx) = mpsc::channel(1024);
    let (out_tx, out_rx) = mpsc::channel(1024);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let ctx = Ctx {
        shared: shared.clone(),
        events: events_tx.clone(),
        outbound: out_rx,
        shutdown: shutdown_rx,
        role,
        device_id,
        app_version,
        protocol_version,
        fast_dest,
    };
    let session = Session {
        shared,
        out_tx,
        events: events_rx,
        shutdown_tx,
        tasks: Vec::new(),
        fast: Some(fast),
        producer: Some(producer),
    };
    (Parts { ctx, session }, events_tx)
}

/// Lado do agente: escuta e aceita só peers pareados.
pub struct SessionListener {
    session: Session,
    control_addr: SocketAddr,
    peers: Arc<Mutex<Vec<PairedPeer>>>,
    offer: Arc<Mutex<Option<PairingOffer>>>,
}

impl SessionListener {
    /// Abre o TCP de controle e o UDP de input e começa a aceitar. `on_fast` recebe os eventos do
    /// canal rápido na thread de recepção.
    pub async fn start(
        cfg: ListenerConfig,
        on_fast: Box<dyn FnMut(FastEvent) + Send>,
    ) -> Result<Self, NetError> {
        let tcp = TcpListener::bind(SocketAddr::new(cfg.bind_ip, cfg.ports.control))
            .await
            .map_err(|e| NetError::Io(e.to_string()))?;
        let control_addr = tcp.local_addr().map_err(|e| NetError::Io(e.to_string()))?;
        let (fast, producer) = start_fast(cfg.bind_ip, cfg.ports.input, on_fast)?;
        let device_id = device_id_of(&cfg.identity.public);
        let (parts, events_tx) = build(
            fast,
            producer,
            Role::Listener,
            device_id,
            cfg.app_version.clone(),
            cfg.protocol_version,
            None,
        );
        let Parts { ctx, mut session } = parts;
        let peers = Arc::new(Mutex::new(cfg.peers));
        let offer = Arc::new(Mutex::new(None));
        let state = Arc::new(ListenerState {
            identity: cfg.identity,
            device: cfg.device,
            peers: peers.clone(),
            offer: offer.clone(),
            events: events_tx,
        });
        let (est_tx, est_rx) = mpsc::channel(4);
        session.tasks.push(tokio::spawn(control::accept_loop(
            tcp,
            state,
            est_tx,
            session.shutdown_tx.subscribe(),
        )));
        session
            .tasks
            .push(tokio::spawn(control::listener_main(ctx, est_rx)));
        Ok(Self {
            session,
            control_addr,
            peers,
            offer,
        })
    }

    pub fn local_control_addr(&self) -> SocketAddr {
        self.control_addr
    }

    /// Endereço do UDP de input (porta efêmera quando `ports.input` era 0).
    pub fn local_input_addr(&self) -> SocketAddr {
        self.session
            .fast_local_addr()
            .expect("canal rápido vive até o shutdown")
    }

    /// Abre um código de pareamento (uso único: só o sucesso o consome; 3 tentativas erradas o invalidam, com 500 ms entre tentativas).
    pub fn open_pairing(&self, code: String, ttl: Duration) {
        *self.offer.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(PairingOffer::new(code, Instant::now() + ttl));
    }

    pub fn add_peer(&self, peer: PairedPeer) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        peers.retain(|p| p.device_id != peer.device_id);
        peers.push(peer);
    }

    pub fn remove_peer(&self, device_id: &str) {
        self.peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|p| p.device_id != device_id);
    }
}

impl std::ops::Deref for SessionListener {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}

impl std::ops::DerefMut for SessionListener {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

/// Lado do Mac: disca para o agente.
pub struct SessionDialer {
    session: Session,
}

impl SessionDialer {
    /// Pareamento: uma conexão, uma tentativa com o código de 6 dígitos. Quem chama persiste o
    /// peer devolvido com `crypto::save_peer`.
    pub async fn pair(
        addr: SocketAddr,
        code: &str,
        me: &LocalDevice,
    ) -> Result<PairedPeer, NetError> {
        control::dial_pair(addr, code, me).await
    }

    /// Começa a discar (e a reconectar sozinho, com backoff de 250 ms a 5 s).
    pub async fn start(
        cfg: DialerConfig,
        on_fast: Box<dyn FnMut(FastEvent) + Send>,
    ) -> Result<Self, NetError> {
        let (fast, producer) = start_fast(cfg.bind_ip, cfg.local_ports.input, on_fast)?;
        let dest = SocketAddr::new(cfg.remote_ip, cfg.remote_ports.input);
        let (parts, _events_tx) = build(
            fast,
            producer,
            Role::Dialer,
            device_id_of(&cfg.identity.public),
            cfg.app_version.clone(),
            cfg.protocol_version,
            Some(dest),
        );
        let Parts { ctx, mut session } = parts;
        let identity = cfg.identity.clone();
        session.tasks.push(tokio::spawn(control::dialer_main(
            ctx,
            DialerState { cfg, identity },
        )));
        Ok(Self { session })
    }
}

impl std::ops::Deref for SessionDialer {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}

impl std::ops::DerefMut for SessionDialer {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl SessionListener {
    pub async fn shutdown(self) {
        self.session.shutdown().await;
    }
}

impl SessionDialer {
    pub async fn shutdown(self) {
        self.session.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
