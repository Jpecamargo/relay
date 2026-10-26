//! dono: MI3
//! Microfone do Mac no PC (D13): PCM s16le, 48 kHz estéreo, quadros de 5 ms.
//! `AudioCapture` → `AudioPacket` → `UdpAead` → UDP `audio_mac_to_pc`.
//!
//! Datagrama (contrato do WI3): `relay_core::audio::seal_datagram`, `stream` = 1.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use relay_core::audio::{CHANNELS, DATAGRAM_LEN, PLAIN_LEN, STREAM_MIC, seal_datagram};
use relay_core::crypto::{ReplayMode, UdpAead};
use relay_core::platform::{AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame};
use relay_core::types::{AudioConfig, Message};

use super::Sh;
use crate::core::SessionCtx;

const TAG_LEN: usize = 16;
const CAPTURE_RING: usize = 64;

/// O microfone de uma sessão, quando ligado.
#[derive(Default)]
pub(crate) struct AudioRun {
    tx: Option<Capture>,
}

impl AudioRun {
    #[cfg(test)]
    pub fn tx_running(&self) -> bool {
        self.tx.is_some()
    }

    pub fn stop(&mut self) {
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

/// Pedido do usuário: manda `AudioControl` e liga/desliga o microfone.
pub(super) fn set_streams(sh: &Arc<Sh>, mac_to_pc: bool) {
    let has_session = sh.session().is_some();
    tracing::info!(mac_to_pc, has_session, "áudio: pedido do usuário");
    sh.st().want = mac_to_pc;
    sh.send(Message::AudioControl {
        pc_to_mac: false,
        mac_to_pc,
    });
    apply(sh);
}

/// O PC respondeu com o estado real do microfone: ajusta o local, sem reenviar.
pub(super) fn pc_state(sh: &Arc<Sh>, mac_to_pc: bool) {
    sh.st().want = mac_to_pc;
    apply(sh);
}

/// Sessão nova: refaz o que o usuário pediu com as chaves dela.
pub(super) fn session_up(sh: &Arc<Sh>) {
    // Chaves novas: nada do que corria com as velhas serve.
    sh.st().audio.stop();
    if sh.st().want {
        sh.send(Message::AudioControl {
            pc_to_mac: false,
            mac_to_pc: true,
        });
    }
    apply(sh);
}

pub(super) fn session_down(sh: &Arc<Sh>) {
    sh.st().audio.stop();
}

/// O microfone escolhido mudou.
pub(super) fn config_changed(sh: &Arc<Sh>) {
    let mut st = sh.st();
    let input = st.cfg.audio.mac_input_id.clone();
    if let Some(c) = st.audio.tx.as_mut()
        && c.cur_device != input
    {
        match c.device.set_device(input.as_deref()) {
            Ok(()) => c.cur_device = input,
            Err(e) => tracing::warn!("entrada de áudio {input:?}: {e}"),
        }
    }
}

/// Liga ou desliga o microfone para casar com `want` e a sessão atual.
fn apply(sh: &Arc<Sh>) {
    let (want, ctx, cfg) = {
        let st = sh.st();
        (st.want, st.session.clone(), st.cfg.audio.clone())
    };
    // Tira do estado antes de soltar: o `Drop` espera as threads, sem segurar o lock.
    let old = {
        let mut st = sh.st();
        if want && ctx.is_some() && st.audio.tx.is_some() {
            None
        } else {
            st.audio.tx.take()
        }
    };
    drop(old);
    let Some(ctx) = ctx else {
        if want {
            tracing::warn!("áudio pedido sem sessão de mídia: liga quando a sessão subir");
        }
        return;
    };
    let Some(keys) = ctx.audio_keys.clone() else {
        if want {
            tracing::warn!("sessão sem chaves de áudio: áudio desligado");
        }
        return;
    };
    if want && sh.st().audio.tx.is_none() {
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
        let (mut plain, mut sealed, mut datagram) = (
            Vec::with_capacity(PLAIN_LEN),
            Vec::with_capacity(PLAIN_LEN + TAG_LEN),
            Vec::with_capacity(DATAGRAM_LEN),
        );
        while !flag.load(Ordering::Relaxed) {
            let Ok(frame) = consumer.pop() else {
                thread::sleep(Duration::from_millis(1));
                continue;
            };
            debug_assert_eq!(frame.len(), AUDIO_FRAME_SAMPLES * CHANNELS);
            let seq = counter.fetch_add(1, Ordering::Relaxed);
            if !seal_datagram(
                &aead,
                STREAM_MIC,
                seq,
                &frame,
                &mut plain,
                &mut sealed,
                &mut datagram,
            ) {
                continue;
            }
            let _ = socket.send_to(&datagram, dest);
        }
    });
    Ok(Capture {
        device,
        cur_device: cfg.mac_input_id.clone(),
        stopper,
    })
}
