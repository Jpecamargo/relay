//! dono: C1
//! Codec do protocolo: pacote rápido de layout fixo (sem alocação), frames confiáveis com
//! prefixo de tamanho e filtro de sequência.
//!
//! Layout do pacote rápido (little-endian): `kind u8 | seq u64 | ts_us u64` (17 bytes) seguido do
//! corpo. MouseMove (`kind` 1): `dx i16, dy i16` (21 bytes). Scroll (`kind` 2): `dx i16, dy i16`
//! em 1/120 de degrau + `phase u8` (22 bytes).

use crate::types::consts::MAX_FRAME;
use crate::types::{Message, ScrollPhase};

/// Cabeçalho do pacote rápido: kind u8 + seq u64 LE + ts_us u64 LE.
pub const FAST_HEADER_LEN: usize = 17;
/// Tamanho do buffer de um pacote rápido, sem a tag AEAD de 16 bytes.
pub const FAST_PACKET_MAX: usize = 32;
/// Tamanho total de um MouseMove no fio.
pub const MOUSE_MOVE_LEN: usize = FAST_HEADER_LEN + 4;
/// Tamanho total de um Scroll no fio.
pub const SCROLL_LEN: usize = FAST_HEADER_LEN + 5;

const KIND_MOUSE_MOVE: u8 = 1;
const KIND_SCROLL: u8 = 2;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("não implementado")]
    NotImplemented,
    #[error("tamanho inválido para o kind: {0} bytes")]
    BadLength(usize),
    #[error("fase de scroll inválida: {0}")]
    BadPhase(u8),
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
    pub fn encode(&self, buf: &mut [u8; FAST_PACKET_MAX]) -> Result<usize, ProtocolError> {
        let (kind, len) = match self.body {
            FastBody::MouseMove { .. } => (KIND_MOUSE_MOVE, MOUSE_MOVE_LEN),
            FastBody::Scroll { .. } => (KIND_SCROLL, SCROLL_LEN),
        };
        buf[0] = kind;
        buf[1..9].copy_from_slice(&self.seq.to_le_bytes());
        buf[9..17].copy_from_slice(&self.ts_us.to_le_bytes());
        match self.body {
            FastBody::MouseMove { dx, dy } => {
                buf[17..19].copy_from_slice(&dx.to_le_bytes());
                buf[19..21].copy_from_slice(&dy.to_le_bytes());
            }
            FastBody::Scroll { dx, dy, phase } => {
                buf[17..19].copy_from_slice(&dx.to_le_bytes());
                buf[19..21].copy_from_slice(&dy.to_le_bytes());
                buf[21] = phase.as_u8();
            }
        }
        Ok(len)
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtocolError> {
        let Some(&kind) = buf.first() else {
            return Err(ProtocolError::Truncated);
        };
        let expected = match kind {
            KIND_MOUSE_MOVE => MOUSE_MOVE_LEN,
            KIND_SCROLL => SCROLL_LEN,
            other => return Err(ProtocolError::UnknownKind(other)),
        };
        if buf.len() < expected {
            return Err(ProtocolError::Truncated);
        }
        if buf.len() > expected {
            return Err(ProtocolError::BadLength(buf.len()));
        }
        let u64_at = |i: usize| u64::from_le_bytes(buf[i..i + 8].try_into().expect("8 bytes"));
        let i16_at = |i: usize| i16::from_le_bytes([buf[i], buf[i + 1]]);
        let body = if kind == KIND_MOUSE_MOVE {
            FastBody::MouseMove {
                dx: i16_at(17),
                dy: i16_at(19),
            }
        } else {
            FastBody::Scroll {
                dx: i16_at(17),
                dy: i16_at(19),
                phase: ScrollPhase::from_u8(buf[21]).ok_or(ProtocolError::BadPhase(buf[21]))?,
            }
        };
        Ok(Self {
            seq: u64_at(1),
            ts_us: u64_at(9),
            body,
        })
    }
}

/// Aceita só `seq` maior que a última aceita (buracos são permitidos).
#[derive(Debug, Default)]
pub struct SeqFilter {
    last: Option<u64>,
}

impl SeqFilter {
    pub fn accept(&mut self, seq: u64) -> bool {
        if self.last.is_some_and(|last| seq <= last) {
            return false;
        }
        self.last = Some(seq);
        true
    }
}

fn bincode_config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<MAX_FRAME>()
}

/// Escreve `Message` como frame: u32 LE de tamanho + bincode.
#[derive(Debug, Default)]
pub struct FrameWriter;

impl FrameWriter {
    /// Acrescenta o frame a `out`. Em erro, `out` fica como estava.
    pub fn encode(&self, msg: &Message, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        let payload =
            match bincode::serde::encode_into_std_write(msg, &mut VecWriter(out), bincode_config())
            {
                Ok(n) => n,
                Err(e) => {
                    out.truncate(start);
                    return Err(ProtocolError::Codec(e.to_string()));
                }
            };
        if payload > MAX_FRAME {
            out.truncate(start);
            return Err(ProtocolError::FrameTooLarge(payload));
        }
        out[start..start + 4].copy_from_slice(&(payload as u32).to_le_bytes());
        Ok(())
    }
}

struct VecWriter<'a>(&'a mut Vec<u8>);

impl std::io::Write for VecWriter<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Monta frames a partir de bytes de entrada (até byte a byte).
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    /// Alimenta bytes; devolve as mensagens completas montadas. Tamanho declarado acima de
    /// `MAX_FRAME` é erro (o buffer interno nunca passa de `MAX_FRAME + 4`); após erro a conexão
    /// deve ser descartada.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Message>, ProtocolError> {
        let mut out = Vec::new();
        let mut rest = bytes;
        loop {
            // Completa o cabeçalho de 4 bytes antes de decidir o tamanho.
            if self.buf.len() < 4 {
                let take = (4 - self.buf.len()).min(rest.len());
                self.buf.extend_from_slice(&rest[..take]);
                rest = &rest[take..];
                if self.buf.len() < 4 {
                    return Ok(out);
                }
            }
            let len =
                u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            if len > MAX_FRAME {
                self.buf.clear();
                return Err(ProtocolError::FrameTooLarge(len));
            }
            let take = (4 + len - self.buf.len()).min(rest.len());
            self.buf.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.buf.len() < 4 + len {
                return Ok(out);
            }
            let decoded =
                bincode::serde::decode_from_slice::<Message, _>(&self.buf[4..], bincode_config());
            self.buf.clear();
            match decoded {
                Ok((msg, used)) if used == len => out.push(msg),
                Ok((_, used)) => {
                    return Err(ProtocolError::Codec(format!(
                        "frame de {len} bytes com {used} usados"
                    )));
                }
                Err(e) => return Err(ProtocolError::Codec(e.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests;
