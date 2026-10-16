//! dono: WI3
//! Áudio do agente (D2, D13, D14): som do PC → Mac (UDP 47802) e microfone do Mac → "CABLE Input"
//! (UDP 47803), ambos cifrados com as chaves de `Session::audio_link`.
//!
//! Datagrama (contrato no mural): `hdr17 ++ seal(seq, aad = hdr17, plain)`, com
//! `hdr17 = stream u8, seq u64 LE, ts_samples u64 LE` (os 17 primeiros bytes do `AudioPacket`)
//! e `plain = codec u8, channels u8, frames u16 LE, PCM s16le` (o resto do `AudioPacket`).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use relay_core::audio::{
    AudioHeader, CHANNELS, CODEC_PCM16, FRAMES_PER_PACKET, HEADER_LEN, JitterBuffer,
    PACKET_PAYLOAD_BYTES, i16_to_f32, i16_to_pcm_bytes,
};
use relay_core::crypto::{ReplayMode, UdpAead};
use relay_core::net::AudioLink;
use relay_core::platform::{AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, PlatformError};
use relay_core::types::{AudioConfig, AudioState, PcAudioMode, Ports, error_code};
use relay_platform_windows::audio_endpoints::{find_vbcable, find_virtual_output};

use super::platform::{AudioHost, OutputGuard};

/// `stream` do cabeçalho: 0 = som do PC, 1 = microfone do Mac.
pub const STREAM_PC: u8 = 0;
pub const STREAM_MIC: u8 = 1;

/// Bytes do cabeçalho autenticado (AAD) no fio.
const HDR17: usize = 17;
/// Parte cifrada: codec, channels, frames (4 B) + PCM.
const PLAIN_LEN: usize = (HEADER_LEN - HDR17) + PACKET_PAYLOAD_BYTES;
const TAG_LEN: usize = 16;
pub const DATAGRAM_LEN: usize = HDR17 + PLAIN_LEN + TAG_LEN;

/// Frames de 5 ms que o ring do dispositivo de reprodução mantém cheios (15 ms).
const RING_FILL_FRAMES: usize = 3;
const RING_SAMPLES: usize = AUDIO_FRAME_SAMPLES * CHANNELS * 8;

fn plat_err(e: &PlatformError) -> String {
    match e {
        PlatformError::Unsupported => error_code::UNSUPPORTED.to_string(),
        other => other.to_string(),
    }
}

/// Monta o datagrama de um quadro em `wire` (substitui o conteúdo).
pub fn encode_datagram(
    aead: &UdpAead,
    stream: u8,
    seq: u64,
    frame: &AudioFrame,
    plain: &mut Vec<u8>,
    cipher: &mut Vec<u8>,
    wire: &mut Vec<u8>,
) -> bool {
    let mut hdr = [0u8; HDR17];
    hdr[0] = stream;
    hdr[1..9].copy_from_slice(&seq.to_le_bytes());
    hdr[9..17].copy_from_slice(&(seq * FRAMES_PER_PACKET as u64).to_le_bytes());
    plain.clear();
    plain.push(CODEC_PCM16);
    plain.push(CHANNELS as u8);
    plain.extend_from_slice(&(FRAMES_PER_PACKET as u16).to_le_bytes());
    let start = plain.len();
    plain.resize(start + PACKET_PAYLOAD_BYTES, 0);
    if i16_to_pcm_bytes(frame, &mut plain[start..]).is_err() {
        return false;
    }
    if aead.seal(seq, &hdr, plain, cipher).is_err() {
        return false;
    }
    wire.clear();
    wire.extend_from_slice(&hdr);
    wire.extend_from_slice(cipher);
    true
}

/// Abre um datagrama; devolve `(seq, payload PCM)` em `pkt` (`hdr17 ++ plain`, reaproveitado).
pub fn decode_datagram<'a>(
    aead: &mut UdpAead,
    wire: &[u8],
    stream: u8,
    plain: &mut Vec<u8>,
    pkt: &'a mut Vec<u8>,
) -> Option<(u64, &'a [u8])> {
    if wire.len() != DATAGRAM_LEN || wire[0] != stream {
        return None;
    }
    let (hdr, cipher) = wire.split_at(HDR17);
    let seq = u64::from_le_bytes(hdr[1..9].try_into().ok()?);
    aead.open(seq, hdr, cipher, plain).ok()?;
    pkt.clear();
    pkt.extend_from_slice(hdr);
    pkt.extend_from_slice(plain);
    let (h, payload) = AudioHeader::parse(pkt).ok()?;
    Some((h.seq, payload))
}

fn unspecified(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => IpAddr::from([0, 0, 0, 0]),
        IpAddr::V6(_) => IpAddr::from([0u16; 8]),
    }
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Som do PC → Mac.
struct PcStream {
    capture: Box<dyn AudioCapture>,
    worker: Worker,
    guard: Option<Box<dyn OutputGuard>>,
}

impl PcStream {
    fn stop(&mut self) {
        self.capture.stop();
        self.worker.stop();
        if let Some(g) = self.guard.as_mut() {
            if let Err(e) = g.restore() {
                tracing::warn!(error = %e, "não consegui restaurar a saída padrão");
            }
        }
        self.guard = None;
    }
}

impl Drop for PcStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_pc(
    host: &dyn AudioHost,
    cfg: &AudioConfig,
    ports: &Ports,
    link: &AudioLink,
) -> Result<PcStream, String> {
    fn begin(c: &mut dyn AudioCapture) -> Result<rtrb::Consumer<AudioFrame>, String> {
        let (tx, rx) = rtrb::RingBuffer::<AudioFrame>::new(64);
        c.start(tx).map_err(|e| plat_err(&e))?;
        Ok(rx)
    }
    let mut guard: Option<Box<dyn OutputGuard>> = None;
    let (mut capture, ring_rx): (Box<dyn AudioCapture>, _) = match cfg.pc_mode {
        PcAudioMode::System => {
            let name = cfg.virtual_output_name.as_deref().unwrap_or("");
            let devices = host.devices().map_err(|e| plat_err(&e))?;
            let dev = find_virtual_output(&devices.outputs, name)
                .ok_or_else(|| error_code::VIRTUAL_OUTPUT_MISSING.to_string())?;
            let mut g = host.guard().map_err(|e| plat_err(&e))?;
            // Em erro daqui em diante, `g` cai e restaura a saída original.
            g.switch_to(&dev.id).map_err(|e| plat_err(&e))?;
            let mut c = host.loopback(Some(dev.id));
            let rx = begin(&mut *c)?;
            guard = Some(g);
            (c, rx)
        }
        PcAudioMode::Process => {
            let mut last = error_code::UNSUPPORTED.to_string();
            let mut found = None;
            for exe in &cfg.process_names {
                let mut c = host.process_loopback(exe);
                match begin(&mut *c) {
                    Ok(rx) => {
                        found = Some((c, rx));
                        break;
                    }
                    Err(e) => last = e,
                }
            }
            found.ok_or(last)?
        }
    };
    let socket =
        UdpSocket::bind(SocketAddr::new(unspecified(link.remote_ip), 0)).map_err(|e| e.to_string());
    let socket = match socket {
        Ok(s) => s,
        Err(e) => {
            capture.stop();
            return Err(e);
        }
    };
    let dest = SocketAddr::new(link.remote_ip, ports.audio_pc_to_mac);
    let aead = UdpAead::new(link.pc_to_mac.clone(), ReplayMode::Window64);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let mut rx = ring_rx;
    let thread = std::thread::Builder::new()
        .name("relay-audio-tx".into())
        .spawn(move || {
            let (mut plain, mut cipher, mut wire) = (
                Vec::with_capacity(PLAIN_LEN),
                Vec::with_capacity(PLAIN_LEN + TAG_LEN),
                Vec::with_capacity(DATAGRAM_LEN),
            );
            let mut seq = 0u64;
            while !flag.load(Ordering::Acquire) {
                match rx.pop() {
                    Ok(frame) => {
                        if encode_datagram(
                            &aead,
                            STREAM_PC,
                            seq,
                            &frame,
                            &mut plain,
                            &mut cipher,
                            &mut wire,
                        ) {
                            let _ = socket.send_to(&wire, dest);
                        }
                        seq += 1;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        })
        .map_err(|e| e.to_string());
    match thread {
        Ok(t) => Ok(PcStream {
            capture,
            worker: Worker {
                stop,
                thread: Some(t),
            },
            guard,
        }),
        Err(e) => {
            capture.stop();
            Err(e)
        }
    }
}

/// Microfone do Mac → "CABLE Input".
struct MicStream {
    render: Box<dyn relay_core::platform::AudioPlayback>,
    worker: Worker,
    addr: SocketAddr,
}

impl MicStream {
    fn stop(&mut self) {
        self.worker.stop();
        self.render.stop();
    }
}

impl Drop for MicStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_mic(
    host: &dyn AudioHost,
    cfg: &AudioConfig,
    ports: &Ports,
    link: &AudioLink,
) -> Result<MicStream, String> {
    let devices = host.devices().map_err(|e| plat_err(&e))?;
    let cable = find_vbcable(&devices)
        .input
        .ok_or_else(|| error_code::VBCABLE_MISSING.to_string())?;
    let socket = UdpSocket::bind(SocketAddr::new(
        unspecified(link.remote_ip),
        ports.audio_mac_to_pc,
    ))
    .map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(1)))
        .map_err(|e| e.to_string())?;
    let addr = socket.local_addr().map_err(|e| e.to_string())?;
    let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(RING_SAMPLES);
    let mut render = host.render(Some(cable.id));
    render.set_volume(1.0);
    render.start(cons).map_err(|e| plat_err(&e))?;

    let mut aead = UdpAead::new(link.mac_to_pc.clone(), ReplayMode::Window64);
    let mut jitter = JitterBuffer::new(cfg.jitter_target_ms);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("relay-audio-rx".into())
        .spawn(move || {
            let mut wire = vec![0u8; DATAGRAM_LEN + 64];
            let (mut plain, mut pkt) = (
                Vec::with_capacity(PLAIN_LEN + TAG_LEN),
                Vec::with_capacity(HDR17 + PLAIN_LEN),
            );
            let mut pcm = [0i16; AUDIO_FRAME_SAMPLES * CHANNELS];
            let mut f = [0f32; AUDIO_FRAME_SAMPLES * CHANNELS];
            while !flag.load(Ordering::Acquire) {
                if let Ok(n) = socket.recv(&mut wire) {
                    if let Some((seq, payload)) =
                        decode_datagram(&mut aead, &wire[..n], STREAM_MIC, &mut plain, &mut pkt)
                    {
                        let _ = jitter.push_payload(seq, payload);
                    }
                }
                // O relógio é o do dispositivo: reabastece o ring conforme ele consome.
                while RING_SAMPLES - prod.slots() < RING_FILL_FRAMES * pcm.len()
                    && prod.slots() >= pcm.len()
                {
                    jitter.read(&mut pcm);
                    if i16_to_f32(&pcm, &mut f).is_err() {
                        break;
                    }
                    for s in f {
                        let _ = prod.push(s);
                    }
                }
            }
        })
        .map_err(|e| e.to_string());
    match thread {
        Ok(t) => Ok(MicStream {
            render,
            worker: Worker {
                stop,
                thread: Some(t),
            },
            addr,
        }),
        Err(e) => {
            render.stop();
            Err(e)
        }
    }
}

/// Resultado de aplicar um `AudioControl`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioOutcome {
    pub state: AudioState,
    /// Código de erro (`vbcable_missing`, `virtual_output_missing`, `unsupported`) ou texto.
    pub error: Option<String>,
}

pub struct AudioEngine {
    host: Arc<dyn AudioHost>,
    pc: Option<PcStream>,
    mic: Option<MicStream>,
}

impl AudioEngine {
    pub fn new(host: Arc<dyn AudioHost>) -> Self {
        Self {
            host,
            pc: None,
            mic: None,
        }
    }

    pub fn state(&self) -> AudioState {
        AudioState {
            pc_to_mac: self.pc.is_some(),
            mac_to_pc: self.mic.is_some(),
        }
    }

    /// Endereço UDP em que o microfone do Mac é recebido (testes e diagnóstico).
    pub fn mic_addr(&self) -> Option<SocketAddr> {
        self.mic.as_ref().map(|m| m.addr)
    }

    /// Liga/desliga cada sentido conforme o pedido. Sem `link` (sessão caída) tudo é desligado
    /// e a saída padrão original volta.
    pub fn apply(
        &mut self,
        want_pc: bool,
        want_mic: bool,
        link: Option<&AudioLink>,
        cfg: &AudioConfig,
        ports: &Ports,
    ) -> AudioOutcome {
        let mut error = None;
        let Some(link) = link else {
            self.stop_all();
            return AudioOutcome {
                state: self.state(),
                error: (want_pc || want_mic).then(|| "not_connected".to_string()),
            };
        };
        if !want_pc {
            self.pc = None;
        } else if self.pc.is_none() {
            match start_pc(&*self.host, cfg, ports, link) {
                Ok(s) => self.pc = Some(s),
                Err(e) => error = Some(e),
            }
        }
        if !want_mic {
            self.mic = None;
        } else if self.mic.is_none() {
            match start_mic(&*self.host, cfg, ports, link) {
                Ok(s) => self.mic = Some(s),
                Err(e) => error = error.or(Some(e)),
            }
        }
        AudioOutcome {
            state: self.state(),
            error,
        }
    }

    pub fn stop_all(&mut self) {
        self.pc = None;
        self.mic = None;
    }
}

/// Estado de áudio visível à UI, compartilhado entre o worker e o pump.
pub type SharedAudioState = Arc<Mutex<AudioState>>;
