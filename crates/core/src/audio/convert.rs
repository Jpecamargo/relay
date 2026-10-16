//! Conversões de formato sobre buffers do chamador (sem alocação).

use super::AudioError;
use crate::platform::{AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES, AudioFrame};

fn same_len(a: usize, b: usize) -> Result<(), AudioError> {
    if a == b {
        Ok(())
    } else {
        Err(AudioError::BadBufferSize)
    }
}

/// f32 [-1, 1] → i16, com saturação. Inverso exato de [`i16_to_f32`].
pub fn f32_to_i16(src: &[f32], dst: &mut [i16]) -> Result<(), AudioError> {
    same_len(src.len(), dst.len())?;
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
    }
    Ok(())
}

pub fn i16_to_f32(src: &[i16], dst: &mut [f32]) -> Result<(), AudioError> {
    same_len(src.len(), dst.len())?;
    for (d, s) in dst.iter_mut().zip(src) {
        *d = f32::from(*s) / 32768.0;
    }
    Ok(())
}

/// Duplica cada amostra mono em L e R (`dst.len() == 2 * src.len()`).
pub fn mono_to_stereo<T: Copy>(src: &[T], dst: &mut [T]) -> Result<(), AudioError> {
    same_len(src.len() * 2, dst.len())?;
    for (d, s) in dst.chunks_exact_mut(2).zip(src) {
        d[0] = *s;
        d[1] = *s;
    }
    Ok(())
}

/// Média de L e R (`src.len() == 2 * dst.len()`).
pub fn stereo_to_mono_i16(src: &[i16], dst: &mut [i16]) -> Result<(), AudioError> {
    same_len(src.len(), dst.len() * 2)?;
    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
        *d = ((i32::from(s[0]) + i32::from(s[1])) >> 1) as i16;
    }
    Ok(())
}

pub fn stereo_to_mono_f32(src: &[f32], dst: &mut [f32]) -> Result<(), AudioError> {
    same_len(src.len(), dst.len() * 2)?;
    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
        *d = (s[0] + s[1]) * 0.5;
    }
    Ok(())
}

/// Bytes PCM s16le → amostras (`src.len() == 2 * dst.len()`).
pub fn pcm_bytes_to_i16(src: &[u8], dst: &mut [i16]) -> Result<(), AudioError> {
    same_len(src.len(), dst.len() * 2)?;
    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
        *d = i16::from_le_bytes([s[0], s[1]]);
    }
    Ok(())
}

/// Amostras → bytes PCM s16le (`dst.len() == 2 * src.len()`).
pub fn i16_to_pcm_bytes(src: &[i16], dst: &mut [u8]) -> Result<(), AudioError> {
    same_len(src.len() * 2, dst.len())?;
    for (d, s) in dst.chunks_exact_mut(2).zip(src) {
        d.copy_from_slice(&s.to_le_bytes());
    }
    Ok(())
}

/// Uma amostra f32 (-1.0..=1.0) → i16 com saturação, escala 32767 (±1.0 → ±32767; NaN → 0).
/// Diferente de [`f32_to_i16`] (escala 32768, usada no pipeline do core): é a escala que as
/// capturas de plataforma sempre usaram.
pub fn f32_sample_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Acumula estéreo i16 intercalado de tamanho arbitrário e emite quadros de exatamente
/// `AUDIO_FRAME_SAMPLES` amostras por canal (`AudioFrame`). Não aloca depois de construído.
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

    /// Amostras (intercaladas) acumuladas e ainda não emitidas.
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

    /// Empurra uma amostra e emite o quadro quando completa.
    pub fn push_sample(&mut self, s: i16, emit: &mut impl FnMut(&AudioFrame)) {
        self.buf[self.len] = s;
        self.len += 1;
        if self.len == self.buf.len() {
            self.len = 0;
            emit(&self.buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i16_f32_roundtrip_is_exact() {
        let src: Vec<i16> = (-32768..=32767).step_by(7).map(|v| v as i16).collect();
        let mut f = vec![0.0; src.len()];
        let mut back = vec![0; src.len()];
        i16_to_f32(&src, &mut f).unwrap();
        f32_to_i16(&f, &mut back).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn f32_to_i16_saturates() {
        let mut d = [0i16; 4];
        f32_to_i16(&[2.0, -2.0, 1.0, -1.0], &mut d).unwrap();
        assert_eq!(d, [32767, -32768, 32767, -32768]);
    }

    #[test]
    fn sample_scale_32767() {
        assert_eq!(f32_sample_to_i16(1.0), 32767);
        assert_eq!(f32_sample_to_i16(-1.0), -32767);
        assert_eq!(f32_sample_to_i16(-9.0), -32767);
        assert_eq!(f32_sample_to_i16(0.5), 16384);
        assert_eq!(f32_sample_to_i16(f32::NAN), 0);
    }

    #[test]
    fn splitter_emits_exact_frames_across_pushes() {
        const N: usize = AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS;
        let mut sp = FrameSplitter::new();
        let mut frames: Vec<Vec<i16>> = vec![];
        let input: Vec<i16> = (0..(N * 2 + 10) as i32).map(|i| i as i16).collect();
        for chunk in input.chunks(333) {
            sp.push_i16(chunk, |f| frames.push(f.to_vec()));
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(sp.pending(), 10);
        assert_eq!(frames[1][0], N as i16);
        assert_eq!(frames[1][N - 1], (2 * N - 1) as i16);
        sp.reset();
        assert_eq!(sp.pending(), 0);
    }

    #[test]
    fn mono_stereo() {
        let mut st = [0i16; 6];
        mono_to_stereo(&[1i16, -2, 3], &mut st).unwrap();
        assert_eq!(st, [1, 1, -2, -2, 3, 3]);
        let mut mono = [0i16; 3];
        stereo_to_mono_i16(&[10, 20, -5, -5, 32767, 32767], &mut mono).unwrap();
        assert_eq!(mono, [15, -5, 32767]);
        let mut mf = [0f32; 2];
        stereo_to_mono_f32(&[1.0, 0.0, 0.5, 0.25], &mut mf).unwrap();
        assert_eq!(mf, [0.5, 0.375]);
        let mut sf = [0f32; 4];
        mono_to_stereo(&[0.1f32, 0.2], &mut sf).unwrap();
        assert_eq!(sf, [0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn length_mismatch_errors() {
        assert!(f32_to_i16(&[0.0], &mut [0i16; 2]).is_err());
        assert!(mono_to_stereo(&[1i16], &mut [0i16; 3]).is_err());
        assert!(stereo_to_mono_i16(&[1, 2, 3], &mut [0i16; 1]).is_err());
    }

    #[test]
    fn pcm_bytes_roundtrip() {
        let src = [0i16, 1, -1, 32767, -32768];
        let mut bytes = [0u8; 10];
        i16_to_pcm_bytes(&src, &mut bytes).unwrap();
        assert_eq!(&bytes[2..4], &[1, 0]);
        let mut back = [0i16; 5];
        pcm_bytes_to_i16(&bytes, &mut back).unwrap();
        assert_eq!(src, back);
    }
}
