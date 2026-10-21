//! dono: WI3
//! Áudio do agente (D2, D13, D14): som do PC → Mac (UDP 47802) e microfone do Mac → "CABLE Input"
//! (UDP 47803), ambos cifrados com as chaves de `Session::audio_link`.
//!
//! Datagrama (contrato no mural): `relay_core::audio::{seal,open}_datagram`; `stream` 0 = som do
//! PC, 1 = microfone do Mac.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use relay_core::audio::DATAGRAM_LEN;
use relay_core::audio::{
    CHANNELS, JitterBuffer, PLAIN_LEN, STREAM_MIC, STREAM_PC, i16_to_f32, open_datagram,
    seal_datagram,
};
use relay_core::crypto::{ReplayMode, UdpAead};
use relay_core::net::AudioLink;
use relay_core::platform::{AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, PlatformError};
use relay_core::types::{AudioConfig, AudioState, PcAudioMode, Ports, error_code};
use relay_platform_windows::audio_endpoints::{find_vbcable, find_virtual_output};

use super::platform::{AudioHost, LocalMute, OutputGuard};

const TAG_LEN: usize = 16;
const STATS_EVERY: Duration = Duration::from_secs(5);

/// Frames de 5 ms que o ring do dispositivo de reprodução mantém cheios (15 ms).
const RING_FILL_FRAMES: usize = 3;
const RING_SAMPLES: usize = AUDIO_FRAME_SAMPLES * CHANNELS * 8;
/// Espera máxima do `recv` do microfone: o datagrama acorda o laço na hora; o timeout só limita
/// o atraso do reabastecimento do ring (alvo de 15 ms), então 2 ms poupam despertares à toa.
const READ_TIMEOUT: Duration = Duration::from_millis(2);

fn plat_err(e: &PlatformError) -> String {
    match e {
        PlatformError::Unsupported => error_code::UNSUPPORTED.to_string(),
        other => other.to_string(),
    }
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
    mute: Option<Box<dyn LocalMute>>,
}

impl PcStream {
    fn stop(&mut self) {
        self.capture.stop();
        self.worker.stop();
        if let Some(m) = self.mute.as_mut()
            && let Err(e) = m.restore()
        {
            tracing::warn!(error = %e, "não consegui desmutar a saída do PC");
        }
        self.mute = None;
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
    next_seq: Arc<AtomicU64>,
) -> Result<PcStream, String> {
    fn begin(c: &mut dyn AudioCapture) -> Result<rtrb::Consumer<AudioFrame>, String> {
        let (tx, rx) = rtrb::RingBuffer::<AudioFrame>::new(64);
        c.start(tx).map_err(|e| plat_err(&e))?;
        Ok(rx)
    }
    let mut guard: Option<Box<dyn OutputGuard>> = None;
    let mut mute: Option<Box<dyn LocalMute>> = None;
    let (mut capture, ring_rx): (Box<dyn AudioCapture>, _) = match cfg.pc_mode {
        PcAudioMode::System
            if cfg
                .virtual_output_name
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty() =>
        {
            // Sem saída virtual: captura a saída padrão sem trocá-la (o PC também continua tocando).
            // O instalador do VB-CABLE a deixa como padrão: o loopback dele traria de volta o
            // microfone do Mac (que o agente toca ali) e o PC ficaria mudo. Sai dele para uma real.
            let devices = host.devices().map_err(|e| plat_err(&e))?;
            let cable = find_vbcable(&devices).input;
            let default_is_cable = cable
                .as_ref()
                .is_some_and(|c| devices.outputs.iter().any(|d| d.is_default && d.id == c.id));
            let real = default_is_cable
                .then(|| {
                    devices
                        .outputs
                        .iter()
                        .find(|d| !d.name.trim().to_lowercase().starts_with("cable"))
                })
                .flatten();
            let id = match real {
                Some(dev) => {
                    let mut g = host.guard().map_err(|e| plat_err(&e))?;
                    g.switch_to(&dev.id).map_err(|e| plat_err(&e))?;
                    guard = Some(g);
                    tracing::info!(saida = %dev.name, "saída padrão era o CABLE Input; trocada para captura");
                    Some(dev.id.clone())
                }
                None => {
                    if default_is_cable {
                        tracing::warn!("saída padrão é o CABLE Input e não há outra saída real");
                    }
                    None
                }
            };
            let mut c = host.loopback(id.clone());
            let rx = begin(&mut *c)?;
            // O som vai só para o Mac: o loopback captura antes do mudo do endpoint.
            mute = match host.mute(id) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "não consegui mutar o PC: o som sai nos dois");
                    None
                }
            };
            (c, rx)
        }
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
            let mut last = error_code::PROCESS_NOT_FOUND.to_string();
            let mut found = None;
            for exe in &cfg.process_names {
                let mut c = host.process_loopback(exe);
                match begin(&mut *c) {
                    Ok(rx) => {
                        found = Some((c, rx));
                        break;
                    }
                    Err(e) if e == error_code::UNSUPPORTED => last = e,
                    Err(_) => {}
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
    tracing::info!(%dest, "áudio PC→Mac iniciado");
    let thread = std::thread::Builder::new()
        .name("relay-audio-tx".into())
        .spawn(move || {
            let (mut plain, mut cipher, mut wire) = (
                Vec::with_capacity(PLAIN_LEN),
                Vec::with_capacity(PLAIN_LEN + TAG_LEN),
                Vec::with_capacity(DATAGRAM_LEN),
            );
            let (mut sent, mut send_errors, mut last) = (0u64, 0u64, Instant::now());
            while !flag.load(Ordering::Acquire) {
                if last.elapsed() >= STATS_EVERY {
                    last = Instant::now();
                    // Zero pacotes: endpoint em silêncio (o WASAPI não entrega frames) ou captura morta.
                    tracing::info!(sent, send_errors, %dest, "áudio PC→Mac");
                    (sent, send_errors) = (0, 0);
                }
                match rx.pop() {
                    Ok(frame) => {
                        // Contador por sessão e canal: nunca repete seq com a mesma chave (nonce).
                        let seq = next_seq.fetch_add(1, Ordering::Relaxed);
                        if seal_datagram(
                            &aead,
                            STREAM_PC,
                            seq,
                            &frame,
                            &mut plain,
                            &mut cipher,
                            &mut wire,
                        ) {
                            match socket.send_to(&wire, dest) {
                                Ok(_) => sent += 1,
                                Err(_) => send_errors += 1,
                            }
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(1)),
                }
            }
            tracing::info!(%dest, "áudio PC→Mac encerrado");
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
            mute,
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
        .set_read_timeout(Some(READ_TIMEOUT))
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
            let mut plain = Vec::with_capacity(PLAIN_LEN + TAG_LEN);
            let mut pcm = [0i16; AUDIO_FRAME_SAMPLES * CHANNELS];
            let mut f = [0f32; AUDIO_FRAME_SAMPLES * CHANNELS];
            while !flag.load(Ordering::Acquire) {
                if let Ok(n) = socket.recv(&mut wire) {
                    if let Some((seq, payload)) =
                        open_datagram(&mut aead, &wire[..n], STREAM_MIC, &mut plain)
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
    /// Código de erro (`vbcable_missing`, `virtual_output_missing`, `process_not_found`,
    /// `unsupported`) ou texto.
    pub error: Option<String>,
}

pub struct AudioEngine {
    host: Arc<dyn AudioHost>,
    pc: Option<PcStream>,
    mic: Option<MicStream>,
    /// Próximo seq do som do PC; só volta a 0 quando a sessão (e a chave) muda.
    pc_seq: Arc<AtomicU64>,
}

impl AudioEngine {
    pub fn new(host: Arc<dyn AudioHost>) -> Self {
        Self {
            host,
            pc: None,
            mic: None,
            pc_seq: Arc::default(),
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
            // Sessão encerrada: a próxima terá chaves novas.
            self.pc_seq = Arc::default();
            return AudioOutcome {
                state: self.state(),
                error: (want_pc || want_mic).then(|| "not_connected".to_string()),
            };
        };
        if !want_pc {
            self.pc = None;
        } else if self.pc.is_none() {
            match start_pc(&*self.host, cfg, ports, link, self.pc_seq.clone()) {
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
