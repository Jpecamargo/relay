//! Chaves e AEAD dos canais UDP (decisão D6).

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

use super::CryptoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpChannel {
    Input,
    AudioPcToMac,
    AudioMacToPc,
}

impl UdpChannel {
    fn label(self) -> &'static str {
        match self {
            UdpChannel::Input => "input",
            UdpChannel::AudioPcToMac => "audio_pc_to_mac",
            UdpChannel::AudioMacToPc => "audio_mac_to_pc",
        }
    }
}

/// Chaves do canal UDP (uma por direção). `send_dir` (0 = dialer→listener, 1 = o inverso)
/// entra no nonce; a direção de recepção é a oposta.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct UdpKeys {
    pub send: [u8; 32],
    pub recv: [u8; 32],
    pub send_dir: u8,
}

impl std::fmt::Debug for UdpKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpKeys")
            .field("send_dir", &self.send_dir)
            .finish_non_exhaustive()
    }
}

/// HKDF-SHA256 do hash do handshake Noise, separando por canal e direção.
pub fn derive_udp_keys(handshake_hash: &[u8], channel: UdpChannel, is_dialer: bool) -> UdpKeys {
    let hk = Hkdf::<Sha256>::new(Some(b"relay udp v1"), handshake_hash);
    let expand = |dir: &str| {
        let mut k = [0u8; 32];
        let info = format!("relay udp {} {dir}", channel.label());
        hk.expand(info.as_bytes(), &mut k)
            .expect("32 bytes cabem no HKDF-SHA256");
        k
    };
    let d2l = expand("dialer_to_listener");
    let l2d = expand("listener_to_dialer");
    if is_dialer {
        UdpKeys {
            send: d2l,
            recv: l2d,
            send_dir: 0,
        }
    } else {
        UdpKeys {
            send: l2d,
            recv: d2l,
            send_dir: 1,
        }
    }
}

/// Política anti-replay: `Strict` aceita só seq maior que o último (input); `Window64` aceita
/// fora de ordem dentro de uma janela de 64 (áudio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    Strict,
    Window64,
}

#[derive(Debug)]
struct ReplayState {
    mode: ReplayMode,
    seen_any: bool,
    highest: u64,
    /// Bit `i` = `highest - i` já visto.
    bitmap: u64,
}

impl ReplayState {
    fn accepts(&self, seq: u64) -> bool {
        if !self.seen_any || seq > self.highest {
            return true;
        }
        match self.mode {
            ReplayMode::Strict => false,
            ReplayMode::Window64 => {
                let diff = self.highest - seq;
                diff < 64 && self.bitmap & (1 << diff) == 0
            }
        }
    }

    fn commit(&mut self, seq: u64) {
        if !self.seen_any || seq > self.highest {
            let shift = if self.seen_any {
                seq - self.highest
            } else {
                64
            };
            self.bitmap = if shift >= 64 { 0 } else { self.bitmap << shift };
            self.bitmap |= 1;
            self.highest = seq;
            self.seen_any = true;
        } else {
            self.bitmap |= 1 << (self.highest - seq);
        }
    }
}

/// ChaCha20-Poly1305 do UDP: nonce = direção + seq; o cabeçalho de 17 bytes é o AAD.
pub struct UdpAead {
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    send_dir: u8,
    replay: ReplayState,
}

impl std::fmt::Debug for UdpAead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpAead")
            .field("replay", &self.replay)
            .finish_non_exhaustive()
    }
}

fn nonce(dir: u8, seq: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[0] = dir;
    n[4..].copy_from_slice(&seq.to_be_bytes());
    Nonce::from(n)
}

impl UdpAead {
    pub fn new(keys: UdpKeys, mode: ReplayMode) -> Self {
        Self {
            send: ChaCha20Poly1305::new(&keys.send.into()),
            recv: ChaCha20Poly1305::new(&keys.recv.into()),
            send_dir: keys.send_dir,
            replay: ReplayState {
                mode,
                seen_any: false,
                highest: 0,
                bitmap: 0,
            },
        }
    }

    /// Cifra `plain` em `out` (substituindo o conteúdo): texto cifrado + tag de 16 bytes.
    /// Sem alocar se `out` já tem capacidade para `plain.len() + 16`.
    pub fn seal(
        &self,
        seq: u64,
        header: &[u8],
        plain: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        out.clear();
        out.extend_from_slice(plain);
        self.send
            .encrypt_in_place(&nonce(self.send_dir, seq), header, out)
            .map_err(|_| CryptoError::Auth)
    }

    /// Decifra `cipher` (texto + tag) em `out` (substituindo o conteúdo). Rejeita adulteração
    /// (`Auth`) e replay (`Replay`); o estado anti-replay só avança com pacote autêntico.
    pub fn open(
        &mut self,
        seq: u64,
        header: &[u8],
        cipher: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        if !self.replay.accepts(seq) {
            return Err(CryptoError::Replay);
        }
        out.clear();
        out.extend_from_slice(cipher);
        if self
            .recv
            .decrypt_in_place(&nonce(1 - self.send_dir, seq), header, out)
            .is_err()
        {
            out.clear();
            return Err(CryptoError::Auth);
        }
        self.replay.commit(seq);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_alloc::measure;

    const HH: [u8; 32] = [7; 32];
    const HDR: [u8; 17] = [1; 17];

    fn pair(mode: ReplayMode) -> (UdpAead, UdpAead) {
        let d = derive_udp_keys(&HH, UdpChannel::Input, true);
        let l = derive_udp_keys(&HH, UdpChannel::Input, false);
        (UdpAead::new(d, mode), UdpAead::new(l, mode))
    }

    #[test]
    fn both_sides_derive_the_same_and_keys_are_distinct() {
        let chans = [
            UdpChannel::Input,
            UdpChannel::AudioPcToMac,
            UdpChannel::AudioMacToPc,
        ];
        let mut all = Vec::new();
        for c in chans {
            let d = derive_udp_keys(&HH, c, true);
            let l = derive_udp_keys(&HH, c, false);
            assert_eq!(d.send, l.recv);
            assert_eq!(d.recv, l.send);
            assert_ne!(d.send, d.recv);
            all.push(d.send);
            all.push(d.recv);
        }
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
        assert_ne!(
            derive_udp_keys(&[8; 32], UdpChannel::Input, true).send,
            all[0]
        );
    }

    #[test]
    fn roundtrip_both_directions() {
        let (mut d, mut l) = pair(ReplayMode::Strict);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        d.seal(1, &HDR, b"ola", &mut a).unwrap();
        l.open(1, &HDR, &a, &mut b).unwrap();
        assert_eq!(b, b"ola");
        l.seal(1, &HDR, b"volta", &mut a).unwrap();
        d.open(1, &HDR, &a, &mut b).unwrap();
        assert_eq!(b, b"volta");
    }

    #[test]
    fn rejects_tampering_and_wrong_header_or_seq() {
        let (d, mut l) = pair(ReplayMode::Strict);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        d.seal(5, &HDR, b"payload", &mut a).unwrap();
        let mut bad = a.clone();
        bad[2] ^= 1;
        assert!(matches!(
            l.open(5, &HDR, &bad, &mut b),
            Err(CryptoError::Auth)
        ));
        let mut h2 = HDR;
        h2[0] ^= 1;
        assert!(matches!(l.open(5, &h2, &a, &mut b), Err(CryptoError::Auth)));
        assert!(matches!(
            l.open(6, &HDR, &a, &mut b),
            Err(CryptoError::Auth)
        ));
        // pacote adulterado não consome o seq
        l.open(5, &HDR, &a, &mut b).unwrap();
    }

    #[test]
    fn strict_mode_rejects_replay_and_old_seq() {
        let (d, mut l) = pair(ReplayMode::Strict);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        d.seal(10, &HDR, b"x", &mut a).unwrap();
        l.open(10, &HDR, &a, &mut b).unwrap();
        assert!(matches!(
            l.open(10, &HDR, &a, &mut b),
            Err(CryptoError::Replay)
        ));
        d.seal(9, &HDR, b"x", &mut a).unwrap();
        assert!(matches!(
            l.open(9, &HDR, &a, &mut b),
            Err(CryptoError::Replay)
        ));
        d.seal(11, &HDR, b"x", &mut a).unwrap();
        l.open(11, &HDR, &a, &mut b).unwrap();
    }

    #[test]
    fn window_mode_accepts_reorder_within_64_only_once() {
        let (d, mut l) = pair(ReplayMode::Window64);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut send = |seq: u64, l: &mut UdpAead| {
            d.seal(seq, &HDR, b"x", &mut a).unwrap();
            l.open(seq, &HDR, &a, &mut b)
        };
        send(100, &mut l).unwrap();
        send(90, &mut l).unwrap();
        assert!(matches!(send(90, &mut l), Err(CryptoError::Replay)));
        assert!(matches!(send(100, &mut l), Err(CryptoError::Replay)));
        send(37, &mut l).unwrap(); // diff 63
        assert!(matches!(send(36, &mut l), Err(CryptoError::Replay))); // diff 64
        send(300, &mut l).unwrap();
        assert!(matches!(send(100, &mut l), Err(CryptoError::Replay)));
        send(299, &mut l).unwrap();
    }

    #[test]
    fn seal_and_open_do_not_allocate() {
        let (d, mut l) = pair(ReplayMode::Strict);
        let mut a = Vec::with_capacity(256);
        let mut b = Vec::with_capacity(256);
        let plain = [3u8; 64];
        let ((), allocs, _) = measure(|| {
            for seq in 0..100 {
                d.seal(seq, &HDR, &plain, &mut a).unwrap();
                l.open(seq, &HDR, &a, &mut b).unwrap();
            }
        });
        assert_eq!(allocs, 0);
        assert_eq!(b, plain);
    }

    #[test]
    #[ignore = "medição de tempo: rode em release (cargo test --release -- --ignored)"]
    fn seal_open_64_bytes_under_5us() {
        let (d, mut l) = pair(ReplayMode::Strict);
        let mut a = Vec::with_capacity(256);
        let mut b = Vec::with_capacity(256);
        let plain = [3u8; 64];
        let n = 200_000u64;
        let t = std::time::Instant::now();
        for seq in 0..n {
            d.seal(seq, &HDR, &plain, &mut a).unwrap();
            l.open(seq, &HDR, &a, &mut b).unwrap();
        }
        let avg = t.elapsed().as_nanos() as f64 / n as f64;
        assert!(avg < 5_000.0, "média {avg} ns");
    }
}
