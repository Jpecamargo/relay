//! Pareamento SPAKE2 com código de 6 dígitos (decisão D5).
//!
//! Fluxo: SPAKE2 (uma mensagem de cada lado) → confirmação de chave dos dois lados (código
//! errado falha aqui, antes de qualquer chave pública trafegar) → troca da identidade cifrada
//! com a chave da sessão SPAKE2.

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::io::{AsyncRead, AsyncWrite};

use super::PairingError;
use super::keystore::device_id_of;
use super::noise::{read_frame, write_frame};
use crate::types::{PairedPeer, Role};

const MAX_NAME: usize = 64;

/// Identidade que este lado entrega ao outro durante o pareamento.
#[derive(Debug, Clone)]
pub struct LocalDevice {
    pub name: String,
    pub public_key: [u8; 32],
    pub mac_address: [u8; 6],
}

fn other(e: impl std::fmt::Display) -> PairingError {
    PairingError::Other(e.to_string())
}

fn expand(hk: &Hkdf<Sha256>, info: &str) -> [u8; 32] {
    let mut k = [0u8; 32];
    hk.expand(info.as_bytes(), &mut k)
        .expect("32 bytes cabem no HKDF-SHA256");
    k
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Dialer => "dialer",
        Role::Listener => "listener",
    }
}

fn opposite(role: Role) -> Role {
    match role {
        Role::Dialer => Role::Listener,
        Role::Listener => Role::Dialer,
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Roda o pareamento sobre `io` (frames com prefixo u32). O dialer é quem abriu a conexão.
/// Devolve o outro dispositivo; quem chama persiste com `save_peer`.
pub async fn pair<S>(
    io: &mut S,
    role: Role,
    code: &str,
    me: &LocalDevice,
) -> Result<PairedPeer, PairingError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PairingError::Other("código deve ter 6 dígitos".into()));
    }
    if me.name.len() > MAX_NAME {
        return Err(PairingError::Other(
            "nome do dispositivo longo demais".into(),
        ));
    }
    let password = Password::new(code.as_bytes());
    let id_a = Identity::new(b"relay-dialer");
    let id_b = Identity::new(b"relay-listener");
    let (state, msg) = match role {
        Role::Dialer => Spake2::<Ed25519Group>::start_a(&password, &id_a, &id_b),
        Role::Listener => Spake2::<Ed25519Group>::start_b(&password, &id_a, &id_b),
    };
    write_frame(io, &msg).await.map_err(other)?;
    let theirs = read_frame(io).await.map_err(other)?;
    let key = state.finish(&theirs).map_err(other)?;
    let hk = Hkdf::<Sha256>::new(Some(b"relay pairing v1"), &key);

    // Confirmação: os dois lados enviam e conferem antes de qualquer outra coisa.
    let tag = |r: Role| expand(&hk, &format!("confirm {}", role_label(r)));
    write_frame(io, &tag(role)).await.map_err(other)?;
    let their_tag = read_frame(io).await.map_err(other)?;
    if !ct_eq(&their_tag, &tag(opposite(role))) {
        return Err(PairingError::CodeMismatch);
    }

    // Troca de identidade, cifrada com a chave da sessão (nonce fixo: cada chave cifra uma vez).
    let cipher = |r: Role| {
        ChaCha20Poly1305::new(&expand(&hk, &format!("identity {}", role_label(r))).into())
    };
    let nonce = Nonce::from([0u8; 12]);
    let send = |me: &LocalDevice| -> Result<Vec<u8>, PairingError> {
        let mut buf = Vec::with_capacity(38 + me.name.len() + 16);
        buf.extend_from_slice(&me.public_key);
        buf.extend_from_slice(&me.mac_address);
        buf.extend_from_slice(me.name.as_bytes());
        cipher(role)
            .encrypt_in_place(&nonce, b"", &mut buf)
            .map_err(other)?;
        Ok(buf)
    };
    let recv = |mut buf: Vec<u8>| -> Result<PairedPeer, PairingError> {
        cipher(opposite(role))
            .decrypt_in_place(&nonce, b"", &mut buf)
            .map_err(other)?;
        if buf.len() < 38 || buf.len() > 38 + MAX_NAME {
            return Err(PairingError::Other("identidade malformada".into()));
        }
        let public_key: [u8; 32] = buf[..32].try_into().map_err(other)?;
        let mac_address: [u8; 6] = buf[32..38].try_into().map_err(other)?;
        let name = String::from_utf8(buf[38..].to_vec()).map_err(other)?;
        Ok(PairedPeer {
            device_id: device_id_of(&public_key),
            name,
            public_key,
            mac_address,
        })
    };
    match role {
        Role::Dialer => {
            write_frame(io, &send(me)?).await.map_err(other)?;
            recv(read_frame(io).await.map_err(other)?)
        }
        Role::Listener => {
            let peer = recv(read_frame(io).await.map_err(other)?)?;
            write_frame(io, &send(me)?).await.map_err(other)?;
            Ok(peer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, duplex};

    fn dev(n: u8, name: &str) -> LocalDevice {
        LocalDevice {
            name: name.into(),
            public_key: [n; 32],
            mac_address: [n; 6],
        }
    }

    #[tokio::test]
    async fn same_code_exchanges_identities() {
        let (mut a, mut b) = duplex(4096);
        let (mac, pc) = (dev(1, "MacBook"), dev(2, "PC"));
        let (ra, rb) = tokio::join!(
            pair(&mut a, Role::Dialer, "012345", &mac),
            pair(&mut b, Role::Listener, "012345", &pc)
        );
        let (pa, pb) = (ra.unwrap(), rb.unwrap());
        assert_eq!(pa.name, "PC");
        assert_eq!(pa.public_key, [2; 32]);
        assert_eq!(pa.mac_address, [2; 6]);
        assert_eq!(pa.device_id, device_id_of(&[2; 32]));
        assert_eq!(pb.name, "MacBook");
        assert_eq!(pb.public_key, [1; 32]);
        assert_eq!(pb.mac_address, [1; 6]);
    }

    #[tokio::test]
    async fn different_code_fails_on_both_sides_without_key_exchange() {
        let (mut a, mut b) = duplex(4096);
        let (mac, pc) = (dev(1, "MacBook"), dev(2, "PC"));
        let (ra, rb) = tokio::join!(
            pair(&mut a, Role::Dialer, "012345", &mac),
            pair(&mut b, Role::Listener, "543210", &pc)
        );
        assert!(matches!(ra, Err(PairingError::CodeMismatch)));
        assert!(matches!(rb, Err(PairingError::CodeMismatch)));
        // nada além de SPAKE2 + confirmação trafegou: a chave pública não vai no fio
        drop((a, b));
    }

    #[tokio::test]
    async fn wrong_code_never_sends_identity_frame() {
        let (mut a, mut spy) = duplex(4096);
        let mac = dev(1, "MacBook");
        let dialer = async move { pair(&mut a, Role::Dialer, "012345", &mac).await };
        // listener falso com código errado: fala SPAKE2 + confirmação e vê o que mais chega
        let fake = async move {
            let (st, msg) = Spake2::<Ed25519Group>::start_b(
                &Password::new(b"999999"),
                &Identity::new(b"relay-dialer"),
                &Identity::new(b"relay-listener"),
            );
            write_frame(&mut spy, &msg).await.unwrap();
            let theirs = read_frame(&mut spy).await.unwrap();
            st.finish(&theirs).unwrap();
            write_frame(&mut spy, &[0u8; 32]).await.unwrap();
            let confirm = read_frame(&mut spy).await.unwrap();
            let mut rest = Vec::new();
            spy.read_to_end(&mut rest).await.unwrap();
            (confirm.len(), rest)
        };
        let (r, (confirm_len, rest)) = tokio::join!(dialer, fake);
        assert!(matches!(r, Err(PairingError::CodeMismatch)));
        assert_eq!(confirm_len, 32);
        assert!(rest.is_empty());
    }

    #[tokio::test]
    async fn rejects_malformed_code() {
        let (mut a, _b) = duplex(64);
        let r = pair(&mut a, Role::Dialer, "12ab56", &dev(1, "x")).await;
        assert!(matches!(r, Err(PairingError::Other(_))));
    }
}
