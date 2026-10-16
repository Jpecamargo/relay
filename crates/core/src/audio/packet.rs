//! Pacote de áudio: cabeçalho fixo little-endian seguido do payload PCM s16le.

use super::{AudioError, CODEC_PCM16};

/// Tamanho do cabeçalho: stream u8, seq u64, ts_samples u64, codec u8, channels u8, frames u16.
pub const HEADER_LEN: usize = 1 + 8 + 8 + 1 + 1 + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioHeader {
    pub stream: u8,
    pub seq: u64,
    pub ts_samples: u64,
    pub codec: u8,
    pub channels: u8,
    pub frames: u16,
}

impl AudioHeader {
    /// Bytes de payload que este cabeçalho declara (PCM s16le).
    pub fn payload_len(&self) -> usize {
        usize::from(self.frames) * usize::from(self.channels) * 2
    }

    fn validate(&self) -> Result<(), AudioError> {
        if self.codec != CODEC_PCM16 || self.channels == 0 || self.frames == 0 {
            return Err(AudioError::InvalidPacket);
        }
        Ok(())
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(self.stream);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.ts_samples.to_le_bytes());
        out.push(self.codec);
        out.push(self.channels);
        out.extend_from_slice(&self.frames.to_le_bytes());
    }

    /// Lê e valida o cabeçalho e devolve o payload (sem alocar). O buffer deve ter exatamente
    /// cabeçalho + payload.
    pub fn parse(buf: &[u8]) -> Result<(Self, &[u8]), AudioError> {
        if buf.len() < HEADER_LEN {
            return Err(AudioError::InvalidPacket);
        }
        let h = Self {
            stream: buf[0],
            seq: u64::from_le_bytes(
                buf[1..9]
                    .try_into()
                    .map_err(|_| AudioError::InvalidPacket)?,
            ),
            ts_samples: u64::from_le_bytes(
                buf[9..17]
                    .try_into()
                    .map_err(|_| AudioError::InvalidPacket)?,
            ),
            codec: buf[17],
            channels: buf[18],
            frames: u16::from_le_bytes([buf[19], buf[20]]),
        };
        h.validate()?;
        let payload = &buf[HEADER_LEN..];
        if payload.len() != h.payload_len() {
            return Err(AudioError::InvalidPacket);
        }
        Ok((h, payload))
    }
}

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
    pub fn header(&self) -> AudioHeader {
        AudioHeader {
            stream: self.stream,
            seq: self.seq,
            ts_samples: self.ts_samples,
            codec: self.codec,
            channels: self.channels,
            frames: self.frames,
        }
    }

    /// Acrescenta cabeçalho + payload a `out` (não limpa; reserve capacidade para não alocar).
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), AudioError> {
        let h = self.header();
        h.validate()?;
        if self.payload.len() != h.payload_len() {
            return Err(AudioError::InvalidPacket);
        }
        out.reserve(HEADER_LEN + self.payload.len());
        h.write(out);
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    /// Decodifica copiando o payload. No caminho quente prefira [`AudioHeader::parse`].
    pub fn decode(buf: &[u8]) -> Result<Self, AudioError> {
        let (h, payload) = AudioHeader::parse(buf)?;
        Ok(Self {
            stream: h.stream,
            seq: h.seq,
            ts_samples: h.ts_samples,
            codec: h.codec,
            channels: h.channels,
            frames: h.frames,
            payload: payload.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{FRAMES_PER_PACKET, PACKET_PAYLOAD_BYTES};

    fn sample() -> AudioPacket {
        AudioPacket {
            stream: 1,
            seq: 0x0102_0304_0506,
            ts_samples: 960_000,
            codec: CODEC_PCM16,
            channels: 2,
            frames: FRAMES_PER_PACKET as u16,
            payload: (0..PACKET_PAYLOAD_BYTES).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn roundtrip() {
        let p = sample();
        let mut buf = Vec::new();
        p.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), HEADER_LEN + 960);
        assert_eq!(AudioPacket::decode(&buf).unwrap(), p);
        let (h, payload) = AudioHeader::parse(&buf).unwrap();
        assert_eq!(h, p.header());
        assert_eq!(payload, &p.payload[..]);
    }

    #[test]
    fn with_aead_tag_stays_under_1200() {
        let mut buf = Vec::new();
        sample().encode(&mut buf).unwrap();
        assert!(buf.len() + 16 < 1200);
    }

    #[test]
    fn layout_is_little_endian() {
        let mut buf = Vec::new();
        sample().encode(&mut buf).unwrap();
        assert_eq!(buf[0], 1);
        assert_eq!(&buf[1..9], &0x0102_0304_0506u64.to_le_bytes());
        assert_eq!(buf[17], CODEC_PCM16);
        assert_eq!(buf[18], 2);
        assert_eq!(&buf[19..21], &240u16.to_le_bytes());
    }

    #[test]
    fn rejects_invalid() {
        let mut buf = Vec::new();
        sample().encode(&mut buf).unwrap();
        assert!(AudioPacket::decode(&buf[..HEADER_LEN - 1]).is_err());
        assert!(AudioPacket::decode(&buf[..buf.len() - 1]).is_err());
        let mut longer = buf.clone();
        longer.push(0);
        assert!(AudioPacket::decode(&longer).is_err());
        let mut bad_codec = buf.clone();
        bad_codec[17] = 1;
        assert!(AudioPacket::decode(&bad_codec).is_err());
        let mut p = sample();
        p.payload.pop();
        assert!(p.encode(&mut Vec::new()).is_err());
        p = sample();
        p.channels = 0;
        assert!(p.encode(&mut Vec::new()).is_err());
    }
}
