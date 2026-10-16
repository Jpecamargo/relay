//! dono: C12
//! Pipeline de áudio do core: pacote PCM, buffer de jitter, compensação de deriva e conversões.
//! Stubs do F2: assinaturas.

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("não implementado")]
    NotImplemented,
    #[error("pacote inválido")]
    InvalidPacket,
}

/// Codec do campo `codec` do pacote (D13: só PCM).
pub const CODEC_PCM16: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPacket {
    pub stream: u8,
    pub seq: u64,
    pub ts_samples: u64,
    pub codec: u8,
    pub channels: u8,
    pub frames: u16,
    pub payload: Vec<u8>,
}

impl AudioPacket {
    pub fn encode(&self, _out: &mut Vec<u8>) -> Result<(), AudioError> {
        Err(AudioError::NotImplemented)
    }

    pub fn decode(_buf: &[u8]) -> Result<Self, AudioError> {
        Err(AudioError::NotImplemented)
    }
}

/// Buffer de jitter adaptativo (começa em 20 ms, teto 80 ms).
#[derive(Debug, Default)]
pub struct JitterBuffer;

/// Compensação de deriva de relógio por reamostragem (rubato + controle PI).
#[derive(Debug, Default)]
pub struct DriftCompensator;
