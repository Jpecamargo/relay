//! dono: WI3
//! Áudio do agente (D2, D13): microfone do Mac → "CABLE Input" (UDP 47803), cifrado com as
//! chaves de `Session::audio_link`.
//!
//! Datagrama (contrato no mural): `relay_core::audio::open_datagram`, `stream` 1.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use relay_core::audio::DATAGRAM_LEN;
use relay_core::audio::{CHANNELS, JitterBuffer, PLAIN_LEN, STREAM_MIC, i16_to_f32, open_datagram};
use relay_core::crypto::{ReplayMode, UdpAead};
use relay_core::net::AudioLink;
use relay_core::platform::{AUDIO_FRAME_SAMPLES, PlatformError};
use relay_core::types::{AudioConfig, AudioState, Ports, error_code};
use relay_platform_windows::audio_endpoints::find_vbcable;

use super::platform::AudioHost;

const TAG_LEN: usize = 16;

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
    /// Código de erro (`vbcable_missing`, `unsupported`) ou texto.
    pub error: Option<String>,
}

pub struct AudioEngine {
    host: Arc<dyn AudioHost>,
    mic: Option<MicStream>,
}

impl AudioEngine {
    pub fn new(host: Arc<dyn AudioHost>) -> Self {
        Self { host, mic: None }
    }

    pub fn state(&self) -> AudioState {
        AudioState {
            mic_to_peer: false,
            mic_from_peer: self.mic.is_some(),
        }
    }

    /// Endereço UDP em que o microfone do Mac é recebido (testes e diagnóstico).
    pub fn mic_addr(&self) -> Option<SocketAddr> {
        self.mic.as_ref().map(|m| m.addr)
    }

    /// Liga/desliga o microfone conforme o pedido. Sem `link` (sessão caída) ele é desligado.
    pub fn apply(
        &mut self,
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
                error: want_mic.then(|| "not_connected".to_string()),
            };
        };
        if !want_mic {
            self.mic = None;
        } else if self.mic.is_none() {
            match start_mic(&*self.host, cfg, ports, link) {
                Ok(s) => self.mic = Some(s),
                Err(e) => error = Some(e),
            }
        }
        AudioOutcome {
            state: self.state(),
            error,
        }
    }

    pub fn stop_all(&mut self) {
        self.mic = None;
    }
}

/// Estado de áudio visível à UI, compartilhado entre o worker e o pump.
pub type SharedAudioState = Arc<Mutex<AudioState>>;
