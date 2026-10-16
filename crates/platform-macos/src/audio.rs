//! dono: M4
//! Áudio no macOS com cpal: enumeração, reprodução do stream do PC e captura do microfone.
//!
//! Formato do fio (D13): 48 kHz estéreo, quadros de 5 ms (240 amostras por canal).
//! A reprodução consome f32 intercalado de um ring sem lock; a captura entrega quadros i16 exatos
//! a um ring sem lock. Nos callbacks do sistema nada é alocado: os buffers nascem na criação do
//! stream. A posse do ring fica num `Arc<Mutex<_>>` só para sobreviver à troca de dispositivo
//! (o stream antigo morre, o novo reaproveita o ring); o callback usa `try_lock`, que nunca bloqueia.

use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use relay_core::audio::{FrameSplitter, f32_sample_to_i16 as f32_to_i16};
use relay_core::platform::{
    AUDIO_CHANNELS, AudioCapture, AudioFrame, AudioPlayback, PlatformError,
};
use relay_core::types::{AudioDevice, AudioDevices};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// Taxa do fio.
pub const WIRE_RATE: u32 = 48_000;
/// Quadros de entrada por chamada do reamostrador (10 ms na taxa de entrada, ordem de grandeza).
const RESAMPLER_CHUNK: usize = 480;

fn err(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Platform(e.to_string())
}

// ---------------------------------------------------------------------------
// Enumeração
// ---------------------------------------------------------------------------

fn describe(device: &cpal::Device, default_id: Option<&str>) -> Option<AudioDevice> {
    let id = device.id().ok()?.to_string();
    let name = device.description().ok()?.name().to_string();
    let is_default = default_id == Some(id.as_str());
    Some(AudioDevice {
        id,
        name,
        is_default,
    })
}

pub fn list_input_devices() -> Result<Vec<AudioDevice>, PlatformError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());
    let devices = host.input_devices().map_err(err)?;
    Ok(devices
        .filter_map(|d| describe(&d, default_id.as_deref()))
        .collect())
}

pub fn list_output_devices() -> Result<Vec<AudioDevice>, PlatformError> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());
    let devices = host.output_devices().map_err(err)?;
    Ok(devices
        .filter_map(|d| describe(&d, default_id.as_deref()))
        .collect())
}

/// Entradas e saídas do Mac (id, nome, padrão).
pub fn list_devices() -> Result<AudioDevices, PlatformError> {
    Ok(AudioDevices {
        outputs: list_output_devices()?,
        inputs: list_input_devices()?,
    })
}

/// Resolve o id escolhido; `None` (ou id que sumiu) cai no padrão do sistema só se `None`.
fn find_device(id: Option<&str>, input: bool) -> Result<cpal::Device, PlatformError> {
    let host = cpal::default_host();
    match id {
        Some(id) => {
            let id = cpal::DeviceId::from_str(id).map_err(err)?;
            host.device_by_id(&id)
                .ok_or_else(|| PlatformError::Platform(format!("dispositivo {id} não encontrado")))
        }
        None => if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
        .ok_or_else(|| PlatformError::Platform("sem dispositivo de áudio padrão".into())),
    }
}

// ---------------------------------------------------------------------------
// Conversões puras (testadas sem dispositivo)
// ---------------------------------------------------------------------------

/// Junta f32 intercalado de `channels` canais em estéreo intercalado: mono duplica, 3+ canais
/// ficam com os dois primeiros.
fn to_stereo_into(input: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => out.extend(input.iter().flat_map(|&s| [s, s])),
        2 => out.extend_from_slice(input),
        n => {
            for frame in input.chunks_exact(n) {
                out.push(frame[0]);
                out.push(frame[1]);
            }
        }
    }
}

/// Entrada do dispositivo (qualquer taxa/canais) -> quadros de 48 kHz estéreo i16.
struct CaptureConverter {
    channels: usize,
    resampler: Option<Async<f32>>,
    /// Estéreo f32 ainda não reamostrado.
    acc: Vec<f32>,
    out: Vec<f32>,
    pcm: Vec<i16>,
    splitter: FrameSplitter,
}

impl CaptureConverter {
    fn new(rate: u32, channels: usize) -> Result<Self, PlatformError> {
        let resampler = if rate == WIRE_RATE {
            None
        } else {
            let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2)
                .oversampling_factor(128)
                .interpolation(SincInterpolationType::Linear);
            Some(
                Async::<f32>::new_sinc(
                    f64::from(WIRE_RATE) / f64::from(rate),
                    1.0,
                    &params,
                    RESAMPLER_CHUNK,
                    AUDIO_CHANNELS,
                    FixedAsync::Input,
                )
                .map_err(err)?,
            )
        };
        let out_cap = resampler
            .as_ref()
            .map_or(0, |r| r.output_frames_max() * AUDIO_CHANNELS);
        Ok(Self {
            channels,
            resampler,
            acc: Vec::with_capacity(16 * 1024),
            out: vec![0.0; out_cap],
            pcm: Vec::with_capacity(16 * 1024),
            splitter: FrameSplitter::new(),
        })
    }

    fn push(&mut self, data: &[f32], emit: &mut impl FnMut(&AudioFrame)) {
        let Self {
            channels,
            resampler,
            acc,
            out,
            pcm,
            splitter,
        } = self;
        match resampler {
            None => {
                pcm.clear();
                if *channels == 2 {
                    pcm.extend(data.iter().map(|&s| f32_to_i16(s)));
                } else {
                    acc.clear();
                    to_stereo_into(data, *channels, acc);
                    pcm.extend(acc.iter().map(|&s| f32_to_i16(s)));
                    acc.clear();
                }
                splitter.push_i16(pcm, &mut *emit);
            }
            Some(r) => {
                to_stereo_into(data, *channels, acc);
                loop {
                    let need = r.input_frames_next();
                    if acc.len() < need * AUDIO_CHANNELS {
                        break;
                    }
                    let (Ok(input), Ok(output)) = (
                        InterleavedSlice::new(&acc[..need * AUDIO_CHANNELS], AUDIO_CHANNELS, need),
                        {
                            let frames = out.len() / AUDIO_CHANNELS;
                            InterleavedSlice::new_mut(out.as_mut_slice(), AUDIO_CHANNELS, frames)
                        },
                    ) else {
                        acc.clear();
                        return;
                    };
                    let mut output = output;
                    let produced = match r.process_into_buffer(&input, &mut output, None) {
                        Ok((_, produced)) => produced,
                        Err(_) => 0,
                    };
                    acc.drain(..need * AUDIO_CHANNELS);
                    pcm.clear();
                    pcm.extend(
                        out[..produced * AUDIO_CHANNELS]
                            .iter()
                            .map(|&s| f32_to_i16(s)),
                    );
                    splitter.push_i16(pcm, &mut *emit);
                }
            }
        }
    }
}

fn volume_to_bits(v: f32) -> u32 {
    v.clamp(0.0, 1.0).to_bits()
}

/// Preenche `out` (intercalado, `out_channels` canais) com estéreo do ring, aplicando ganho.
/// Falta de dados vira silêncio. Não aloca.
fn fill_output(out: &mut [f32], out_channels: usize, source: &mut rtrb::Consumer<f32>, gain: f32) {
    for frame in out.chunks_exact_mut(out_channels) {
        if source.slots() >= 2 {
            let l = source.pop().unwrap_or(0.0) * gain;
            let r = source.pop().unwrap_or(0.0) * gain;
            frame[0] = l;
            if out_channels == 1 {
                frame[0] = (l + r) * 0.5;
            } else {
                frame[1] = r;
                frame[2..].fill(0.0);
            }
        } else {
            frame.fill(0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Reprodução
// ---------------------------------------------------------------------------

type SharedConsumer = Arc<Mutex<rtrb::Consumer<f32>>>;

pub struct MacAudioPlayback {
    device_id: Option<String>,
    volume: Arc<AtomicU32>,
    source: Option<SharedConsumer>,
    stream: Option<cpal::Stream>,
}

impl std::fmt::Debug for MacAudioPlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacAudioPlayback")
            .field("device_id", &self.device_id)
            .field("running", &self.stream.is_some())
            .finish()
    }
}

impl Default for MacAudioPlayback {
    fn default() -> Self {
        Self {
            device_id: None,
            volume: Arc::new(AtomicU32::new(volume_to_bits(1.0))),
            source: None,
            stream: None,
        }
    }
}

impl MacAudioPlayback {
    fn build(&self, source: &SharedConsumer) -> Result<cpal::Stream, PlatformError> {
        let device = find_device(self.device_id.as_deref(), false)?;
        let supported = {
            let default = device.default_output_config().map_err(err)?;
            if default.sample_rate() == WIRE_RATE
                && default.sample_format() == cpal::SampleFormat::F32
            {
                default
            } else {
                device
                    .supported_output_configs()
                    .map_err(err)?
                    .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
                    .filter_map(|c| c.try_with_sample_rate(WIRE_RATE))
                    .min_by_key(|c| c.channels().abs_diff(2))
                    .ok_or(PlatformError::Unsupported)?
            }
        };
        let config = supported.config();
        let out_channels = usize::from(config.channels);
        let source = Arc::clone(source);
        let volume = Arc::clone(&self.volume);
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _| {
                    if let Ok(mut src) = source.try_lock() {
                        let gain = f32::from_bits(volume.load(Ordering::Relaxed));
                        fill_output(data, out_channels, &mut src, gain);
                    } else {
                        data.fill(0.0);
                    }
                },
                |e| tracing::warn!("áudio (saída): {e}"),
                None,
            )
            .map_err(err)?;
        stream.play().map_err(err)?;
        Ok(stream)
    }
}

impl AudioPlayback for MacAudioPlayback {
    fn start(&mut self, source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        self.stop();
        let source = Arc::new(Mutex::new(source));
        self.stream = Some(self.build(&source)?);
        self.source = Some(source);
        Ok(())
    }

    fn stop(&mut self) {
        self.stream = None;
        self.source = None;
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume.store(volume_to_bits(volume), Ordering::Relaxed);
    }

    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError> {
        let previous = std::mem::replace(&mut self.device_id, device_id.map(str::to_owned));
        let Some(source) = self.source.clone() else {
            return Ok(());
        };
        // Para o stream antigo antes de abrir o novo; se o novo falhar, volta ao anterior.
        self.stream = None;
        match self.build(&source) {
            Ok(stream) => {
                self.stream = Some(stream);
                Ok(())
            }
            Err(e) => {
                self.device_id = previous;
                self.stream = self.build(&source).ok();
                Err(e)
            }
        }
    }

    fn list_devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        list_output_devices()
    }
}

// ---------------------------------------------------------------------------
// Captura
// ---------------------------------------------------------------------------

type SharedProducer = Arc<Mutex<rtrb::Producer<AudioFrame>>>;

#[derive(Default)]
pub struct MacAudioCapture {
    device_id: Option<String>,
    sink: Option<SharedProducer>,
    stream: Option<cpal::Stream>,
}

impl std::fmt::Debug for MacAudioCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacAudioCapture")
            .field("device_id", &self.device_id)
            .field("running", &self.stream.is_some())
            .finish()
    }
}

impl MacAudioCapture {
    fn build(&self, sink: &SharedProducer) -> Result<cpal::Stream, PlatformError> {
        let device = find_device(self.device_id.as_deref(), true)?;
        let supported = device.default_input_config().map_err(err)?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(PlatformError::Unsupported);
        }
        let mut converter =
            CaptureConverter::new(supported.sample_rate(), usize::from(supported.channels()))?;
        let sink = Arc::clone(sink);
        let stream = device
            .build_input_stream(
                supported.config(),
                move |data: &[f32], _| {
                    if let Ok(mut tx) = sink.try_lock() {
                        // Ring cheio: o quadro é descartado (o consumidor está atrasado).
                        converter.push(data, &mut |frame| {
                            let _ = tx.push(*frame);
                        });
                    }
                },
                |e| tracing::warn!("áudio (entrada): {e}"),
                None,
            )
            .map_err(err)?;
        stream.play().map_err(err)?;
        Ok(stream)
    }
}

impl AudioCapture for MacAudioCapture {
    fn start(&mut self, sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        self.stop();
        let sink = Arc::new(Mutex::new(sink));
        self.stream = Some(self.build(&sink)?);
        self.sink = Some(sink);
        Ok(())
    }

    fn stop(&mut self) {
        self.stream = None;
        self.sink = None;
    }

    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError> {
        let previous = std::mem::replace(&mut self.device_id, device_id.map(str::to_owned));
        let Some(sink) = self.sink.clone() else {
            return Ok(());
        };
        self.stream = None;
        match self.build(&sink) {
            Ok(stream) => {
                self.stream = Some(stream);
                Ok(())
            }
            Err(e) => {
                self.device_id = previous;
                self.stream = self.build(&sink).ok();
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_core::platform::AUDIO_FRAME_SAMPLES;

    const FRAME: usize = AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS;

    #[test]
    fn splitter_emite_quadros_exatos_com_callbacks_arbitrarios() {
        let total = FRAME * 7 + 123;
        let input: Vec<i16> = (0..total).map(|i| (i % 30000) as i16).collect();
        for size in [1usize, 2, 7, 100, 479, 480, 481, 1000, 4096] {
            let mut splitter = FrameSplitter::new();
            let mut frames: Vec<AudioFrame> = Vec::new();
            for chunk in input.chunks(size) {
                splitter.push_i16(chunk, |f| frames.push(*f));
            }
            assert_eq!(frames.len(), 7, "callback de {size}");
            assert_eq!(splitter.pending(), 123);
            let joined: Vec<i16> = frames.iter().flatten().copied().collect();
            assert_eq!(joined[..], input[..FRAME * 7], "callback de {size}");
        }
    }

    #[test]
    fn mono_e_multicanal_viram_estereo() {
        let mut out = Vec::new();
        to_stereo_into(&[0.1, 0.2], 1, &mut out);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2]);
        out.clear();
        to_stereo_into(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn f32_para_i16_satura() {
        assert_eq!(f32_to_i16(2.0), 32767);
        assert_eq!(f32_to_i16(-2.0), -32767);
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn conversor_sem_reamostrar_preserva_amostras() {
        let mut conv = CaptureConverter::new(WIRE_RATE, 1).unwrap();
        let mut n = 0;
        // 240 frames mono = 1 quadro estéreo.
        conv.push(&vec![0.5; AUDIO_FRAME_SAMPLES], &mut |f| {
            assert!(f.iter().all(|&s| s == f32_to_i16(0.5)));
            n += 1;
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn conversor_reamostra_44100_para_48000() {
        let mut conv = CaptureConverter::new(44_100, 2).unwrap();
        let seconds = 2usize;
        let total_in = 44_100 * seconds;
        let mut frames = 0usize;
        let mut peak = 0i16;
        let mut phase = 0.0f32;
        let mut buf = Vec::new();
        let mut done = 0;
        while done < total_in {
            let n = 441.min(total_in - done);
            buf.clear();
            for _ in 0..n {
                let s = (phase).sin() * 0.5;
                phase += 2.0 * std::f32::consts::PI * 440.0 / 44_100.0;
                buf.extend([s, s]);
            }
            conv.push(&buf, &mut |f| {
                frames += 1;
                peak = peak.max(f.iter().copied().max().unwrap_or(0));
            });
            done += n;
        }
        let expected = 48_000 * seconds / AUDIO_FRAME_SAMPLES;
        assert!(
            frames + 4 >= expected && frames <= expected,
            "{frames} quadros, esperado ~{expected}"
        );
        assert!((15_000..17_500).contains(&peak), "pico {peak}");
    }

    #[test]
    fn fill_output_aplica_ganho_e_silencio_no_underrun() {
        let (mut tx, mut rx) = rtrb::RingBuffer::<f32>::new(16);
        for s in [1.0, -1.0, 0.5, 0.5] {
            tx.push(s).unwrap();
        }
        let mut out = [9.0f32; 6];
        fill_output(&mut out, 2, &mut rx, 0.5);
        assert_eq!(out, [0.5, -0.5, 0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn fill_output_mapeia_para_mais_canais() {
        let (mut tx, mut rx) = rtrb::RingBuffer::<f32>::new(4);
        tx.push(1.0).unwrap();
        tx.push(2.0).unwrap();
        let mut out = [9.0f32; 4];
        fill_output(&mut out, 4, &mut rx, 1.0);
        assert_eq!(out, [1.0, 2.0, 0.0, 0.0]);
    }

    /// Depende de dispositivo de áudio: não roda em runner de CI sem placa de som.
    #[test]
    #[ignore = "precisa de dispositivo de áudio (rode com --ignored neste Mac)"]
    fn list_devices_tem_ao_menos_uma_saida() {
        let d = list_devices().unwrap();
        assert!(!d.outputs.is_empty());
        assert!(d.outputs.iter().any(|o| o.is_default));
    }
}
