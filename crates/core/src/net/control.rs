//! Canal de controle: TCP_NODELAY + Noise, primeiro frame Pair/Session, Hello com versão,
//! heartbeat, detecção de silêncio, reconexão com backoff e estatísticas.

use std::collections::VecDeque;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};

use super::fast::FastShared;
use super::stats::{Backoff, RttTracker, now_us};
use super::{DialerConfig, NetError, SessionEvent};
use crate::crypto::{
    CryptoError, LocalDevice, NoiseSession, StaticKeypair, UdpChannel, noise_accept, noise_dial,
    pair, read_frame, write_frame,
};
use crate::protocol::{FrameReader, FrameWriter};
use crate::types::consts::HEARTBEAT_MS;
use crate::types::{ConnectionStatus, LatencyStats, Message, PairedPeer, Role};

/// Primeiro frame da conexão (1 byte): o que vem a seguir é uma sessão Noise ou um pareamento.
pub(super) const TAG_SESSION: u8 = 1;
pub(super) const TAG_PAIR: u8 = 2;

/// Sem nenhum frame do par por mais que isso: a sessão cai.
const SILENCE: Duration = Duration::from_millis(2000);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const PAIR_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);

/// Estado lido pelo `Session` (handle público).
pub(super) struct Shared {
    pub status: watch::Sender<ConnectionStatus>,
    pub stats: watch::Sender<LatencyStats>,
    pub force: Notify,
    /// 0 = sem sessão, 1 = TCP_NODELAY desligado, 2 = ligado (lido de volta do socket).
    pub nodelay: AtomicU8,
    pub connections: AtomicU64,
    pub fast: Arc<FastShared>,
    /// Endereço do par e chaves UDP de áudio da sessão atual (WI3/MI3).
    pub audio_link: Mutex<Option<super::AudioLink>>,
}

impl Shared {
    pub(super) fn new(fast: Arc<FastShared>) -> Arc<Self> {
        Arc::new(Self {
            status: watch::channel(ConnectionStatus::Disconnected).0,
            stats: watch::channel(LatencyStats::default()).0,
            force: Notify::new(),
            nodelay: AtomicU8::new(0),
            connections: AtomicU64::new(0),
            fast,
            audio_link: Mutex::new(None),
        })
    }

    fn set_status(&self, new: ConnectionStatus) {
        self.status.send_if_modified(|s| {
            if *s == new {
                false
            } else {
                *s = new;
                true
            }
        });
    }
}

/// O que cada lado traz para o loop de sessão.
pub(super) struct Ctx {
    pub shared: Arc<Shared>,
    pub events: mpsc::Sender<SessionEvent>,
    pub outbound: mpsc::Receiver<Message>,
    pub shutdown: watch::Receiver<bool>,
    pub role: Role,
    pub device_id: String,
    pub app_version: String,
    pub protocol_version: u16,
    /// Endereço UDP do par (Mac); `None` no agente, que o aprende pelo primeiro pacote.
    pub fast_dest: Option<SocketAddr>,
}

impl Ctx {
    fn is_shutdown(&self) -> bool {
        *self.shutdown.borrow()
    }
}

async fn wait_shutdown(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Conexão que já passou pelo handshake Noise.
pub(super) struct Established {
    noise: NoiseSession,
    stream: TcpStream,
}

enum End {
    Lost,
    Forced,
    Shutdown,
    Incompatible,
    Replaced(Box<Established>),
}

fn io_err(e: std::io::Error) -> NetError {
    NetError::Io(e.to_string())
}

fn crypto_err(e: CryptoError) -> NetError {
    match e {
        CryptoError::UnknownPeer => NetError::UnknownPeer,
        other => NetError::Crypto(other.to_string()),
    }
}

/// Buffers reaproveitados entre mensagens.
#[derive(Default)]
struct Scratch {
    frame: Vec<u8>,
    cipher: Vec<u8>,
    wire: Vec<u8>,
    plain: Vec<u8>,
}

/// Um frame do protocolo por mensagem Noise; escrito de uma vez só (com TCP_NODELAY, escrever
/// tamanho e corpo separados viraria dois segmentos).
async fn write_msg(
    noise: &mut NoiseSession,
    wr: &mut OwnedWriteHalf,
    msg: &Message,
    sc: &mut Scratch,
) -> Result<(), NetError> {
    sc.frame.clear();
    FrameWriter
        .encode(msg, &mut sc.frame)
        .map_err(|e| NetError::Protocol(e.to_string()))?;
    noise
        .encrypt(&sc.frame, &mut sc.cipher)
        .map_err(crypto_err)?;
    sc.wire.clear();
    sc.wire
        .extend_from_slice(&(sc.cipher.len() as u32).to_be_bytes());
    sc.wire.extend_from_slice(&sc.cipher);
    wr.write_all(&sc.wire).await.map_err(io_err)
}

fn decode_msgs(
    noise: &mut NoiseSession,
    reader: &mut FrameReader,
    cipher: &[u8],
    sc: &mut Scratch,
) -> Result<Vec<Message>, NetError> {
    noise.decrypt(cipher, &mut sc.plain).map_err(crypto_err)?;
    reader
        .push(&sc.plain)
        .map_err(|e| NetError::Protocol(e.to_string()))
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn next_established(rx: &mut Option<&mut mpsc::Receiver<Established>>) -> Established {
    if let Some(rx) = rx.as_deref_mut() {
        if let Some(e) = rx.recv().await {
            return e;
        }
    }
    pending().await
}

/// Roda uma sessão já cifrada até ela acabar. O `bool` diz se chegou a `Connected`.
async fn run_session(
    ctx: &mut Ctx,
    est: Established,
    mut replace: Option<&mut mpsc::Receiver<Established>>,
) -> (End, bool) {
    let Established { mut noise, stream } = est;
    let shared = ctx.shared.clone();
    let mut shutdown = ctx.shutdown.clone();
    shared.nodelay.store(
        match stream.nodelay() {
            Ok(true) => 2,
            _ => 1,
        },
        Ordering::Relaxed,
    );
    let peer = noise.peer.clone();
    let remote_ip = stream.peer_addr().ok().map(|a| a.ip());
    let (mut rd, mut wr) = stream.into_split();
    let (ftx, mut frx) = mpsc::channel::<Vec<u8>>(64);
    let _reader = AbortOnDrop(tokio::spawn(async move {
        while let Ok(frame) = read_frame(&mut rd).await {
            if ftx.send(frame).await.is_err() {
                break;
            }
        }
    }));
    // Mensagens anteriores a esta sessão não valem mais.
    while ctx.outbound.try_recv().is_ok() {}
    let mut sc = Scratch::default();
    let mut reader = FrameReader::default();
    let mut pending_msgs: VecDeque<Message> = VecDeque::new();

    // Hello dos dois lados: cada um envia o seu e depois lê o do outro.
    let hello = Message::Hello {
        protocol_version: ctx.protocol_version,
        device_id: ctx.device_id.clone(),
        role: ctx.role,
        app_version: ctx.app_version.clone(),
    };
    if write_msg(&mut noise, &mut wr, &hello, &mut sc)
        .await
        .is_err()
    {
        return (End::Lost, false);
    }
    let remote_hello = loop {
        let frame = tokio::select! {
            f = timeout(HELLO_TIMEOUT, frx.recv()) => match f {
                Ok(Some(f)) => f,
                _ => return (End::Lost, false),
            },
            () = wait_shutdown(&mut shutdown) => return (End::Shutdown, false),
            () = shared.force.notified() => return (End::Forced, false),
        };
        match decode_msgs(&mut noise, &mut reader, &frame, &mut sc) {
            Ok(msgs) => {
                pending_msgs.extend(msgs);
                if let Some(first) = pending_msgs.pop_front() {
                    break first;
                }
            }
            Err(_) => return (End::Lost, false),
        }
    };
    let Message::Hello {
        protocol_version: remote_version,
        device_id: remote_id,
        ..
    } = remote_hello
    else {
        return (End::Lost, false);
    };
    if remote_version != ctx.protocol_version {
        shared.set_status(ConnectionStatus::IncompatibleVersion {
            local: ctx.protocol_version,
            remote: remote_version,
        });
        // O Hello já foi enviado: fecha com FIN e deixa o outro lado ler o nosso antes de cair.
        let _ = wr.shutdown().await;
        let _ = timeout(Duration::from_millis(300), async {
            while frx.recv().await.is_some() {}
        })
        .await;
        return (End::Incompatible, false);
    }
    if remote_id != peer.device_id {
        return (End::Lost, false);
    }

    // Sessão de pé: liga o canal rápido com as chaves deste handshake.
    shared
        .fast
        .install(noise.udp_keys(UdpChannel::Input), ctx.fast_dest);
    *shared.audio_link.lock().unwrap_or_else(|e| e.into_inner()) =
        remote_ip.map(|remote_ip| super::AudioLink {
            remote_ip,
            pc_to_mac: noise.udp_keys(UdpChannel::AudioPcToMac),
            mac_to_pc: noise.udp_keys(UdpChannel::AudioMacToPc),
        });
    shared.connections.fetch_add(1, Ordering::Relaxed);
    shared.stats.send_replace(LatencyStats::default());
    shared.set_status(ConnectionStatus::Connected);
    let _ = ctx.events.send(SessionEvent::Connected(peer)).await;
    if ctx.role == Role::Dialer {
        shared.fast.probe();
    }

    let mut rtt = RttTracker::default();
    let mut tick = interval(Duration::from_millis(HEARTBEAT_MS));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_rx = Instant::now();
    let mut ping_id = 0u32;

    let end = 'session: loop {
        let mut inbound: Vec<Message> = pending_msgs.drain(..).collect();
        tokio::select! {
            () = wait_shutdown(&mut shutdown) => {
                let _ = write_msg(&mut noise, &mut wr, &Message::Bye, &mut sc).await;
                break End::Shutdown;
            }
            () = shared.force.notified() => {
                let _ = write_msg(&mut noise, &mut wr, &Message::Bye, &mut sc).await;
                break End::Forced;
            }
            new = next_established(&mut replace) => break End::Replaced(Box::new(new)),
            frame = frx.recv() => {
                let Some(frame) = frame else { break End::Lost };
                last_rx = Instant::now();
                match decode_msgs(&mut noise, &mut reader, &frame, &mut sc) {
                    Ok(msgs) => inbound.extend(msgs),
                    Err(_) => break End::Lost,
                }
            }
            out = ctx.outbound.recv() => {
                let Some(msg) = out else { break End::Shutdown };
                if write_msg(&mut noise, &mut wr, &msg, &mut sc).await.is_err() {
                    break End::Lost;
                }
            }
            _ = tick.tick() => {
                if last_rx.elapsed() > SILENCE {
                    break End::Lost;
                }
                ping_id = ping_id.wrapping_add(1);
                let ping = Message::Ping { id: ping_id, ts_us: now_us() };
                if write_msg(&mut noise, &mut wr, &ping, &mut sc).await.is_err() {
                    break End::Lost;
                }
                if ctx.role == Role::Dialer {
                    shared.fast.probe();
                }
                let fast = shared.fast.stats().map(|s| (s.jitter_us, s.loss_pct));
                shared.stats.send_replace(rtt.snapshot(fast));
            }
        }
        for msg in inbound {
            match msg {
                Message::Ping { id, ts_us } => {
                    let pong = Message::Pong { id, ts_us };
                    if write_msg(&mut noise, &mut wr, &pong, &mut sc)
                        .await
                        .is_err()
                    {
                        break 'session End::Lost;
                    }
                }
                Message::Pong { ts_us, .. } => {
                    rtt.sample(now_us().saturating_sub(ts_us));
                    let fast = shared.fast.stats().map(|s| (s.jitter_us, s.loss_pct));
                    shared.stats.send_replace(rtt.snapshot(fast));
                }
                Message::Hello { .. } | Message::HelloAck => {}
                Message::Bye => break 'session End::Lost,
                other => {
                    if ctx.events.send(SessionEvent::Message(other)).await.is_err() {
                        break 'session End::Shutdown;
                    }
                }
            }
        }
    };
    shared.fast.clear();
    shared.nodelay.store(0, Ordering::Relaxed);
    (end, true)
}

// ---------------------------------------------------------------- dialer

pub(super) struct DialerState {
    pub cfg: DialerConfig,
    pub identity: StaticKeypair,
}

async fn connect(cfg: &DialerConfig, identity: &StaticKeypair) -> Result<Established, NetError> {
    let addr = SocketAddr::new(cfg.remote_ip, cfg.remote_ports.control);
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(io_err)?;
    stream.set_nodelay(true).map_err(io_err)?;
    let noise = timeout(HANDSHAKE_TIMEOUT, async {
        write_frame(&mut stream, &[TAG_SESSION])
            .await
            .map_err(io_err)?;
        noise_dial(&mut stream, identity, &cfg.peer)
            .await
            .map_err(crypto_err)
    })
    .await
    .map_err(|_| NetError::Timeout)??;
    Ok(Established { noise, stream })
}

pub(super) async fn dialer_main(mut ctx: Ctx, st: DialerState) {
    let shared = ctx.shared.clone();
    let mut shutdown = ctx.shutdown.clone();
    let mut backoff = Backoff::new();
    let mut ever_connected = false;
    while !ctx.is_shutdown() {
        shared.set_status(if ever_connected {
            ConnectionStatus::Reconnecting
        } else {
            ConnectionStatus::Connecting
        });
        let attempt = tokio::select! {
            () = wait_shutdown(&mut shutdown) => break,
            r = connect(&st.cfg, &st.identity) => r,
        };
        let (end, up) = match attempt {
            Ok(est) => run_session(&mut ctx, est, None).await,
            Err(e) => {
                tracing::debug!("conexão ao agente falhou: {e}");
                (End::Lost, false)
            }
        };
        if up {
            ever_connected = true;
            backoff.reset();
        }
        match end {
            End::Shutdown => break,
            End::Forced => backoff.reset(),
            End::Incompatible => {
                // Não adianta insistir: só uma ação do usuário (ou o fim da divergência) refaz.
                tokio::select! {
                    () = wait_shutdown(&mut shutdown) => break,
                    () = shared.force.notified() => {}
                }
                backoff.reset();
            }
            End::Lost | End::Replaced(_) => {
                if ever_connected {
                    shared.set_status(ConnectionStatus::Reconnecting);
                }
                tokio::select! {
                    () = wait_shutdown(&mut shutdown) => break,
                    () = shared.force.notified() => backoff.reset(),
                    () = sleep(backoff.next_delay()) => {}
                }
            }
        }
    }
    shared.set_status(ConnectionStatus::Disconnected);
}

/// Pareamento pelo lado do Mac: uma conexão, uma tentativa.
pub(super) async fn dial_pair(
    addr: SocketAddr,
    code: &str,
    me: &LocalDevice,
) -> Result<PairedPeer, NetError> {
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(io_err)?;
    stream.set_nodelay(true).map_err(io_err)?;
    timeout(PAIR_TIMEOUT, async {
        write_frame(&mut stream, &[TAG_PAIR])
            .await
            .map_err(io_err)?;
        pair(&mut stream, Role::Dialer, code, me)
            .await
            .map_err(|e| NetError::Pairing(e.to_string()))
    })
    .await
    .map_err(|_| NetError::Timeout)?
}

// -------------------------------------------------------------- listener

/// Código de pareamento aberto no agente: uso único, com validade.
pub(super) struct PairingOffer {
    pub code: String,
    pub expires: Instant,
}

pub(super) struct ListenerState {
    pub identity: StaticKeypair,
    pub device: LocalDevice,
    pub peers: Arc<Mutex<Vec<PairedPeer>>>,
    pub offer: Arc<Mutex<Option<PairingOffer>>>,
    pub events: mpsc::Sender<SessionEvent>,
}

impl ListenerState {
    fn take_code(&self) -> Option<String> {
        let offer = self
            .offer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        (Instant::now() < offer.expires).then_some(offer.code)
    }

    async fn emit(&self, ev: SessionEvent) {
        let _ = self.events.send(ev).await;
    }
}

async fn handle_incoming(
    mut stream: TcpStream,
    st: Arc<ListenerState>,
    est_tx: mpsc::Sender<Established>,
) {
    let tag = match timeout(HANDSHAKE_TIMEOUT, read_frame(&mut stream)).await {
        Ok(Ok(f)) if f.len() == 1 => f[0],
        _ => return,
    };
    match tag {
        TAG_SESSION => {
            let peers = st.peers.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match timeout(
                HANDSHAKE_TIMEOUT,
                noise_accept(&mut stream, &st.identity, &peers),
            )
            .await
            {
                Ok(Ok(noise)) => {
                    let _ = est_tx.send(Established { noise, stream }).await;
                }
                Ok(Err(CryptoError::UnknownPeer)) => st.emit(SessionEvent::Refused).await,
                _ => {}
            }
        }
        TAG_PAIR => {
            let Some(code) = st.take_code() else {
                st.emit(SessionEvent::PairFailed(
                    "nenhum código de pareamento aberto".into(),
                ))
                .await;
                return;
            };
            match timeout(
                PAIR_TIMEOUT,
                pair(&mut stream, Role::Listener, &code, &st.device),
            )
            .await
            {
                Ok(Ok(peer)) => {
                    {
                        let mut peers = st.peers.lock().unwrap_or_else(|e| e.into_inner());
                        peers.retain(|p| p.device_id != peer.device_id);
                        peers.push(peer.clone());
                    }
                    st.emit(SessionEvent::Paired(peer)).await;
                }
                Ok(Err(e)) => st.emit(SessionEvent::PairFailed(e.to_string())).await,
                Err(_) => st.emit(SessionEvent::PairFailed("timeout".into())).await,
            }
        }
        _ => {}
    }
}

pub(super) async fn accept_loop(
    listener: TcpListener,
    st: Arc<ListenerState>,
    est_tx: mpsc::Sender<Established>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let accepted = tokio::select! {
            () = wait_shutdown(&mut shutdown) => return,
            r = listener.accept() => r,
        };
        let Ok((stream, _)) = accepted else {
            sleep(Duration::from_millis(50)).await;
            continue;
        };
        if stream.set_nodelay(true).is_err() {
            continue;
        }
        tokio::spawn(handle_incoming(stream, st.clone(), est_tx.clone()));
    }
}

pub(super) async fn listener_main(mut ctx: Ctx, mut est_rx: mpsc::Receiver<Established>) {
    let shared = ctx.shared.clone();
    let mut shutdown = ctx.shutdown.clone();
    let mut next: Option<Established> = None;
    loop {
        let est = match next.take() {
            Some(e) => e,
            None => tokio::select! {
                () = wait_shutdown(&mut shutdown) => break,
                e = est_rx.recv() => match e { Some(e) => e, None => break },
            },
        };
        let (end, _) = run_session(&mut ctx, est, Some(&mut est_rx)).await;
        match end {
            End::Shutdown => break,
            End::Replaced(e) => next = Some(*e),
            End::Incompatible => {}
            End::Lost | End::Forced => shared.set_status(ConnectionStatus::Reconnecting),
        }
    }
    shared.set_status(ConnectionStatus::Disconnected);
}
