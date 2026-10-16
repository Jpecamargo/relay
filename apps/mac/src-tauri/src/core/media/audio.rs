//! dono: MI3
//! Áudio (D13): PCM s16le, 48 kHz estéreo, quadros de 5 ms.
//!
//! PC→Mac: UDP `audio_pc_to_mac` → `UdpAead` → `JitterBuffer` → `DriftCompensator` → ring f32 →
//! `AudioPlayback`. Mac→PC: `AudioCapture` → `AudioPacket` → `UdpAead` → UDP `audio_mac_to_pc`.
//!
//! Datagrama (contrato do WI3, os dois sentidos): os 17 primeiros bytes do `AudioPacket`
//! (stream, seq, ts_samples) em claro, que são o AAD; depois o resto do pacote (codec, channels,
//! frames e os 960 bytes de PCM) cifrado, com tag de 16. O nonce usa o `seq`. `stream` = 0 no
//! PC→Mac e 1 no Mac→PC.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use relay_core::audio::{
    AudioPacket, CHANNELS, CODEC_PCM16, DriftCompensator, FRAMES_PER_PACKET, HEADER_LEN,
    JitterBuffer, PACKET_PAYLOAD_BYTES, i16_to_f32, i16_to_pcm_bytes,
};
use relay_core::crypto::{ReplayMode, UdpAead};
use relay_core::platform::{AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, AudioPlayback};
use relay_core::types::{AudioConfig, AudioStats, Message};

use super::Sh;
use crate::core::{CoreEvents, SessionCtx};

const TAG_LEN: usize = 16;
/// Parte do cabeçalho do `AudioPacket` que vai em claro (stream u8, seq u64, ts_samples u64).
const CLEAR_LEN: usize = 17;
/// Codec, canais e frames (u16): o início do trecho cifrado.
const SEALED_HEAD: usize = HEADER_LEN - CLEAR_LEN;
const STREAM_MAC_TO_PC: u8 = 1;
const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// Frames por chamada do compensador (2,5 ms: abaixo do pacote, o alvo do jitter fica livre).
const CHUNK_FRAMES: usize = 120;
/// Quanto o ring de reprodução guarda à frente do dispositivo (em chunks).
const RING_CHUNKS: usize = 4;
const STATS_EVERY: Duration = Duration::from_secs(1);
const CAPTURE_RING: usize = 64;

/// Os dois sentidos de áudio de uma sessão; cada um liga e desliga sozinho.
#[derive(Default)]
pub(crate) struct AudioRun {
    rx: Option<Playback>,
    tx: Option<Capture>,
}

impl AudioRun {
    pub fn rx_addr(&self) -> Option<SocketAddr> {
        self.rx.as_ref().map(|p| p.addr)
    }

    #[cfg(test)]
    pub fn tx_running(&self) -> bool {
        self.tx.is_some()
    }

    pub fn stop(&mut self) {
        self.rx = None;
        self.tx = None;
    }
}

struct Stopper {
    flag: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Stopper {
    fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
        }
    }

    fn spawn(&mut self, name: &str, f: impl FnOnce() + Send + 'static) {
        match thread::Builder::new().name(name.into()).spawn(f) {
            Ok(h) => self.threads.push(h),
            Err(e) => tracing::warn!("thread {name}: {e}"),
        }
    }
}

impl Drop for Stopper {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

struct Playback {
    addr: SocketAddr,
    device: Arc<Mutex<Box<dyn AudioPlayback>>>,
    cur_device: Option<String>,
    // Declarado depois do dispositivo: as threads páram antes de ele cair.
    stopper: Stopper,
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stopper.flag.store(true, Ordering::Relaxed);
        for t in self.stopper.threads.drain(..) {
            let _ = t.join();
        }
        self.device.lock().unwrap_or_else(|e| e.into_inner()).stop();
    }
}

struct Capture {
    device: Box<dyn AudioCapture>,
    cur_device: Option<String>,
    stopper: Stopper,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.device.stop();
        self.stopper.flag.store(true, Ordering::Relaxed);
        for t in self.stopper.threads.drain(..) {
            let _ = t.join();
        }
    }
}

// ---- ganchos ----

/// Pedido do usuário: manda `AudioControl` e liga/desliga os sentidos locais.
pub(super) fn set_streams(sh: &Arc<Sh>, pc_to_mac: bool, mac_to_pc: bool) {
    sh.st().want = (pc_to_mac, mac_to_pc);
    sh.send(Message::AudioControl {
        pc_to_mac,
        mac_to_pc,
    });
    apply(sh);
}

/// O PC respondeu com o estado real dos streams: ajusta os sentidos locais, sem reenviar.
pub(super) fn pc_state(sh: &Arc<Sh>, pc_to_mac: bool, mac_to_pc: bool) {
    sh.st().want = (pc_to_mac, mac_to_pc);
    apply(sh);
}

/// Sessão nova: refaz o que o usuário pediu com as chaves dela.
pub(super) fn session_up(sh: &Arc<Sh>) {
    // Chaves novas: nada do que corria com as velhas serve.
    sh.st().audio.stop();
    let (p, m) = sh.st().want;
    if p || m {
        sh.send(Message::AudioControl {
            pc_to_mac: p,
            mac_to_pc: m,
        });
    }
    apply(sh);
}

pub(super) fn session_down(sh: &Arc<Sh>) {
    sh.st().audio.stop();
}

/// Volume e dispositivos mudaram.
pub(super) fn config_changed(sh: &Arc<Sh>) {
    let mut st = sh.st();
    let cfg = st.cfg.audio.clone();
    if let Some(p) = st.audio.rx.as_mut() {
        let mut dev = p.device.lock().unwrap_or_else(|e| e.into_inner());
        dev.set_volume(cfg.stream_volume.clamp(0.0, 1.0));
        if p.cur_device != cfg.mac_output_id {
            match dev.set_device(cfg.mac_output_id.as_deref()) {
                Ok(()) => p.cur_device = cfg.mac_output_id.clone(),
                Err(e) => tracing::warn!("saída de áudio {:?}: {e}", cfg.mac_output_id),
            }
        }
    }
    if let Some(c) = st.audio.tx.as_mut()
        && c.cur_device != cfg.mac_input_id
    {
        match c.device.set_device(cfg.mac_input_id.as_deref()) {
            Ok(()) => c.cur_device = cfg.mac_input_id.clone(),
            Err(e) => tracing::warn!("entrada de áudio {:?}: {e}", cfg.mac_input_id),
        }
    }
}

/// Liga ou desliga cada sentido para casar com `want` e a sessão atual.
fn apply(sh: &Arc<Sh>) {
    let (want, ctx, cfg) = {
        let st = sh.st();
        (st.want, st.session.clone(), st.cfg.audio.clone())
    };
    // Tira do estado antes de soltar: o `Drop` espera as threads, sem segurar o lock.
    let (old_rx, old_tx) = {
        let mut st = sh.st();
        let rx = if want.0 && ctx.is_some() && st.audio.rx.is_some() {
            None
        } else {
            st.audio.rx.take()
        };
        let tx = if want.1 && ctx.is_some() && st.audio.tx.is_some() {
            None
        } else {
            st.audio.tx.take()
        };
        (rx, tx)
    };
    drop((old_rx, old_tx));
    let Some(ctx) = ctx else { return };
    let Some(keys) = ctx.audio_keys.clone() else {
        if want.0 || want.1 {
            tracing::warn!("sessão sem chaves de áudio: áudio desligado");
        }
        return;
    };
    if want.0 && sh.st().audio.rx.is_none() {
        match start_playback(
            sh,
            &ctx,
            UdpAead::new(keys.pc_to_mac, ReplayMode::Window64),
            &cfg,
        ) {
            Ok(p) => sh.st().audio.rx = Some(p),
            Err(e) => tracing::warn!("áudio PC→Mac não iniciou: {e}"),
        }
    }
    if want.1 && sh.st().audio.tx.is_none() {
        match start_capture(
            sh,
            &ctx,
            UdpAead::new(keys.mac_to_pc, ReplayMode::Window64),
            &cfg,
        ) {
            Ok(c) => sh.st().audio.tx = Some(c),
            Err(e) => tracing::warn!("áudio Mac→PC não iniciou: {e}"),
        }
    }
}

// ---- PC → Mac ----

fn start_playback(
    sh: &Arc<Sh>,
    ctx: &SessionCtx,
    mut aead: UdpAead,
    cfg: &AudioConfig,
) -> Result<Playback, String> {
    let socket = UdpSocket::bind((ctx.bind_ip, ctx.local_ports.audio_pc_to_mac))
        .map_err(|e| format!("UDP {}: {e}", ctx.local_ports.audio_pc_to_mac))?;
    socket
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let addr = socket.local_addr().map_err(|e| e.to_string())?;
    let (producer, consumer) = rtrb::RingBuffer::new(CHUNK_FRAMES * CHANNELS * RING_CHUNKS * 2);
    let mut device = sh.platform.playback();
    device.set_volume(cfg.stream_volume.clamp(0.0, 1.0));
    if cfg.mac_output_id.is_some()
        && let Err(e) = device.set_device(cfg.mac_output_id.as_deref())
    {
        tracing::warn!("saída de áudio {:?}: {e}", cfg.mac_output_id);
    }
    device.start(consumer).map_err(|e| e.to_string())?;
    let device = Arc::new(Mutex::new(device));

    let jitter = Arc::new(Mutex::new(JitterBuffer::new(cfg.jitter_target_ms)));
    let mut stopper = Stopper::new();
    let (flag, peer_ip) = (stopper.flag.clone(), ctx.peer_ip);
    let jb = jitter.clone();
    stopper.spawn("relay-audio-rx", move || {
        let mut buf = [0u8; 2048];
        let mut plain = Vec::with_capacity(SEALED_HEAD + PACKET_PAYLOAD_BYTES + TAG_LEN);
        while !flag.load(Ordering::Relaxed) {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if from.ip() != peer_ip {
                continue;
            }
            let Some((seq, payload)) = open_datagram(&mut aead, &buf[..n], &mut plain) else {
                continue;
            };
            jb.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_payload(seq, payload)
                .ok();
        }
    });
    let flag = stopper.flag.clone();
    let events = sh.events.clone();
    stopper.spawn("relay-audio-play", move || {
        play_loop(&flag, &jitter, producer, &*events);
    });
    Ok(Playback {
        addr,
        device,
        cur_device: cfg.mac_output_id.clone(),
        stopper,
    })
}

/// Confere e decifra um datagrama; devolve `(seq, payload PCM)`.
fn open_datagram<'a>(
    aead: &mut UdpAead,
    datagram: &[u8],
    plain: &'a mut Vec<u8>,
) -> Option<(u64, &'a [u8])> {
    if datagram.len() != CLEAR_LEN + SEALED_HEAD + PACKET_PAYLOAD_BYTES + TAG_LEN {
        return None;
    }
    let (header, cipher) = datagram.split_at(CLEAR_LEN);
    let seq = u64::from_le_bytes(header[1..9].try_into().ok()?);
    aead.open(seq, header, cipher, plain).ok()?;
    let (head, payload) = plain.split_at_checked(SEALED_HEAD)?;
    let frames = u16::from_le_bytes([head[2], head[3]]);
    if head[0] != CODEC_PCM16
        || usize::from(head[1]) != CHANNELS
        || usize::from(frames) != FRAMES_PER_PACKET
        || payload.len() != PACKET_PAYLOAD_BYTES
    {
        return None;
    }
    Some((seq, payload))
}

/// Mantém o ring do dispositivo cheio até `RING_CHUNKS` chunks, tirando do jitter buffer pelo
/// compensador de deriva. O dispositivo consome no relógio dele: é isso que paceia o laço.
fn play_loop(
    stop: &AtomicBool,
    jitter: &Mutex<JitterBuffer>,
    mut ring: rtrb::Producer<f32>,
    events: &dyn CoreEvents,
) {
    let Ok(mut drift) = DriftCompensator::new(CHUNK_FRAMES) else {
        tracing::warn!("compensador de deriva indisponível");
        return;
    };
    let capacity = ring.buffer().capacity();
    let chunk_samples = CHUNK_FRAMES * CHANNELS;
    let mut pcm = vec![0i16; 4 * CHUNK_FRAMES * CHANNELS];
    let mut input = vec![0f32; pcm.len()];
    let mut out = vec![0f32; chunk_samples];
    let mut last_stats = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let filled = capacity - ring.slots();
        if filled >= RING_CHUNKS * chunk_samples {
            thread::sleep(Duration::from_micros(500));
        } else {
            let n = {
                let mut jb = jitter.lock().unwrap_or_else(|e| e.into_inner());
                let _ = drift.update(jb.mean_level_frames() as f32, jb.target_frames() as f32);
                let n = drift.input_frames_next() * CHANNELS;
                if n > pcm.len() {
                    pcm.resize(n, 0);
                    input.resize(n, 0.0);
                }
                jb.read(&mut pcm[..n]);
                n
            };
            if i16_to_f32(&pcm[..n], &mut input[..n]).is_err()
                || drift.process(&input[..n], &mut out).is_err()
            {
                continue;
            }
            if let Ok(mut chunk) = ring.write_chunk(chunk_samples) {
                let (a, b) = chunk.as_mut_slices();
                let (first, second) = out.split_at(a.len());
                a.copy_from_slice(first);
                b.copy_from_slice(second);
                chunk.commit_all();
            }
        }
        if last_stats.elapsed() >= STATS_EVERY {
            last_stats = Instant::now();
            let s = jitter.lock().unwrap_or_else(|e| e.into_inner()).stats();
            let ring_ms = (capacity - ring.slots()) as f32 / CHANNELS as f32 / 48.0;
            events.audio_stats(&AudioStats {
                buffer_ms: s.level_ms,
                loss_pct: s.loss_pct(),
                latency_ms: s.level_ms + ring_ms,
            });
        }
    }
}

// ---- Mac → PC ----

fn start_capture(
    sh: &Arc<Sh>,
    ctx: &SessionCtx,
    aead: UdpAead,
    cfg: &AudioConfig,
) -> Result<Capture, String> {
    let socket = UdpSocket::bind((ctx.bind_ip, 0)).map_err(|e| e.to_string())?;
    let dest = SocketAddr::new(ctx.peer_ip, ctx.remote_ports.audio_mac_to_pc);
    let (producer, mut consumer) = rtrb::RingBuffer::<AudioFrame>::new(CAPTURE_RING);
    let mut device = sh.platform.capture();
    if cfg.mac_input_id.is_some()
        && let Err(e) = device.set_device(cfg.mac_input_id.as_deref())
    {
        tracing::warn!("entrada de áudio {:?}: {e}", cfg.mac_input_id);
    }
    device.start(producer).map_err(|e| e.to_string())?;
    let mut stopper = Stopper::new();
    let flag = stopper.flag.clone();
    let counter = sh.st().tx_seq.clone();
    stopper.spawn("relay-audio-tx", move || {
        let mut pkt = AudioPacket {
            stream: STREAM_MAC_TO_PC,
            seq: 0,
            ts_samples: 0,
            codec: CODEC_PCM16,
            channels: CHANNELS as u8,
            frames: FRAMES_PER_PACKET as u16,
            payload: vec![0; PACKET_PAYLOAD_BYTES],
        };
        let mut wire = Vec::with_capacity(HEADER_LEN + PACKET_PAYLOAD_BYTES);
        let mut sealed = Vec::with_capacity(SEALED_HEAD + PACKET_PAYLOAD_BYTES + TAG_LEN);
        let mut datagram = Vec::with_capacity(HEADER_LEN + PACKET_PAYLOAD_BYTES + TAG_LEN);
        while !flag.load(Ordering::Relaxed) {
            let Ok(frame) = consumer.pop() else {
                thread::sleep(Duration::from_millis(1));
                continue;
            };
            debug_assert_eq!(frame.len(), AUDIO_FRAME_SAMPLES * CHANNELS);
            if i16_to_pcm_bytes(&frame, &mut pkt.payload).is_err() {
                continue;
            }
            pkt.seq = counter.fetch_add(1, Ordering::Relaxed);
            pkt.ts_samples = pkt.seq * FRAMES_PER_PACKET as u64;
            wire.clear();
            if pkt.encode(&mut wire).is_err() {
                continue;
            }
            let (header, payload) = wire.split_at(CLEAR_LEN);
            if aead.seal(pkt.seq, header, payload, &mut sealed).is_err() {
                continue;
            }
            datagram.clear();
            datagram.extend_from_slice(header);
            datagram.extend_from_slice(&sealed);
            let _ = socket.send_to(&datagram, dest);
        }
    });
    Ok(Capture {
        device,
        cur_device: cfg.mac_input_id.clone(),
        stopper,
    })
}
