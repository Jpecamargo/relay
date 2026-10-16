//! dono: C1
//! Codec do protocolo: pacote rápido de layout fixo (sem alocação), frames confiáveis com
//! prefixo de tamanho e filtro de sequência. Stubs do F2: assinaturas; o corpo é do C1.

use crate::types::{Message, ScrollPhase};

/// Cabeçalho do pacote rápido: kind u8 + seq u64 LE + ts_us u64 LE.
pub const FAST_HEADER_LEN: usize = 17;
/// Tamanho do buffer de um pacote rápido, sem a tag AEAD de 16 bytes.
pub const FAST_PACKET_MAX: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("não implementado")]
    NotImplemented,
    #[error("buffer truncado")]
    Truncated,
    #[error("kind desconhecido: {0}")]
    UnknownKind(u8),
    #[error("frame maior que MAX_FRAME: {0}")]
    FrameTooLarge(usize),
    #[error("{0}")]
    Codec(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastBody {
    MouseMove {
        dx: i16,
        dy: i16,
    },
    Scroll {
        dx: i16,
        dy: i16,
        phase: ScrollPhase,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastPacket {
    pub seq: u64,
    pub ts_us: u64,
    pub body: FastBody,
}

impl FastPacket {
    /// Codifica em `buf` sem heap e devolve o número de bytes escritos.
    pub fn encode(&self, _buf: &mut [u8; FAST_PACKET_MAX]) -> Result<usize, ProtocolError> {
        Err(ProtocolError::NotImplemented)
    }

    pub fn decode(_buf: &[u8]) -> Result<Self, ProtocolError> {
        Err(ProtocolError::NotImplemented)
    }
}

/// Aceita só `seq` maior que a última aceita.
#[derive(Debug, Default)]
pub struct SeqFilter {
    last: Option<u64>,
}

impl SeqFilter {
    pub fn accept(&mut self, _seq: u64) -> bool {
        let _ = self.last;
        false
    }
}

/// Escreve `Message` como frame: u32 LE de tamanho + bincode.
#[derive(Debug, Default)]
pub struct FrameWriter;

impl FrameWriter {
    pub fn encode(&self, _msg: &Message, _out: &mut Vec<u8>) -> Result<(), ProtocolError> {
        Err(ProtocolError::NotImplemented)
    }
}

/// Monta frames a partir de bytes de entrada (até byte a byte).
#[derive(Debug, Default)]
pub struct FrameReader;

impl FrameReader {
    /// Alimenta bytes; devolve as mensagens completas montadas.
    pub fn push(&mut self, _bytes: &[u8]) -> Result<Vec<Message>, ProtocolError> {
        Err(ProtocolError::NotImplemented)
    }
}
