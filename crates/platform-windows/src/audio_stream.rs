//! dono: W4b
//! Streams de áudio: loopback WASAPI de endpoint, loopback por processo e reprodução.
//!
//! Tudo em modo compartilhado orientado a evento, 48 kHz estéreo (a conversão de taxa fica com
//! o `autoconvert` do WASAPI; downmix e i16 em `audio_convert`). Cada stream roda numa thread
//! própria no MMCSS "Pro Audio". Só a parte Win32 fica sob `cfg(windows)`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use relay_core::platform::{AudioCapture, AudioFrame, AudioPlayback, PlatformError};
use relay_core::types::AudioDevice;

/// Primeiro build do Windows 10 2004, onde surgiu o loopback por processo.
const MIN_PROCESS_LOOPBACK_BUILD: u32 = 19041;

/// Loopback por processo exige Windows 10 2004 (build 19041) ou mais novo.
pub fn process_loopback_supported(build: u32) -> bool {
    build >= MIN_PROCESS_LOOPBACK_BUILD
}

/// `path` (caminho completo de imagem de um processo) termina no executável `exe_name`,
/// sem diferenciar maiúsculas; aceita `chrome` e `chrome.exe`.
pub fn exe_matches(path: &str, exe_name: &str) -> bool {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let want = exe_name.trim();
    if want.is_empty() {
        return false;
    }
    file.eq_ignore_ascii_case(want)
        || (!want.to_ascii_lowercase().ends_with(".exe")
            && file.eq_ignore_ascii_case(&format!("{want}.exe")))
}

#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Target {
    /// Loopback de um endpoint de render (`None` = padrão).
    Endpoint(Option<String>),
    Process(String),
}

type CaptureJoin = JoinHandle<rtrb::Producer<AudioFrame>>;
type RenderJoin = JoinHandle<rtrb::Consumer<f32>>;
type RenderSpawn = Result<RenderJoin, (PlatformError, Option<rtrb::Consumer<f32>>)>;
type CaptureSpawn = Result<CaptureJoin, (PlatformError, Option<rtrb::Producer<AudioFrame>>)>;

/// Estado comum das capturas: a thread devolve o `Producer` ao terminar, para `set_device`
/// poder reiniciar sem o chamador reenviar o ring.
#[derive(Debug)]
struct CaptureRunner {
    stop: Arc<AtomicBool>,
    thread: Option<CaptureJoin>,
    sink: Option<rtrb::Producer<AudioFrame>>,
}

impl Default for CaptureRunner {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            sink: None,
        }
    }
}

impl CaptureRunner {
    fn start(
        &mut self,
        target: Target,
        sink: rtrb::Producer<AudioFrame>,
    ) -> Result<(), PlatformError> {
        self.stop();
        self.stop = Arc::new(AtomicBool::new(false));
        match imp::spawn_capture(target, sink, self.stop.clone()) {
            Ok(handle) => {
                self.thread = Some(handle);
                Ok(())
            }
            Err((e, sink)) => {
                self.sink = sink;
                Err(e)
            }
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.thread.take() {
            if let Ok(sink) = h.join() {
                self.sink = Some(sink);
            }
        }
    }

    fn running(&self) -> bool {
        self.thread.is_some()
    }

    /// Reinicia com outro alvo reaproveitando o ring; sem stream em execução só guarda o alvo.
    fn restart(&mut self, target: Target) -> Result<(), PlatformError> {
        if !self.running() {
            return Ok(());
        }
        self.stop();
        match self.sink.take() {
            Some(sink) => self.start(target, sink),
            None => Err(PlatformError::Platform("ring de captura perdido".into())),
        }
    }
}

/// Loopback WASAPI de um endpoint de render (o que o PC está tocando), em quadros i16 48 kHz
/// estéreo de 240 amostras.
#[derive(Debug, Default)]
pub struct LoopbackCapture {
    endpoint_id: Option<String>,
    runner: CaptureRunner,
}

impl LoopbackCapture {
    /// `None` = endpoint de render padrão.
    pub fn new(endpoint_id: Option<String>) -> Self {
        Self {
            endpoint_id,
            runner: CaptureRunner::default(),
        }
    }
}

impl AudioCapture for LoopbackCapture {
    fn start(&mut self, sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        self.runner
            .start(Target::Endpoint(self.endpoint_id.clone()), sink)
    }

    fn stop(&mut self) {
        self.runner.stop();
    }

    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError> {
        self.endpoint_id = device_id.map(str::to_owned);
        self.runner
            .restart(Target::Endpoint(self.endpoint_id.clone()))
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.runner.stop();
    }
}

/// Loopback por processo (Windows 10 2004+); em build anterior devolve `Unsupported`.
/// Captura o processo `exe_name` e seus filhos (include tree).
#[derive(Debug, Default)]
pub struct ProcessLoopbackCapture {
    exe_name: String,
    runner: CaptureRunner,
}

impl ProcessLoopbackCapture {
    pub fn new(exe_name: impl Into<String>) -> Self {
        Self {
            exe_name: exe_name.into(),
            runner: CaptureRunner::default(),
        }
    }
}

impl AudioCapture for ProcessLoopbackCapture {
    fn start(&mut self, sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        self.runner
            .start(Target::Process(self.exe_name.clone()), sink)
    }

    fn stop(&mut self) {
        self.runner.stop();
    }

    fn set_device(&mut self, _device_id: Option<&str>) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

impl Drop for ProcessLoopbackCapture {
    fn drop(&mut self) {
        self.runner.stop();
    }
}

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

    pub fn spawn_capture(
        _target: Target,
        sink: rtrb::Producer<AudioFrame>,
        _stop: Arc<AtomicBool>,
    ) -> CaptureSpawn {
        Err((PlatformError::Unsupported, Some(sink)))
    }

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
    use crate::audio_convert::{f32_from_le, fill_f32_le, push_f32_interleaved};
    use relay_core::audio::FrameSplitter;
    use std::sync::mpsc;
    use wasapi::{
        AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
        calculate_period_100ns, initialize_mta,
    };
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::ProcessStatus::EnumProcesses;
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, OpenProcess,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::core::{PWSTR, w};

    const RATE: usize = 48_000;
    /// Espera máxima por evento antes de reavaliar o `stop`.
    const WAIT_MS: u32 = 100;
    /// 1 s de áudio por leitura: folga de sobra para qualquer pacote do WASAPI.
    const MAX_BYTES_PER_CHANNEL: usize = RATE * 4;

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

    fn current_build() -> Option<u32> {
        let mut buf = [0u16; 16];
        let mut size = std::mem::size_of_val(&buf) as u32;
        unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                w!("CurrentBuildNumber"),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
            .ok()
            .ok()?;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len]).trim().parse().ok()
    }

    /// Menor PID cujo executável é `exe_name`.
    fn find_pid(exe_name: &str) -> Result<u32, PlatformError> {
        let mut pids = vec![0u32; 4096];
        let mut needed = 0u32;
        unsafe {
            EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed)
                .map_err(|e| err("EnumProcesses", e))?;
        }
        pids.truncate(needed as usize / 4);
        pids.sort_unstable();
        for pid in pids.into_iter().filter(|&p| p != 0) {
            let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
            else {
                continue;
            };
            let mut path = [0u16; 1024];
            let mut len = path.len() as u32;
            let ok = unsafe {
                QueryFullProcessImageNameW(
                    h,
                    PROCESS_NAME_WIN32,
                    PWSTR(path.as_mut_ptr()),
                    &mut len,
                )
            }
            .is_ok();
            let _ = unsafe { CloseHandle(h) };
            if ok && exe_matches(&String::from_utf16_lossy(&path[..len as usize]), exe_name) {
                return Ok(pid);
            }
        }
        Err(PlatformError::Platform(format!(
            "processo '{exe_name}' não encontrado"
        )))
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

    struct CaptureSetup {
        client: AudioClient,
        channels: usize,
    }

    fn setup_capture(target: &Target) -> Result<CaptureSetup, PlatformError> {
        let (mut client, fmt) = match target {
            Target::Endpoint(id) => {
                let client = render_device(id.as_deref())?
                    .get_iaudioclient()
                    .map_err(|e| err("IAudioClient", e))?;
                let mix = client.get_mixformat().map_err(|e| err("mix format", e))?;
                let ch = mix.get_nchannels() as usize;
                let fmt = WaveFormat::new(
                    32,
                    32,
                    &SampleType::Float,
                    RATE,
                    ch,
                    Some(mix.get_dwchannelmask()),
                );
                (client, fmt)
            }
            Target::Process(exe) => {
                if !current_build().is_some_and(process_loopback_supported) {
                    return Err(PlatformError::Unsupported);
                }
                let pid = find_pid(exe)?;
                let client = AudioClient::new_application_loopback_client(pid, true)
                    .map_err(|e| err("loopback por processo", e))?;
                (
                    client,
                    WaveFormat::new(32, 32, &SampleType::Float, RATE, 2, Some(0b11)),
                )
            }
        };
        let channels = fmt.get_nchannels() as usize;
        client
            .initialize_client(&fmt, &Direction::Capture, &shared_events())
            .map_err(|e| err("initialize_client", e))?;
        Ok(CaptureSetup { client, channels })
    }

    type Ready = mpsc::Sender<Result<(), PlatformError>>;

    fn capture_loop(
        s: CaptureSetup,
        sink: &mut rtrb::Producer<AudioFrame>,
        stop: &AtomicBool,
        ready: Ready,
    ) {
        let started = (|| {
            let ev = s
                .client
                .set_get_eventhandle()
                .map_err(|e| err("evento", e))?;
            let cap = s
                .client
                .get_audiocaptureclient()
                .map_err(|e| err("capture client", e))?;
            s.client.start_stream().map_err(|e| err("start", e))?;
            Ok::<_, PlatformError>((ev, cap))
        })();
        let (ev, cap) = match started {
            Ok(v) => {
                let _ = ready.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        let mut bytes = vec![0u8; MAX_BYTES_PER_CHANNEL * s.channels.max(1)];
        let mut samples: Vec<f32> = Vec::with_capacity(bytes.len() / 4);
        let mut splitter = FrameSplitter::new();
        'outer: while !stop.load(Ordering::Acquire) {
            if ev.wait_for_event(WAIT_MS).is_err() {
                continue;
            }
            loop {
                match cap.get_next_packet_size() {
                    Ok(Some(n)) if n > 0 => {}
                    Ok(_) => break,
                    Err(_) => break 'outer,
                }
                let Ok((frames, info)) = cap.read_from_device(&mut bytes) else {
                    break 'outer;
                };
                let len = frames as usize * s.channels * 4;
                if info.flags.silent {
                    bytes[..len].fill(0);
                }
                f32_from_le(&bytes[..len], &mut samples);
                // Ring cheio: descarta o quadro (consumidor atrasado), nunca bloqueia.
                push_f32_interleaved(&mut splitter, &samples, s.channels, |f| {
                    let _ = sink.push(*f);
                });
            }
        }
        let _ = s.client.stop_stream();
    }

    pub fn spawn_capture(
        target: Target,
        sink: rtrb::Producer<AudioFrame>,
        stop: Arc<AtomicBool>,
    ) -> CaptureSpawn {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("relay-audio-capture".into())
            .spawn(move || {
                let mut sink = sink;
                let _ = initialize_mta().ok();
                let _mmcss = Mmcss::pro_audio();
                match setup_capture(&target) {
                    Ok(s) => capture_loop(s, &mut sink, &stop, tx),
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
                sink
            })
            .map_err(|e| (err("thread de captura", e), None))?;
        match rx.recv() {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(e)) => Err((e, handle.join().ok())),
            Err(_) => Err((
                PlatformError::Platform("thread de captura terminou antes de iniciar".into()),
                handle.join().ok(),
            )),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_loopback_build_gate() {
        assert!(!process_loopback_supported(18363));
        assert!(!process_loopback_supported(19040));
        assert!(process_loopback_supported(19041));
        assert!(process_loopback_supported(26100));
    }

    #[test]
    fn exe_name_matching() {
        assert!(exe_matches(
            r"C:\Program Files\Google\Chrome\chrome.exe",
            "chrome.exe"
        ));
        assert!(exe_matches(r"C:\x\Chrome.EXE", "chrome"));
        assert!(exe_matches("spotify.exe", "Spotify.exe"));
        assert!(!exe_matches(r"C:\x\notchrome.exe", "chrome.exe"));
        assert!(!exe_matches(r"C:\x\chrome.exe", ""));
        assert!(!exe_matches(r"C:\chrome.exe\other.exe", "chrome.exe"));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_streams_are_unsupported() {
        let (p, _c) = rtrb::RingBuffer::<AudioFrame>::new(4);
        let mut cap = LoopbackCapture::new(None);
        assert!(matches!(cap.start(p), Err(PlatformError::Unsupported)));
        let (_p, c) = rtrb::RingBuffer::<f32>::new(4);
        let mut r = EndpointRender::new(None);
        assert!(matches!(r.start(c), Err(PlatformError::Unsupported)));
        assert!(matches!(r.list_devices(), Err(PlatformError::Unsupported)));
    }
}
