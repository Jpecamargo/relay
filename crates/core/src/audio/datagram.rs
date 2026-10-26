//! Datagrama UDP de áudio (microfone do Mac → PC): `hdr17 ++ seal(seq, aad = hdr17, plain)`.
//!
//! `hdr17` = stream u8, seq u64 LE, ts_samples u64 LE (os 17 primeiros bytes do `AudioPacket`,
//! em claro: são o AAD). `plain` = codec u8, channels u8, frames u16 LE e os 960 bytes de PCM
//! s16le (o resto do `AudioPacket`). O nonce usa o `seq`.

use super::{
    CHANNELS, CODEC_PCM16, FRAMES_PER_PACKET, HEADER_LEN, PACKET_PAYLOAD_BYTES, i16_to_pcm_bytes,
};
use crate::crypto::UdpAead;

/// `stream` do cabeçalho: 1 = microfone do Mac → PC.
pub const STREAM_MIC: u8 = 1;

/// Bytes do cabeçalho autenticado (AAD) no fio.
const CLEAR_LEN: usize = 17;
/// Parte cifrada: codec, channels, frames (4 B) + PCM.
pub const PLAIN_LEN: usize = (HEADER_LEN - CLEAR_LEN) + PACKET_PAYLOAD_BYTES;
const TAG_LEN: usize = 16;
/// Tamanho de um datagrama completo no fio.
pub const DATAGRAM_LEN: usize = CLEAR_LEN + PLAIN_LEN + TAG_LEN;

/// Monta o datagrama de um quadro (`frame` = 240 frames estéreo intercalados) em `wire`,
/// substituindo o conteúdo. `plain` e `cipher` são buffers de trabalho do chamador.
pub fn seal_datagram(
    aead: &UdpAead,
    stream: u8,
    seq: u64,
    frame: &[i16],
    plain: &mut Vec<u8>,
    cipher: &mut Vec<u8>,
    wire: &mut Vec<u8>,
) -> bool {
    let mut hdr = [0u8; CLEAR_LEN];
    hdr[0] = stream;
    hdr[1..9].copy_from_slice(&seq.to_le_bytes());
    hdr[9..17].copy_from_slice(&(seq * FRAMES_PER_PACKET as u64).to_le_bytes());
    plain.clear();
    plain.push(CODEC_PCM16);
    plain.push(CHANNELS as u8);
    plain.extend_from_slice(&(FRAMES_PER_PACKET as u16).to_le_bytes());
    let start = plain.len();
    plain.resize(start + PACKET_PAYLOAD_BYTES, 0);
    if i16_to_pcm_bytes(frame, &mut plain[start..]).is_err() {
        return false;
    }
    if aead.seal(seq, &hdr, plain, cipher).is_err() {
        return false;
    }
    wire.clear();
    wire.extend_from_slice(&hdr);
    wire.extend_from_slice(cipher);
    true
}

/// Confere (tamanho, `stream`, autenticação, replay, codec/canais/frames) e decifra um datagrama.
/// Devolve `(seq, payload PCM)`, apontando para `plain` (buffer do chamador).
pub fn open_datagram<'a>(
    aead: &mut UdpAead,
    wire: &[u8],
    stream: u8,
    plain: &'a mut Vec<u8>,
) -> Option<(u64, &'a [u8])> {
    if wire.len() != DATAGRAM_LEN || wire[0] != stream {
        return None;
    }
    let (hdr, cipher) = wire.split_at(CLEAR_LEN);
    let seq = u64::from_le_bytes(hdr[1..9].try_into().ok()?);
    aead.open(seq, hdr, cipher, plain).ok()?;
    let (head, payload) = plain.split_at_checked(HEADER_LEN - CLEAR_LEN)?;
    let frames = u16::from_le_bytes([head[2], head[3]]);
    if head[0] != CODEC_PCM16
        || usize::from(head[1]) != CHANNELS
        || usize::from(frames) != FRAMES_PER_PACKET
        || payload.len() != PACKET_PAYLOAD_BYTES
    {
        return None;
    }
    Some((seq, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{ReplayMode, UdpChannel, derive_udp_keys};

    fn pair() -> (UdpAead, UdpAead) {
        let hh = [7u8; 32];
        let k = |dialer| derive_udp_keys(&hh, UdpChannel::AudioMacToPc, dialer);
        (
            UdpAead::new(k(false), ReplayMode::Window64),
            UdpAead::new(k(true), ReplayMode::Window64),
        )
    }

    #[test]
    fn roundtrip_e_stream_conferido() {
        let (tx, mut rx) = pair();
        let frame: Vec<i16> = (0..FRAMES_PER_PACKET * CHANNELS)
            .map(|i| i as i16)
            .collect();
        let (mut p, mut c, mut w) = (Vec::new(), Vec::new(), Vec::new());
        assert!(seal_datagram(
            &tx, STREAM_MIC, 5, &frame, &mut p, &mut c, &mut w
        ));
        assert_eq!(w.len(), DATAGRAM_LEN);
        // stream errado é descartado antes de decifrar (e não consome o seq).
        assert!(open_datagram(&mut rx, &w, 0, &mut p).is_none());
        let (seq, pcm) = open_datagram(&mut rx, &w, STREAM_MIC, &mut p).unwrap();
        assert_eq!(seq, 5);
        assert_eq!(pcm.len(), PACKET_PAYLOAD_BYTES);
        assert_eq!(&pcm[..4], &[0, 0, 1, 0]);
        // replay e adulteração
        assert!(open_datagram(&mut rx, &w, STREAM_MIC, &mut p).is_none());
        w[30] ^= 1;
        assert!(open_datagram(&mut rx, &w, STREAM_MIC, &mut p).is_none());
    }
}
