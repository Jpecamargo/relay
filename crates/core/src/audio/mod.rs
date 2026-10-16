//! dono: C12
//! Pipeline de áudio do core: pacote PCM, buffer de jitter, compensação de deriva e conversões.
//!
//! Formato fixo (D13): PCM s16le, 48 kHz, estéreo, quadros de 5 ms (240 frames, 960 bytes).
//! Nada aqui aloca depois da construção: o caminho quente (`JitterBuffer::push_payload`/`read`,
//! `DriftCompensator::process`, conversões) trabalha só sobre buffers do chamador.

mod convert;
mod datagram;
mod drift;
mod jitter;
mod packet;

pub use convert::{
    FrameSplitter, f32_sample_to_i16, f32_to_i16, i16_to_f32, i16_to_pcm_bytes, mono_to_stereo,
    pcm_bytes_to_i16, stereo_to_mono_f32, stereo_to_mono_i16,
};
pub use datagram::{DATAGRAM_LEN, PLAIN_LEN, STREAM_MIC, STREAM_PC, open_datagram, seal_datagram};
pub use drift::DriftCompensator;
pub use jitter::{JitterBuffer, JitterStats, ReadStatus};
pub use packet::{AudioHeader, AudioPacket, HEADER_LEN};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("pacote inválido")]
    InvalidPacket,
    #[error("buffer com tamanho incompatível")]
    BadBufferSize,
    #[error("falha no resampler")]
    Resample,
}

/// Codec do campo `codec` do pacote (D13: só PCM).
pub const CODEC_PCM16: u8 = 0;
/// Taxa de amostragem do pipeline.
pub const SAMPLE_RATE: u32 = 48_000;
/// Canais do pipeline (estéreo).
pub const CHANNELS: usize = 2;
/// Frames (amostras por canal) de um quadro de 5 ms.
pub const FRAMES_PER_PACKET: usize = 240;
/// Bytes de payload de um pacote de 5 ms estéreo s16le.
pub const PACKET_PAYLOAD_BYTES: usize = FRAMES_PER_PACKET * CHANNELS * 2;
