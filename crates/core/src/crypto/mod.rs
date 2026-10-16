//! dono: C2
//! Pareamento SPAKE2, handshake Noise KK, derivação de chaves UDP, AEAD do UDP e KeyStore.
//! Só cripto em Rust puro (sem ring/aws-lc).

mod keystore;
mod noise;
mod pairing;
mod udp;

use rand::RngExt;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;

pub use keystore::{
    MemoryKeyStore, StaticKeypair, SystemKeyStore, device_id_of, load_or_create_identity,
    load_peer, remove_peer, save_peer,
};
pub use noise::{NoiseSession, noise_accept, noise_dial, read_frame, write_frame};
pub use pairing::{LocalDevice, pair};
pub use udp::{ReplayMode, UdpAead, UdpChannel, UdpKeys, derive_udp_keys};

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("código de pareamento incorreto")]
    CodeMismatch,
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("autenticação falhou")]
    Auth,
    #[error("replay")]
    Replay,
    #[error("par desconhecido")]
    UnknownPeer,
    #[error("{0}")]
    Other(String),
}

/// Código de pareamento de 6 dígitos ASCII (zeros à esquerda incluídos), do RNG do sistema.
pub fn generate_pairing_code() -> String {
    let mut rng = UnwrapErr(SysRng);
    let n: u32 = rng.random_range(0..1_000_000);
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_has_six_ascii_digits() {
        for _ in 0..1000 {
            let c = generate_pairing_code();
            assert_eq!(c.len(), 6, "{c}");
            assert!(c.bytes().all(|b| b.is_ascii_digit()), "{c}");
        }
    }
}
