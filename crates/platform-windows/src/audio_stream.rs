//! dono: W4b
//! Reprodução de áudio num endpoint WASAPI (o microfone do Mac no "CABLE Input").
//!
//! Modo compartilhado orientado a evento, 48 kHz estéreo (a conversão de taxa fica com o
//! `autoconvert` do WASAPI). O stream roda numa thread própria no MMCSS "Pro Audio". Só a parte
//! Win32 fica sob `cfg(windows)`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use relay_core::platform::{AudioPlayback, PlatformError};
use relay_core::types::AudioDevice;

type RenderJoin = JoinHandle<rtrb::Consumer<f32>>;
type RenderSpawn = Result<RenderJoin, (PlatformError, Option<rtrb::Consumer<f32>>)>;

/// Reprodução num endpoint (CABLE Input) a partir de um ring de f32 intercalado 48 kHz estéreo.
#[derive(Debug)]
pub struct EndpointRender {
    endpoint_id: Option<String>,
    volume: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    thread: Option<RenderJoin>,
    source: Option<rtrb::Consumer<f32>>,
}

impl Default for EndpointRender {
    fn default() -> Self {
        Self::new(None)
    }
}

impl EndpointRender {
    /// `None` = endpoint de render padrão.
    pub fn new(endpoint_id: Option<String>) -> Self {
        Self {
            endpoint_id,
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            source: None,
        }
    }

    fn spawn(&mut self, source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        self.stop = Arc::new(AtomicBool::new(false));
        match imp::spawn_render(
            self.endpoint_id.clone(),
            source,
            self.volume.clone(),
            self.stop.clone(),
        ) {
            Ok(h) => {
                self.thread = Some(h);
                Ok(())
            }
            Err((e, source)) => {
                self.source = source;
                Err(e)
            }
        }
    }
}

impl AudioPlayback for EndpointRender {
    fn start(&mut self, source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        self.stop();
        self.spawn(source)
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.thread.take() {
            if let Ok(src) = h.join() {
                self.source = Some(src);
            }
        }
    }

    fn set_volume(&mut self, volume: f32) {
        let v = if volume.is_nan() {
            1.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        self.volume.store(v.to_bits(), Ordering::Relaxed);
    }

    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError> {
        self.endpoint_id = device_id.map(str::to_owned);
        if self.thread.is_none() {
            return Ok(());
        }
        self.stop();
        match self.source.take() {
            Some(src) => self.spawn(src),
            None => Err(PlatformError::Platform("ring de reprodução perdido".into())),
        }
    }

    fn list_devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        imp::list_render_devices()
    }
}

impl Drop for EndpointRender {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn spawn_render(
        _id: Option<String>,
        source: rtrb::Consumer<f32>,
        _volume: Arc<AtomicU32>,
        _stop: Arc<AtomicBool>,
    ) -> RenderSpawn {
        Err((PlatformError::Unsupported, Some(source)))
    }

    pub fn list_render_devices() -> Result<Vec<AudioDevice>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::audio_convert::fill_f32_le;
    use std::sync::mpsc;
    use wasapi::{
        AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
        calculate_period_100ns, initialize_mta,
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
    };
    use windows::core::w;

    const RATE: usize = 48_000;
    /// Espera máxima por evento antes de reavaliar o `stop`.
    const WAIT_MS: u32 = 100;

    fn err(ctx: &str, e: impl std::fmt::Display) -> PlatformError {
        PlatformError::Platform(format!("{ctx}: {e}"))
    }

    /// Entra no MMCSS "Pro Audio" enquanto viver.
    struct Mmcss(Option<HANDLE>);

    impl Mmcss {
        fn pro_audio() -> Self {
            let mut index = 0u32;
            // Falha não é fatal: o stream funciona, só sem prioridade de multimídia.
            Self(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.ok())
        }
    }

    impl Drop for Mmcss {
        fn drop(&mut self) {
            if let Some(h) = self.0 {
                let _ = unsafe { AvRevertMmThreadCharacteristics(h) };
            }
        }
    }

    fn render_device(id: Option<&str>) -> Result<wasapi::Device, PlatformError> {
        let en = DeviceEnumerator::new().map_err(|e| err("DeviceEnumerator", e))?;
        match id {
            Some(id) => en.get_device(id),
            None => en.get_default_device(&Direction::Render),
        }
        .map_err(|e| err("endpoint de render", e))
    }

    fn shared_events() -> StreamMode {
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: calculate_period_100ns(RATE as i64 / 50, RATE as i64),
        }
    }

    type Ready = mpsc::Sender<Result<(), PlatformError>>;

    fn render_loop(
        mut client: AudioClient,
        source: &mut rtrb::Consumer<f32>,
        volume: &AtomicU32,
        stop: &AtomicBool,
        ready: Ready,
    ) {
        let started = (|| {
            let fmt = WaveFormat::new(32, 32, &SampleType::Float, RATE, 2, Some(0b11));
            client
                .initialize_client(&fmt, &Direction::Render, &shared_events())
                .map_err(|e| err("initialize_client", e))?;
            let ev = client.set_get_eventhandle().map_err(|e| err("evento", e))?;
            let rc = client
                .get_audiorenderclient()
                .map_err(|e| err("render client", e))?;
            let total = client.get_buffer_size().map_err(|e| err("buffer", e))? as usize;
            // Buffer pré-preenchido com silêncio, como manda o WASAPI antes do Start.
            let silence = vec![0u8; total * 8];
            rc.write_to_device(total, &silence, None)
                .map_err(|e| err("prefill", e))?;
            client.start_stream().map_err(|e| err("start", e))?;
            Ok::<_, PlatformError>((ev, rc, total))
        })();
        let (ev, rc, total) = match started {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        let mut bytes = vec![0u8; total * 8];
        while !stop.load(Ordering::Acquire) {
            if ev.wait_for_event(WAIT_MS).is_err() {
                continue;
            }
            let Ok(free) = client.get_available_space_in_frames() else {
                break;
            };
            let frames = (free as usize).min(total);
            if frames == 0 {
                continue;
            }
            let gain = f32::from_bits(volume.load(Ordering::Relaxed));
            let out = &mut bytes[..frames * 8];
            fill_f32_le(out, gain, || source.pop().ok());
            if rc.write_to_device(frames, out, None).is_err() {
                break;
            }
        }
        let _ = client.stop_stream();
    }

    pub fn spawn_render(
        id: Option<String>,
        source: rtrb::Consumer<f32>,
        volume: Arc<AtomicU32>,
        stop: Arc<AtomicBool>,
    ) -> RenderSpawn {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("relay-audio-render".into())
            .spawn(move || {
                let mut source = source;
                let _ = initialize_mta().ok();
                let _mmcss = Mmcss::pro_audio();
                match render_device(id.as_deref())
                    .and_then(|d| d.get_iaudioclient().map_err(|e| err("IAudioClient", e)))
                {
                    Ok(client) => render_loop(client, &mut source, &volume, &stop, tx),
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
                source
            })
            .map_err(|e| (err("thread de reprodução", e), None))?;
        match rx.recv() {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(e)) => Err((e, handle.join().ok())),
            Err(_) => Err((
                PlatformError::Platform("thread de reprodução terminou antes de iniciar".into()),
                handle.join().ok(),
            )),
        }
    }

    pub fn list_render_devices() -> Result<Vec<AudioDevice>, PlatformError> {
        // Enumeração pode rodar em qualquer thread do chamador: garante COM no apartamento dela.
        let _ = initialize_mta().ok();
        let en = DeviceEnumerator::new().map_err(|e| err("DeviceEnumerator", e))?;
        let default = en
            .get_default_device(&Direction::Render)
            .and_then(|d| d.get_id())
            .unwrap_or_default();
        let coll = en
            .get_device_collection(&Direction::Render)
            .map_err(|e| err("coleção de endpoints", e))?;
        let n = coll.get_nbr_devices().map_err(|e| err("contagem", e))?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let dev = coll
                .get_device_at_index(i)
                .map_err(|e| err("endpoint", e))?;
            let id = dev.get_id().map_err(|e| err("id", e))?;
            let name = dev.get_friendlyname().unwrap_or_else(|_| id.clone());
            out.push(AudioDevice {
                is_default: id == default,
                id,
                name,
            });
        }
        Ok(out)
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn non_windows_render_is_unsupported() {
        let (_p, c) = rtrb::RingBuffer::<f32>::new(4);
        let mut r = EndpointRender::new(None);
        assert!(matches!(r.start(c), Err(PlatformError::Unsupported)));
        assert!(matches!(r.list_devices(), Err(PlatformError::Unsupported)));
    }
}
