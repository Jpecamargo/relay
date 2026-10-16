//! dono: C2
//! Pareamento SPAKE2, handshake Noise KK, derivação de chaves UDP, AEAD do UDP e KeyStore.
//! Stubs do F2: assinaturas; o corpo é do C2.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::platform::{KeyStore, KeyStoreError};

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("código de pareamento incorreto")]
    CodeMismatch,
    #[error("não implementado")]
    NotImplemented,
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("não implementado")]
    NotImplemented,
    #[error("autenticação falhou")]
    Auth,
    #[error("replay")]
    Replay,
    #[error("{0}")]
    Other(String),
}

/// Código de pareamento de 6 dígitos ASCII.
pub fn generate_pairing_code() -> String {
    "000000".to_string()
}

/// Chaves do canal UDP (uma por direção).
#[derive(Debug, Clone, Default)]
pub struct UdpKeys {
    pub send: [u8; 32],
    pub recv: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpChannel {
    Input,
    AudioPcToMac,
    AudioMacToPc,
}

/// HKDF-SHA256 do hash do handshake Noise, separando por canal e direção.
pub fn derive_udp_keys(_handshake_hash: &[u8], _channel: UdpChannel, _is_dialer: bool) -> UdpKeys {
    UdpKeys::default()
}

/// ChaCha20-Poly1305 do UDP: nonce = direção + seq; o cabeçalho de 17 bytes é o AAD.
#[derive(Debug)]
pub struct UdpAead;

impl UdpAead {
    pub fn new(_keys: UdpKeys) -> Self {
        Self
    }

    pub fn seal(
        &self,
        _seq: u64,
        _header: &[u8],
        _plain: &[u8],
        _out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        Err(CryptoError::NotImplemented)
    }

    pub fn open(
        &mut self,
        _seq: u64,
        _header: &[u8],
        _cipher: &[u8],
        _out: &mut Vec<u8>,
    ) -> Result<(), CryptoError> {
        Err(CryptoError::NotImplemented)
    }
}

/// KeyStore em memória, para testes.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    map: Mutex<HashMap<String, Vec<u8>>>,
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, _account: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        let _ = &self.map;
        Ok(None)
    }

    fn set(&self, _account: &str, _secret: &[u8]) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::Backend("não implementado".into()))
    }

    fn delete(&self, _account: &str) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::Backend("não implementado".into()))
    }
}

/// KeyStore do sistema (crate `keyring`, service `com.joaocamargo.relay`).
#[derive(Debug, Default)]
pub struct SystemKeyStore;

impl KeyStore for SystemKeyStore {
    fn get(&self, _account: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        Ok(None)
    }

    fn set(&self, _account: &str, _secret: &[u8]) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::Backend("não implementado".into()))
    }

    fn delete(&self, _account: &str) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::Backend("não implementado".into()))
    }
}
