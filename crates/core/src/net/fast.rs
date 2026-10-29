//! Canal rápido UDP em threads dedicadas: uma lê o `rtrb` e envia, outra recebe e entrega por
//! callback. Sem runtime assíncrono nem alocação por pacote (D6, D7).
//!
//! Pacote no fio: cabeçalho de 17 bytes (AAD) + corpo cifrado + tag de 16 bytes. Um pacote de
//! `kind` 0 só com a tag é um "probe": autentica o remetente (o agente fixa o endereço do Mac
//! por ele) e mantém o mapeamento do firewall aberto, sem virar evento. `kind` 0x10 é um pedido de
//! eco e 0x11 a resposta, com o `ts_us` do pedido: mede o RTT no mesmo caminho do mouse. Versões
//! que não conhecem esses kinds autenticam e descartam (`UnknownKind`).

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle, Thread};
use std::time::Duration;

use rtrb::{Consumer, Producer, PushError, RingBuffer};
use socket2::{Domain, Protocol, Socket, Type};

use super::stats::{Jitter, now_us};
use crate::crypto::{ReplayMode, UdpAead, UdpKeys};
use crate::protocol::{FAST_HEADER_LEN, FAST_PACKET_MAX, FastBody, FastPacket};
use crate::types::InputEvent;

const TAG_LEN: usize = 16;
const KIND_PROBE: u8 = 0;
const KIND_ECHO: u8 = 0x10;
const KIND_ECHO_REPLY: u8 = 0x11;
const WIRE_MAX: usize = FAST_PACKET_MAX + TAG_LEN;
/// Buffer de recepção do socket: absorve rajadas sem perder pacote.
const RCVBUF: usize = 4 << 20;
const READ_TIMEOUT: Duration = Duration::from_millis(50);

/// Evento recebido pelo canal rápido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastEvent {
    pub seq: u64,
    /// Relógio do remetente (só serve para variação, não para comparar com `recv_us`).
    pub ts_us: u64,
    /// `net::now_us()` na chegada.
    pub recv_us: u64,
    pub event: InputEvent,
}

/// Acordador da thread de envio: ela dorme em `park` quando a fila esvazia e o `push` a acorda.
#[derive(Default)]
struct Waker {
    parked: AtomicBool,
    thread: OnceLock<Thread>,
}

impl Waker {
    fn wake(&self) {
        // Pareia com o `fence` do laço de envio: ou ele vê o item, ou nós vemos `parked`.
        fence(Ordering::SeqCst);
        if self.parked.load(Ordering::Relaxed) && self.parked.swap(false, Ordering::Relaxed) {
            if let Some(t) = self.thread.get() {
                t.unpark();
            }
        }
    }
}

/// Lado produtor da fila de saída do canal rápido: empurra o evento e acorda a thread de envio.
pub struct FastSender {
    ring: Producer<InputEvent>,
    waker: Option<Arc<Waker>>,
}

impl FastSender {
    pub fn push(&mut self, ev: InputEvent) -> Result<(), PushError<InputEvent>> {
        self.ring.push(ev)?;
        if let Some(w) = &self.waker {
            w.wake();
        }
        Ok(())
    }
}

/// Produtor solto, sem thread para acordar (testes que inspecionam a fila).
impl From<Producer<InputEvent>> for FastSender {
    fn from(ring: Producer<InputEvent>) -> Self {
        Self { ring, waker: None }
    }
}

/// Jitter (us) e perda (%) do canal rápido na sessão atual.
#[derive(Debug, Clone, Copy)]
pub struct FastStats {
    pub received: u64,
    pub jitter_us: f64,
    pub loss_pct: f32,
    /// RTT suavizado do eco UDP, em us; `None` antes da primeira resposta (par sem eco).
    pub rtt_us: Option<f64>,
}

struct SendLink {
    aead: UdpAead,
    seq: u64,
    dest: Option<SocketAddr>,
}

struct RecvLink {
    aead: UdpAead,
    /// Lado que não conhece o endereço do par (agente): fixa pelo primeiro pacote autêntico.
    latch: bool,
    last_src: Option<SocketAddr>,
    base: Option<u64>,
    highest: u64,
    received: u64,
    jitter: Jitter,
}

/// Estado compartilhado entre as threads e o controle da sessão.
pub(super) struct FastShared {
    socket: UdpSocket,
    send: Mutex<Option<SendLink>>,
    recv: Mutex<Option<RecvLink>>,
    stop: AtomicBool,
    received: AtomicU64,
    expected: AtomicU64,
    jitter_bits: AtomicU64,
    /// RTT do eco (us, EWMA 1/8) em bits de f64; 0 = sem amostra.
    rtt_bits: AtomicU64,
    /// Amostras de RTT do eco desde o último `take_rtt_samples` (para o log).
    rtt_samples: Mutex<Vec<u32>>,
    unsupported: AtomicU64,
    unsent: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl FastShared {
    /// Liga o canal para uma sessão: `dest` é o endereço UDP do par (`None` no agente, que o
    /// aprende pelo primeiro pacote autêntico).
    pub(super) fn install(&self, keys: UdpKeys, dest: Option<SocketAddr>) {
        self.received.store(0, Ordering::Relaxed);
        self.expected.store(0, Ordering::Relaxed);
        self.jitter_bits.store(0f64.to_bits(), Ordering::Relaxed);
        self.rtt_bits.store(0f64.to_bits(), Ordering::Relaxed);
        lock(&self.rtt_samples).clear();
        *lock(&self.send) = Some(SendLink {
            aead: UdpAead::new(keys.clone(), ReplayMode::Strict),
            seq: 0,
            dest,
        });
        *lock(&self.recv) = Some(RecvLink {
            aead: UdpAead::new(keys, ReplayMode::Strict),
            latch: dest.is_none(),
            last_src: None,
            base: None,
            highest: 0,
            received: 0,
            jitter: Jitter::default(),
        });
    }

    pub(super) fn clear(&self) {
        *lock(&self.send) = None;
        *lock(&self.recv) = None;
    }

    /// Envia um probe (só o cabeçalho autenticado).
    pub(super) fn probe(&self) {
        let mut sc = Scratch::new();
        self.transmit(&mut sc, None);
    }

    /// Pede um eco ao par: a resposta vira amostra de RTT do canal rápido.
    pub(super) fn echo(&self) {
        let mut sc = Scratch::new();
        self.transmit_header(&mut sc, KIND_ECHO, now_us());
    }

    /// Amostras de RTT do eco (us) desde a última chamada.
    pub(super) fn take_rtt_samples(&self) -> Vec<u32> {
        std::mem::take(&mut *lock(&self.rtt_samples))
    }

    fn on_echo_reply(&self, ts_us: u64) {
        let rtt = now_us().saturating_sub(ts_us).max(1) as f64;
        let prev = f64::from_bits(self.rtt_bits.load(Ordering::Relaxed));
        let next = if prev == 0.0 {
            rtt
        } else {
            prev + (rtt - prev) / 8.0
        };
        self.rtt_bits.store(next.to_bits(), Ordering::Relaxed);
        let mut samples = lock(&self.rtt_samples);
        if samples.len() < 4096 {
            samples.push(rtt.min(f64::from(u32::MAX)) as u32);
        }
    }

    pub(super) fn stats(&self) -> Option<FastStats> {
        let received = self.received.load(Ordering::Relaxed);
        if received == 0 {
            return None;
        }
        let expected = self.expected.load(Ordering::Relaxed).max(received);
        Some(FastStats {
            received,
            jitter_us: f64::from_bits(self.jitter_bits.load(Ordering::Relaxed)),
            loss_pct: (100.0 * (expected - received) as f64 / expected as f64) as f32,
            rtt_us: Some(f64::from_bits(self.rtt_bits.load(Ordering::Relaxed)))
                .filter(|r| *r > 0.0),
        })
    }

    /// Eventos que não são MouseMove/Scroll no ring (não viajam pelo UDP) e pacotes descartados
    /// por falta de sessão ou de destino.
    pub(super) fn dropped(&self) -> (u64, u64) {
        (
            self.unsupported.load(Ordering::Relaxed),
            self.unsent.load(Ordering::Relaxed),
        )
    }

    fn transmit(&self, sc: &mut Scratch, body: Option<FastBody>) -> bool {
        match body {
            Some(body) => self.send_packet(sc, now_us(), |seq, ts_us, buf| {
                (FastPacket { seq, ts_us, body }).encode(buf).ok()
            }),
            None => self.transmit_header(sc, KIND_PROBE, now_us()),
        }
    }

    /// Pacote só com o cabeçalho autenticado (probe, eco).
    fn transmit_header(&self, sc: &mut Scratch, kind: u8, ts_us: u64) -> bool {
        self.send_packet(sc, ts_us, |seq, ts_us, buf| {
            buf[0] = kind;
            buf[1..9].copy_from_slice(&seq.to_le_bytes());
            buf[9..17].copy_from_slice(&ts_us.to_le_bytes());
            Some(FAST_HEADER_LEN)
        })
    }

    fn send_packet(
        &self,
        sc: &mut Scratch,
        ts_us: u64,
        encode: impl FnOnce(u64, u64, &mut [u8; FAST_PACKET_MAX]) -> Option<usize>,
    ) -> bool {
        let mut guard = lock(&self.send);
        let Some(link) = guard.as_mut() else {
            return false;
        };
        let Some(dest) = link.dest else {
            return false;
        };
        link.seq += 1;
        let seq = link.seq;
        let Some(len) = encode(seq, ts_us, &mut sc.buf) else {
            return false;
        };
        let (header, plain) = sc.buf[..len].split_at(FAST_HEADER_LEN);
        if link.aead.seal(seq, header, plain, &mut sc.sealed).is_err() {
            return false;
        }
        sc.wire.clear();
        sc.wire.extend_from_slice(header);
        sc.wire.extend_from_slice(&sc.sealed);
        self.socket.send_to(&sc.wire, dest).is_ok()
    }
}

struct Scratch {
    buf: [u8; FAST_PACKET_MAX],
    sealed: Vec<u8>,
    wire: Vec<u8>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            buf: [0; FAST_PACKET_MAX],
            sealed: Vec::with_capacity(WIRE_MAX),
            wire: Vec::with_capacity(FAST_HEADER_LEN + WIRE_MAX),
        }
    }
}

fn to_body(ev: InputEvent) -> Option<FastBody> {
    match ev {
        InputEvent::MouseMove { dx, dy } => Some(FastBody::MouseMove { dx, dy }),
        InputEvent::Scroll { dx, dy, phase } => Some(FastBody::Scroll { dx, dy, phase }),
        _ => None,
    }
}

fn to_event(body: FastBody) -> InputEvent {
    match body {
        FastBody::MouseMove { dx, dy } => InputEvent::MouseMove { dx, dy },
        FastBody::Scroll { dx, dy, phase } => InputEvent::Scroll { dx, dy, phase },
    }
}

/// Os dois threads do canal rápido e o socket UDP. Existe do início ao fim do `Session`; as chaves
/// entram e saem a cada sessão por `FastShared::install/clear`.
pub(super) struct FastChannel {
    pub shared: Arc<FastShared>,
    pub local_addr: SocketAddr,
    threads: Vec<JoinHandle<()>>,
}

impl FastChannel {
    /// Abre o socket em `bind` e sobe as threads. `on_event` roda na thread de recepção: precisa
    /// ser rápido. O `Producer` é a fila de saída (só MouseMove e Scroll são enviados).
    pub fn start(
        bind: SocketAddr,
        capacity: usize,
        on_event: Box<dyn FnMut(FastEvent) + Send>,
    ) -> std::io::Result<(Self, FastSender)> {
        let socket = Socket::new(Domain::for_address(bind), Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        let _ = socket.set_recv_buffer_size(RCVBUF);
        super::mark_low_latency((&socket).into());
        socket.bind(&bind.into())?;
        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(READ_TIMEOUT))?;
        let local_addr = socket.local_addr()?;
        let shared = Arc::new(FastShared {
            socket,
            send: Mutex::new(None),
            recv: Mutex::new(None),
            stop: AtomicBool::new(false),
            received: AtomicU64::new(0),
            expected: AtomicU64::new(0),
            jitter_bits: AtomicU64::new(0),
            rtt_bits: AtomicU64::new(0),
            rtt_samples: Mutex::new(Vec::new()),
            unsupported: AtomicU64::new(0),
            unsent: AtomicU64::new(0),
        });
        let (producer, consumer) = RingBuffer::new(capacity.max(2));
        let waker = Arc::new(Waker::default());
        let tx = {
            let shared = shared.clone();
            let waker = waker.clone();
            thread::Builder::new()
                .name("relay-fast-tx".into())
                .spawn(move || sender_loop(&shared, &waker, consumer))?
        };
        let rx = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("relay-fast-rx".into())
                .spawn(move || receiver_loop(&shared, on_event))?
        };
        Ok((
            Self {
                shared,
                local_addr,
                threads: vec![tx, rx],
            },
            FastSender {
                ring: producer,
                waker: Some(waker),
            },
        ))
    }

    /// Pede às threads que parem (sem esperar).
    pub fn signal_stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.threads.first() {
            t.thread().unpark();
        }
    }

    /// Para e espera as threads (o socket só fecha depois disso).
    pub fn stop(mut self) {
        self.signal_stop();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for FastChannel {
    fn drop(&mut self) {
        self.signal_stop();
    }
}

fn sender_loop(shared: &FastShared, waker: &Waker, mut ring: Consumer<InputEvent>) {
    let _ = waker.thread.set(thread::current());
    let mut sc = Scratch::new();
    let mut idle = 0u32;
    while !shared.stop.load(Ordering::Relaxed) {
        match ring.pop() {
            Ok(ev) => {
                idle = 0;
                match to_body(ev) {
                    Some(body) => {
                        if !shared.transmit(&mut sc, Some(body)) {
                            shared.unsent.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        shared.unsupported.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Err(_) => {
                // Gira primeiro (latência de microssegundos), cede a CPU depois e só estaciona
                // quando está parado de verdade; o `push` acorda na hora.
                idle = idle.saturating_add(1);
                if idle < 256 {
                    std::hint::spin_loop();
                } else if idle < 2048 {
                    thread::yield_now();
                } else {
                    waker.parked.store(true, Ordering::Relaxed);
                    fence(Ordering::SeqCst);
                    if ring.is_empty() {
                        thread::park_timeout(READ_TIMEOUT);
                    }
                    waker.parked.store(false, Ordering::Relaxed);
                }
            }
        }
    }
}

fn receiver_loop(shared: &FastShared, mut on_event: Box<dyn FnMut(FastEvent) + Send>) {
    let mut buf = [0u8; 256];
    let mut sc = Scratch::new();
    let mut plain = Vec::with_capacity(FAST_PACKET_MAX);
    while !shared.stop.load(Ordering::Relaxed) {
        let (n, src) = match shared.socket.recv_from(&mut buf) {
            Ok(x) => x,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock
                        | ErrorKind::TimedOut
                        | ErrorKind::Interrupted
                        | ErrorKind::ConnectionReset
                ) =>
            {
                continue;
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        if !(FAST_HEADER_LEN + TAG_LEN..=FAST_HEADER_LEN + WIRE_MAX).contains(&n) {
            continue;
        }
        let kind = buf[0];
        let seq = u64::from_le_bytes(buf[1..9].try_into().expect("8 bytes"));
        let ts_us = u64::from_le_bytes(buf[9..17].try_into().expect("8 bytes"));
        let moved = {
            let mut guard = lock(&shared.recv);
            let Some(link) = guard.as_mut() else {
                continue;
            };
            if link
                .aead
                .open(
                    seq,
                    &buf[..FAST_HEADER_LEN],
                    &buf[FAST_HEADER_LEN..n],
                    &mut plain,
                )
                .is_err()
            {
                continue;
            }
            let recv_us = now_us();
            // A resposta do eco traz o relógio deste lado: não é trânsito de ida.
            if kind != KIND_ECHO_REPLY {
                link.jitter.update(recv_us as f64 - ts_us as f64);
            }
            link.received += 1;
            let base = *link.base.get_or_insert(seq);
            link.highest = link.highest.max(seq);
            shared.received.store(link.received, Ordering::Relaxed);
            shared
                .expected
                .store(link.highest - base + 1, Ordering::Relaxed);
            shared
                .jitter_bits
                .store(link.jitter.value_us().to_bits(), Ordering::Relaxed);
            let moved = link.latch && link.last_src != Some(src);
            if moved {
                link.last_src = Some(src);
            }
            moved
        };
        if moved {
            if let Some(send) = lock(&shared.send).as_mut() {
                send.dest = Some(src);
            }
        }
        match kind {
            KIND_PROBE => continue,
            KIND_ECHO => {
                shared.transmit_header(&mut sc, KIND_ECHO_REPLY, ts_us);
                continue;
            }
            KIND_ECHO_REPLY => {
                shared.on_echo_reply(ts_us);
                continue;
            }
            _ => {}
        }
        let mut pkt = [0u8; FAST_PACKET_MAX];
        let len = FAST_HEADER_LEN + plain.len();
        if len > FAST_PACKET_MAX {
            continue;
        }
        pkt[..FAST_HEADER_LEN].copy_from_slice(&buf[..FAST_HEADER_LEN]);
        pkt[FAST_HEADER_LEN..len].copy_from_slice(&plain);
        if let Ok(p) = FastPacket::decode(&pkt[..len]) {
            on_event(FastEvent {
                seq: p.seq,
                ts_us: p.ts_us,
                recv_us: now_us(),
                event: to_event(p.body),
            });
        }
    }
}
