//! dono: W4b
//! Conversões puras de áudio (f32→i16, downmix, FrameSplitter), testadas no Mac.
//!
//! O WASAPI entrega f32 intercalado em N canais; o Relay trafega quadros de 5 ms (240 amostras
//! por canal) i16 estéreo a 48 kHz (`AudioFrame`). Nada aqui aloca depois de construído.

use relay_core::platform::{AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES, AudioFrame};

/// Ganho de centro/traseiras no downmix ITU-R BS.775 (-3 dB).
const MIX: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// f32 (-1.0..=1.0) → i16 com saturação. NaN vira 0.
pub fn f32_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Reduz um quadro intercalado (uma amostra por canal) a `(esquerda, direita)`.
///
/// Layouts WASAPI: 1 (mono duplicado), 2 (direto), 6 = 5.1 (FL FR FC LFE BL BR) e
/// 8 = 7.1 (FL FR FC LFE BL BR SL SR) com coeficientes ITU e LFE descartado. Sem normalização:
/// a soma pode passar de 1.0 e satura em `f32_to_i16`. Outras contagens usam os dois primeiros
/// canais.
pub fn downmix_frame(ch: &[f32]) -> (f32, f32) {
    match ch.len() {
        0 => (0.0, 0.0),
        1 => (ch[0], ch[0]),
        6 => (
            ch[0] + MIX * ch[2] + MIX * ch[4],
            ch[1] + MIX * ch[2] + MIX * ch[5],
        ),
        8 => (
            ch[0] + MIX * ch[2] + MIX * ch[4] + MIX * ch[6],
            ch[1] + MIX * ch[2] + MIX * ch[5] + MIX * ch[7],
        ),
        _ => (ch[0], ch[1]),
    }
}

/// Acumula amostras i16 estéreo intercaladas e emite quadros de exatamente `AudioFrame`.
#[derive(Debug, Clone)]
pub struct FrameSplitter {
    buf: AudioFrame,
    len: usize,
}

impl Default for FrameSplitter {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSplitter {
    pub fn new() -> Self {
        Self {
            buf: [0; AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS],
            len: 0,
        }
    }

    /// Amostras (não pares) acumuladas e ainda não emitidas.
    pub fn pending(&self) -> usize {
        self.len
    }

    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Empurra i16 estéreo intercalado (`L R L R …`); amostra ímpar sobrando fica pendente.
    pub fn push_i16(&mut self, samples: &[i16], mut emit: impl FnMut(&AudioFrame)) {
        for &s in samples {
            self.push_sample(s, &mut emit);
        }
    }

    /// Empurra f32 intercalado com `channels` canais, fazendo downmix para estéreo e
    /// convertendo para i16. `channels == 0` é ignorado.
    pub fn push_f32_interleaved(
        &mut self,
        data: &[f32],
        channels: usize,
        mut emit: impl FnMut(&AudioFrame),
    ) {
        if channels == 0 {
            return;
        }
        for frame in data.chunks_exact(channels) {
            let (l, r) = downmix_frame(frame);
            self.push_sample(f32_to_i16(l), &mut emit);
            self.push_sample(f32_to_i16(r), &mut emit);
        }
    }

    fn push_sample(&mut self, s: i16, emit: &mut impl FnMut(&AudioFrame)) {
        self.buf[self.len] = s;
        self.len += 1;
        if self.len == self.buf.len() {
            self.len = 0;
            emit(&self.buf);
        }
    }
}

/// Preenche `out` (bytes f32 little-endian, estéreo intercalado) com amostras de `next`
/// multiplicadas por `gain`; o que faltar vira silêncio (underrun). `out.len()` deve ser
/// múltiplo de 4. Devolve quantas amostras reais foram consumidas de `next`.
pub fn fill_f32_le(out: &mut [u8], gain: f32, mut next: impl FnMut() -> Option<f32>) -> usize {
    let mut taken = 0;
    let mut dry = false;
    for dst in out.chunks_exact_mut(4) {
        let v = if dry {
            0.0
        } else {
            match next() {
                Some(v) => {
                    taken += 1;
                    v * gain
                }
                None => {
                    dry = true;
                    0.0
                }
            }
        };
        dst.copy_from_slice(&v.to_le_bytes());
    }
    taken
}

/// Decodifica bytes f32 little-endian para `out` (limpa antes). Bytes sobrando (<4) são ignorados.
pub fn f32_from_le(bytes: &[u8], out: &mut Vec<f32>) {
    out.clear();
    out.extend(
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS;

    #[test]
    fn f32_to_i16_saturates_and_rounds() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(1.0), 32767);
        assert_eq!(f32_to_i16(-1.0), -32767);
        assert_eq!(f32_to_i16(2.5), 32767);
        assert_eq!(f32_to_i16(-9.0), -32767);
        assert_eq!(f32_to_i16(0.5), 16384);
        assert_eq!(f32_to_i16(f32::NAN), 0);
    }

    #[test]
    fn downmix_mono_stereo_passthrough() {
        assert_eq!(downmix_frame(&[0.25]), (0.25, 0.25));
        assert_eq!(downmix_frame(&[0.1, -0.2]), (0.1, -0.2));
        assert_eq!(downmix_frame(&[]), (0.0, 0.0));
        assert_eq!(downmix_frame(&[1.0, 2.0, 3.0, 4.0]), (1.0, 2.0));
    }

    #[test]
    fn downmix_5_1() {
        // FL FR FC LFE BL BR
        let (l, r) = downmix_frame(&[0.1, 0.2, 0.4, 0.9, 0.2, 0.3]);
        assert!((l - (0.1 + MIX * 0.4 + MIX * 0.2)).abs() < 1e-6);
        assert!((r - (0.2 + MIX * 0.4 + MIX * 0.3)).abs() < 1e-6);
        // LFE não contribui
        let (l2, _) = downmix_frame(&[0.1, 0.2, 0.4, 0.0, 0.2, 0.3]);
        assert_eq!(l, l2);
    }

    #[test]
    fn downmix_7_1() {
        // FL FR FC LFE BL BR SL SR
        let (l, r) = downmix_frame(&[0.1, 0.2, 0.4, 0.9, 0.2, 0.3, 0.05, 0.06]);
        assert!((l - (0.1 + MIX * (0.4 + 0.2 + 0.05))).abs() < 1e-6);
        assert!((r - (0.2 + MIX * (0.4 + 0.3 + 0.06))).abs() < 1e-6);
    }

    #[test]
    fn splitter_emits_exact_frames_across_pushes() {
        let mut sp = FrameSplitter::new();
        let mut frames: Vec<Vec<i16>> = vec![];
        let input: Vec<i16> = (0..(N * 2 + 10) as i32).map(|i| i as i16).collect();
        for chunk in input.chunks(333) {
            sp.push_i16(chunk, |f| frames.push(f.to_vec()));
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(sp.pending(), 10);
        assert_eq!(frames[0].len(), N);
        assert_eq!(frames[0][0], 0);
        assert_eq!(frames[1][0], N as i16);
        assert_eq!(frames[1][N - 1], (2 * N - 1) as i16);
        sp.reset();
        assert_eq!(sp.pending(), 0);
    }

    #[test]
    fn splitter_f32_stereo_and_5_1() {
        let mut sp = FrameSplitter::new();
        let mut count = 0;
        let mut first = [0i16; 2];
        let stereo: Vec<f32> = (0..AUDIO_FRAME_SAMPLES).flat_map(|_| [0.5, -0.5]).collect();
        sp.push_f32_interleaved(&stereo, 2, |f| {
            count += 1;
            first = [f[0], f[1]];
        });
        assert_eq!(count, 1);
        assert_eq!(first, [16384, -16384]);
        assert_eq!(sp.pending(), 0);

        let surround: Vec<f32> = (0..AUDIO_FRAME_SAMPLES)
            .flat_map(|_| [0.1, 0.1, 0.0, 0.0, 0.0, 0.0])
            .collect();
        sp.push_f32_interleaved(&surround, 6, |f| {
            count += 1;
            first = [f[0], f[1]];
        });
        assert_eq!(count, 2);
        assert_eq!(first, [f32_to_i16(0.1), f32_to_i16(0.1)]);
    }

    #[test]
    fn splitter_f32_zero_channels_and_partial_frame_ignored() {
        let mut sp = FrameSplitter::new();
        sp.push_f32_interleaved(&[0.1, 0.2], 0, |_| panic!());
        // 5 amostras em 2 canais: o último f32 incompleto é descartado
        sp.push_f32_interleaved(&[0.1; 5], 2, |_| panic!());
        assert_eq!(sp.pending(), 4);
    }

    #[test]
    fn fill_f32_le_gain_and_underrun() {
        let mut src = vec![1.0f32, -1.0, 0.5].into_iter();
        let mut out = [0xFFu8; 20];
        let taken = fill_f32_le(&mut out, 0.5, || src.next());
        assert_eq!(taken, 3);
        let mut v = vec![];
        f32_from_le(&out, &mut v);
        assert_eq!(v, vec![0.5, -0.5, 0.25, 0.0, 0.0]);
    }
}
