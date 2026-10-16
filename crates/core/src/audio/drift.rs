//! Compensação de deriva de relógio: reamostragem (rubato) com a razão ajustada por um controle
//! PI sobre o nível do buffer de jitter.
//!
//! O dispositivo de saída pede `chunk` frames fixos; o compensador consome do buffer de jitter
//! `input_frames_next()` frames (variável) e entrega exatamente `chunk`. O nível a informar é
//! `JitterBuffer::mean_level_frames()` (sem o dente de serra entre chegadas). Nível acima do alvo
//! significa que o dispositivo consome devagar demais: a razão (saída/entrada) cai e entra mais
//! entrada por saída. Abaixo do alvo, o inverso.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler};

use super::{AudioError, CHANNELS};

/// Desvio máximo da razão em relação a 1.0 (±1%, ~10000 ppm; relógios reais ficam em ~100 ppm).
const MAX_RELATIVE: f64 = 1.01;
/// Ganho proporcional (razão por frame de erro de nível suavizado).
const KP: f64 = 3.0e-5;
/// Ganho integral (razão por frame de erro por chamada).
const KI: f64 = 2.0e-9;
/// Suavização do nível medido (o nível oscila ±1 pacote entre chegadas).
const LEVEL_ALPHA: f64 = 0.1;

/// Compensação de deriva de relógio por reamostragem (rubato + controle PI).
pub struct DriftCompensator {
    resampler: Async<f32>,
    chunk: usize,
    smoothed: f64,
    integral: f64,
    primed: bool,
}

impl std::fmt::Debug for DriftCompensator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriftCompensator")
            .field("chunk", &self.chunk)
            .field("ratio", &self.resampler.resample_ratio())
            .finish()
    }
}

impl DriftCompensator {
    /// `chunk_frames`: frames de saída por chamada (o que o dispositivo pede), estéreo.
    pub fn new(chunk_frames: usize) -> Result<Self, AudioError> {
        let resampler = Async::<f32>::new_poly(
            1.0,
            MAX_RELATIVE,
            PolynomialDegree::Cubic,
            chunk_frames,
            CHANNELS,
            FixedAsync::Output,
        )
        .map_err(|_| AudioError::Resample)?;
        Ok(Self {
            resampler,
            chunk: chunk_frames,
            smoothed: 0.0,
            integral: 0.0,
            primed: false,
        })
    }

    /// Razão atual saída/entrada.
    pub fn ratio(&self) -> f64 {
        self.resampler.resample_ratio()
    }

    /// Frames de entrada que a próxima chamada de [`process`](Self::process) consome.
    pub fn input_frames_next(&self) -> usize {
        self.resampler.input_frames_next()
    }

    /// Atualiza o controle PI com o nível do buffer e o alvo (em frames) e ajusta a razão.
    pub fn update(&mut self, level_frames: f32, target_frames: f32) -> Result<(), AudioError> {
        let err = f64::from(level_frames) - f64::from(target_frames);
        if self.primed {
            self.smoothed += LEVEL_ALPHA * (err - self.smoothed);
        } else {
            self.smoothed = err;
            self.primed = true;
        }
        let limit = (MAX_RELATIVE - 1.0) / KI;
        self.integral = (self.integral + self.smoothed).clamp(-limit, limit);
        let rel = (1.0 - (KP * self.smoothed + KI * self.integral))
            .clamp(1.0 / MAX_RELATIVE, MAX_RELATIVE);
        self.resampler
            .set_resample_ratio_relative(rel, true)
            .map_err(|_| AudioError::Resample)
    }

    /// Reamostra `input` (`input_frames_next()` frames estéreo f32 intercalados) para `out`
    /// (`chunk` frames). Sem alocação.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) -> Result<(), AudioError> {
        let frames_in = self.resampler.input_frames_next();
        if input.len() != frames_in * CHANNELS || out.len() != self.chunk * CHANNELS {
            return Err(AudioError::BadBufferSize);
        }
        let a = InterleavedSlice::new(input, CHANNELS, frames_in)
            .map_err(|_| AudioError::BadBufferSize)?;
        let mut b = InterleavedSlice::new_mut(out, CHANNELS, self.chunk)
            .map_err(|_| AudioError::BadBufferSize)?;
        let (_, produced) = self
            .resampler
            .process_into_buffer(&a, &mut b, None)
            .map_err(|_| AudioError::Resample)?;
        if produced != self.chunk {
            return Err(AudioError::Resample);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{
        FRAMES_PER_PACKET, JitterBuffer, PACKET_PAYLOAD_BYTES, ReadStatus, f32_to_i16, i16_to_f32,
    };

    fn payload(seq: u64) -> Vec<u8> {
        let mut v = Vec::with_capacity(PACKET_PAYLOAD_BYTES);
        for f in 0..FRAMES_PER_PACKET as u64 {
            let t = (seq * FRAMES_PER_PACKET as u64 + f) as f64 / 48_000.0;
            let s = ((t * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16;
            for _ in 0..CHANNELS {
                v.extend_from_slice(&s.to_le_bytes());
            }
        }
        v
    }

    /// Emissor a 48000 Hz, dispositivo do receptor a `rx_hz`; devolve (níveis em ms após a
    /// partida, jb).
    fn simulate(
        rx_hz: f64,
        secs: f64,
        chunk: usize,
        compensate: bool,
    ) -> (Vec<f32>, JitterBuffer, DriftCompensator) {
        let mut jb = JitterBuffer::new(20);
        let mut dc = DriftCompensator::new(chunk).unwrap();
        let tx_period = FRAMES_PER_PACKET as f64 / 48_000.0;
        let rx_period = chunk as f64 / rx_hz;
        let (mut next_tx, mut next_rx) = (0.0f64, 0.0f64);
        let mut seq = 0u64;
        let mut levels = Vec::new();
        let mut started = false;
        let mut raw = vec![0i16; 2 * 2 * chunk];
        let mut fin = vec![0f32; 2 * 2 * chunk];
        let mut fout = vec![0f32; 2 * chunk];
        while next_rx < secs {
            if next_tx <= next_rx {
                jb.push_payload(seq, &payload(seq)).unwrap();
                seq += 1;
                next_tx += tx_period;
                continue;
            }
            let n = dc.input_frames_next();
            let level = jb.level_frames();
            if compensate {
                dc.update(jb.mean_level_frames() as f32, jb.target_frames() as f32)
                    .unwrap();
            }
            let n = n.min(2 * chunk);
            let st = jb.read(&mut raw[..n * 2]);
            i16_to_f32(&raw[..n * 2], &mut fin[..n * 2]).unwrap();
            if n == dc.input_frames_next() {
                dc.process(&fin[..n * 2], &mut fout).unwrap();
            }
            let mut o = vec![0i16; fout.len()];
            f32_to_i16(&fout, &mut o).unwrap();
            if !started && st == ReadStatus::Played {
                started = true;
            }
            if started {
                levels.push(level as f32 / 48.0);
            }
            next_rx += rx_period;
        }
        (levels, jb, dc)
    }

    fn assert_within(levels: &[f32], target: f32, limit: f32) {
        let (i, dev) = levels
            .iter()
            .enumerate()
            .map(|(i, l)| (i, (l - target).abs()))
            .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        let lo = i.saturating_sub(4);
        assert!(
            dev <= limit,
            "desvio {dev} ms no tick {i}: {:?}",
            &levels[lo..(i + 4).min(levels.len())]
        );
    }

    #[test]
    fn holds_buffer_within_5ms_with_10hz_drift() {
        // Callbacks menores que o pacote (2,7 e 1,3 ms) e sem alinhar com ele. Com callback maior
        // que 5 ms o próprio nível instantâneo oscila mais que ±5 ms, e o alvo precisa subir.
        for chunk in [128, 64] {
            let (levels, jb, dc) = simulate(48_010.0, 60.0, chunk, true);
            assert_within(&levels, jb.target_ms() as f32, 5.0);
            let st = jb.stats();
            assert_eq!((st.underruns, st.lost), (0, 0));
            // A razão acompanha a deriva real (48010/48000).
            assert!(
                (dc.ratio() - 48_010.0 / 48_000.0).abs() < 2e-4,
                "razão {}",
                dc.ratio()
            );
        }
    }

    #[test]
    fn holds_buffer_with_negative_drift_too() {
        let (levels, jb, dc) = simulate(47_990.0, 60.0, 128, true);
        assert_within(&levels, jb.target_ms() as f32, 5.0);
        assert_eq!(jb.stats().underruns, 0);
        assert!(
            (dc.ratio() - 47_990.0 / 48_000.0).abs() < 2e-4,
            "razão {}",
            dc.ratio()
        );
    }

    #[test]
    fn without_compensation_the_buffer_drifts_out() {
        // Sanidade da simulação: com a razão travada em 1.0 o buffer foge de ±5 ms em 60 s.
        let (levels, jb, _) = simulate(48_010.0, 60.0, 128, false);
        let target = jb.target_ms() as f32;
        assert!(levels.iter().any(|l| (l - target).abs() > 5.0));
    }

    #[test]
    fn process_checks_sizes() {
        let mut dc = DriftCompensator::new(240).unwrap();
        assert!(dc.process(&[0.0; 4], &mut [0.0; 480]).is_err());
    }
}
