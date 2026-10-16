//! KeyStore (Keychain / Credential Manager), identidade X25519 e pares salvos.

use std::collections::HashMap;
use std::sync::Mutex;

use blake2::{Blake2s256, Digest};

use crate::platform::{KeyStore, KeyStoreError};
use crate::types::PairedPeer;
use crate::types::consts::KEYCHAIN_SERVICE;

const IDENTITY_ACCOUNT: &str = "identity";

/// Par de chaves X25519 estático do dispositivo.
#[derive(Clone, PartialEq, Eq)]
pub struct StaticKeypair {
    pub private: [u8; 32],
    pub public: [u8; 32],
}

impl std::fmt::Debug for StaticKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticKeypair")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

/// `device_id` = hex dos 8 primeiros bytes do BLAKE2s da chave pública.
pub fn device_id_of(public_key: &[u8; 32]) -> String {
    let h = Blake2s256::digest(public_key);
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn backend(e: impl std::fmt::Display) -> KeyStoreError {
    KeyStoreError::Backend(e.to_string())
}

/// Lê a identidade da conta `identity` ou gera e salva uma nova (uma vez só).
pub fn load_or_create_identity(store: &dyn KeyStore) -> Result<StaticKeypair, KeyStoreError> {
    if let Some(bytes) = store.get(IDENTITY_ACCOUNT)? {
        let private: [u8; 32] = bytes
            .try_into()
            .map_err(|_| backend("identidade com tamanho inválido"))?;
        let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private));
        return Ok(StaticKeypair {
            private,
            public: public.to_bytes(),
        });
    }
    let params = "Noise_KK_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(backend)?;
    let kp = snow::Builder::new(params)
        .generate_keypair()
        .map_err(backend)?;
    let private: [u8; 32] = kp
        .private
        .try_into()
        .map_err(|_| backend("chave privada"))?;
    let public: [u8; 32] = kp.public.try_into().map_err(|_| backend("chave pública"))?;
    store.set(IDENTITY_ACCOUNT, &private)?;
    Ok(StaticKeypair { private, public })
}

fn peer_account(device_id: &str) -> String {
    format!("peer:{device_id}")
}

pub fn save_peer(store: &dyn KeyStore, peer: &PairedPeer) -> Result<(), KeyStoreError> {
    let bytes =
        bincode::serde::encode_to_vec(peer, bincode::config::standard()).map_err(backend)?;
    store.set(&peer_account(&peer.device_id), &bytes)
}

pub fn load_peer(
    store: &dyn KeyStore,
    device_id: &str,
) -> Result<Option<PairedPeer>, KeyStoreError> {
    let Some(bytes) = store.get(&peer_account(device_id))? else {
        return Ok(None);
    };
    let (peer, _) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).map_err(backend)?;
    Ok(Some(peer))
}

pub fn remove_peer(store: &dyn KeyStore, device_id: &str) -> Result<(), KeyStoreError> {
    store.delete(&peer_account(device_id))
}

/// Grava o peer e o marca como ativo: `active_account` guarda o `device_id` dele. O nome da conta
/// é de cada app (`active_peer` no Mac, `active-peer` no agente) e não pode mudar, senão o
/// pareamento já gravado some.
pub fn save_active_peer(
    store: &dyn KeyStore,
    active_account: &str,
    peer: &PairedPeer,
) -> Result<(), KeyStoreError> {
    save_peer(store, peer)?;
    store.set(active_account, peer.device_id.as_bytes())
}

/// Lê o peer marcado como ativo em `active_account` (`None` sem conta ou sem peer salvo).
pub fn load_active_peer(
    store: &dyn KeyStore,
    active_account: &str,
) -> Result<Option<PairedPeer>, KeyStoreError> {
    match store.get(active_account)? {
        Some(id) => load_peer(store, &String::from_utf8_lossy(&id)),
        None => Ok(None),
    }
}

/// Esquece o peer ativo: remove o peer e a conta que o aponta. Tenta os dois mesmo se o primeiro
/// falhar e devolve o primeiro erro.
pub fn forget_active_peer(
    store: &dyn KeyStore,
    active_account: &str,
    device_id: &str,
) -> Result<(), KeyStoreError> {
    let peer = remove_peer(store, device_id);
    let account = store.delete(active_account);
    peer.and(account)
}

/// KeyStore em memória, para testes.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    map: Mutex<HashMap<String, Vec<u8>>>,
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        Ok(self.map.lock().map_err(backend)?.get(account).cloned())
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        self.map
            .lock()
            .map_err(backend)?
            .insert(account.to_string(), secret.to_vec());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), KeyStoreError> {
        self.map.lock().map_err(backend)?.remove(account);
        Ok(())
    }
}

/// KeyStore do sistema (crate `keyring`, service `com.joaocamargo.relay`).
#[derive(Debug, Default)]
pub struct SystemKeyStore;

fn entry(account: &str) -> Result<keyring::Entry, KeyStoreError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(backend)
}

impl KeyStore for SystemKeyStore {
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, KeyStoreError> {
        match entry(account)?.get_secret() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(backend(e)),
        }
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<(), KeyStoreError> {
        entry(account)?.set_secret(secret).map_err(backend)
    }

    fn delete(&self, account: &str) -> Result<(), KeyStoreError> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(backend(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_generated_once_and_reused() {
        let s = MemoryKeyStore::default();
        let a = load_or_create_identity(&s).unwrap();
        let b = load_or_create_identity(&s).unwrap();
        assert_eq!(a, b);
        assert_ne!(a.private, [0; 32]);
        assert_ne!(a.public, a.private);
        assert!(s.get("identity").unwrap().is_some());
    }

    #[test]
    fn peer_save_load_remove() {
        let s = MemoryKeyStore::default();
        let pk = [9u8; 32];
        let peer = PairedPeer {
            device_id: device_id_of(&pk),
            name: "PC".into(),
            public_key: pk,
            mac_address: [1, 2, 3, 4, 5, 6],
        };
        assert_eq!(peer.device_id.len(), 16);
        assert_eq!(load_peer(&s, &peer.device_id).unwrap(), None);
        save_peer(&s, &peer).unwrap();
        assert!(
            s.get(&format!("peer:{}", peer.device_id))
                .unwrap()
                .is_some()
        );
        assert_eq!(load_peer(&s, &peer.device_id).unwrap(), Some(peer.clone()));
        remove_peer(&s, &peer.device_id).unwrap();
        assert_eq!(load_peer(&s, &peer.device_id).unwrap(), None);
        remove_peer(&s, &peer.device_id).unwrap();
    }

    #[test]
    fn active_peer_save_load_forget() {
        let s = MemoryKeyStore::default();
        let pk = [3u8; 32];
        let peer = PairedPeer {
            device_id: device_id_of(&pk),
            name: "PC".into(),
            public_key: pk,
            mac_address: [0; 6],
        };
        assert_eq!(load_active_peer(&s, "active_peer").unwrap(), None);
        save_active_peer(&s, "active_peer", &peer).unwrap();
        assert_eq!(
            s.get("active_peer").unwrap(),
            Some(peer.device_id.clone().into_bytes())
        );
        assert_eq!(
            load_active_peer(&s, "active_peer").unwrap(),
            Some(peer.clone())
        );
        // Outra conta não enxerga o ponteiro.
        assert_eq!(load_active_peer(&s, "active-peer").unwrap(), None);
        forget_active_peer(&s, "active_peer", &peer.device_id).unwrap();
        assert_eq!(load_active_peer(&s, "active_peer").unwrap(), None);
        assert_eq!(load_peer(&s, &peer.device_id).unwrap(), None);
    }
}
