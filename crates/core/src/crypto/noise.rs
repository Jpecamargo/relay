//! Handshake Noise_KK_25519_ChaChaPoly_BLAKE2s da sessão (decisão D4) e frames u32.

use snow::{Builder, TransportState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::CryptoError;
use super::keystore::{StaticKeypair, device_id_of};
use super::udp::{UdpChannel, UdpKeys, derive_udp_keys};
use crate::types::PairedPeer;

const PARAMS: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";
/// Maior mensagem Noise (limite do protocolo).
const MAX_NOISE: usize = 65535;

/// Escreve um frame: u32 big-endian com o tamanho + corpo.
pub async fn write_frame<W: AsyncWrite + Unpin>(io: &mut W, body: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(body.len())
        .ok()
        .filter(|l| *l as usize <= MAX_NOISE)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame grande demais")
        })?;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(body).await?;
    io.flush().await
}

/// Lê um frame escrito por `write_frame` (recusa tamanho acima de 65535).
pub async fn read_frame<R: AsyncRead + Unpin>(io: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_NOISE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame grande demais",
        ));
    }
    let mut body = vec![0u8; len];
    io.read_exact(&mut body).await?;
    Ok(body)
}

fn other(e: impl std::fmt::Display) -> CryptoError {
    CryptoError::Other(e.to_string())
}

fn io_err(e: std::io::Error) -> CryptoError {
    CryptoError::Other(e.to_string())
}

/// Sessão cifrada do canal confiável, já com o par autenticado.
pub struct NoiseSession {
    transport: TransportState,
    handshake_hash: Vec<u8>,
    is_dialer: bool,
    pub peer: PairedPeer,
}

impl std::fmt::Debug for NoiseSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoiseSession")
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl NoiseSession {
    /// Cifra uma mensagem (até 65519 bytes) em `out`, que é sobrescrito.
    pub fn encrypt(&mut self, plain: &[u8], out: &mut Vec<u8>) -> Result<(), CryptoError> {
        out.clear();
        out.resize(plain.len() + 16, 0);
        let n = self.transport.write_message(plain, out).map_err(other)?;
        out.truncate(n);
        Ok(())
    }

    /// Decifra uma mensagem em `out`, que é sobrescrito.
    pub fn decrypt(&mut self, cipher: &[u8], out: &mut Vec<u8>) -> Result<(), CryptoError> {
        out.clear();
        out.resize(cipher.len(), 0);
        let n = self
            .transport
            .read_message(cipher, out)
            .map_err(|_| CryptoError::Auth)?;
        out.truncate(n);
        Ok(())
    }

    pub fn handshake_hash(&self) -> &[u8] {
        &self.handshake_hash
    }

    /// Chaves do canal UDP derivadas do hash do handshake.
    pub fn udp_keys(&self, channel: UdpChannel) -> UdpKeys {
        derive_udp_keys(&self.handshake_hash, channel, self.is_dialer)
    }
}

fn prologue(dialer_pub: &[u8; 32], listener_pub: &[u8; 32]) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[..32].copy_from_slice(dialer_pub);
    p[32..].copy_from_slice(listener_pub);
    p
}

fn finish(
    hs: snow::HandshakeState,
    is_dialer: bool,
    peer: PairedPeer,
) -> Result<NoiseSession, CryptoError> {
    let handshake_hash = hs.get_handshake_hash().to_vec();
    let transport = hs.into_transport_mode().map_err(other)?;
    Ok(NoiseSession {
        transport,
        handshake_hash,
        is_dialer,
        peer,
    })
}

/// Lado que abriu a conexão: conhece `peer` (pareado) e inicia o KK.
pub async fn noise_dial<S>(
    io: &mut S,
    me: &StaticKeypair,
    peer: &PairedPeer,
) -> Result<NoiseSession, CryptoError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // O listener precisa saber quem chama antes de montar o KK: a chave pública vai em claro.
    write_frame(io, &me.public).await.map_err(io_err)?;
    let pro = prologue(&me.public, &peer.public_key);
    let mut hs = Builder::new(PARAMS.parse().map_err(other)?)
        .local_private_key(&me.private)
        .map_err(other)?
        .remote_public_key(&peer.public_key)
        .map_err(other)?
        .prologue(&pro)
        .map_err(other)?
        .build_initiator()
        .map_err(other)?;
    let mut buf = vec![0u8; MAX_NOISE];
    let n = hs.write_message(&[], &mut buf).map_err(other)?;
    write_frame(io, &buf[..n]).await.map_err(io_err)?;
    let reply = read_frame(io).await.map_err(io_err)?;
    hs.read_message(&reply, &mut buf)
        .map_err(|_| CryptoError::Auth)?;
    finish(hs, true, peer.clone())
}

/// Lado que escuta: só aceita iniciador cuja chave estática está em `known`.
pub async fn noise_accept<S>(
    io: &mut S,
    me: &StaticKeypair,
    known: &[PairedPeer],
) -> Result<NoiseSession, CryptoError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let claimed = read_frame(io).await.map_err(io_err)?;
    let claimed: [u8; 32] = claimed
        .try_into()
        .map_err(|_| other("chave pública malformada"))?;
    let peer = known
        .iter()
        .find(|p| p.public_key == claimed && p.device_id == device_id_of(&claimed))
        .ok_or(CryptoError::UnknownPeer)?;
    let pro = prologue(&claimed, &me.public);
    let mut hs = Builder::new(PARAMS.parse().map_err(other)?)
        .local_private_key(&me.private)
        .map_err(other)?
        .remote_public_key(&claimed)
        .map_err(other)?
        .prologue(&pro)
        .map_err(other)?
        .build_responder()
        .map_err(other)?;
    let mut buf = vec![0u8; MAX_NOISE];
    let first = read_frame(io).await.map_err(io_err)?;
    hs.read_message(&first, &mut buf)
        .map_err(|_| CryptoError::Auth)?;
    let n = hs.write_message(&[], &mut buf).map_err(other)?;
    write_frame(io, &buf[..n]).await.map_err(io_err)?;
    finish(hs, false, peer.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{MemoryKeyStore, load_or_create_identity};
    use tokio::io::duplex;

    fn ident() -> StaticKeypair {
        load_or_create_identity(&MemoryKeyStore::default()).unwrap()
    }

    fn peer_of(k: &StaticKeypair, name: &str) -> PairedPeer {
        PairedPeer {
            device_id: device_id_of(&k.public),
            name: name.into(),
            public_key: k.public,
            mac_address: [0; 6],
        }
    }

    #[tokio::test]
    async fn paired_peers_complete_handshake_and_talk() {
        let (mac, pc) = (ident(), ident());
        let (mut a, mut b) = duplex(1 << 17);
        let (pc_p, mac_p) = (peer_of(&pc, "PC"), peer_of(&mac, "Mac"));
        let (ra, rb) = tokio::join!(
            noise_dial(&mut a, &mac, &pc_p),
            noise_accept(&mut b, &pc, std::slice::from_ref(&mac_p))
        );
        let (mut d, mut l) = (ra.unwrap(), rb.unwrap());
        assert_eq!(d.handshake_hash(), l.handshake_hash());
        assert_eq!(l.peer, mac_p);
        let (mut c, mut p) = (Vec::new(), Vec::new());
        d.encrypt(b"oi", &mut c).unwrap();
        l.decrypt(&c, &mut p).unwrap();
        assert_eq!(p, b"oi");
        l.encrypt(&[5u8; 65000], &mut c).unwrap();
        d.decrypt(&c, &mut p).unwrap();
        assert_eq!(p.len(), 65000);
        c[0] ^= 1;
        assert!(matches!(d.decrypt(&c, &mut p), Err(CryptoError::Auth)));
        for ch in [UdpChannel::Input, UdpChannel::AudioPcToMac] {
            let (ku, kl) = (d.udp_keys(ch), l.udp_keys(ch));
            assert_eq!(ku.send, kl.recv);
            assert_eq!(ku.recv, kl.send);
        }
    }

    #[tokio::test]
    async fn responder_refuses_unknown_initiator() {
        let (mac, pc, intruder) = (ident(), ident(), ident());
        let (a, b) = duplex(1 << 17);
        let pc_p = peer_of(&pc, "PC");
        let known = [peer_of(&mac, "Mac")];
        let dial = async move {
            let mut a = a;
            noise_dial(&mut a, &intruder, &pc_p).await
        };
        let accept = async move {
            let mut b = b;
            noise_accept(&mut b, &pc, &known).await
        };
        let (rd, ra) = tokio::join!(dial, accept);
        assert!(matches!(ra, Err(CryptoError::UnknownPeer)));
        assert!(rd.is_err());
    }

    #[tokio::test]
    async fn dialer_with_wrong_responder_key_fails() {
        let (mac, pc, other_pc) = (ident(), ident(), ident());
        let (a, b) = duplex(1 << 17);
        let wrong = peer_of(&other_pc, "PC?");
        let known = [peer_of(&mac, "Mac")];
        let dial = async move {
            let mut a = a;
            noise_dial(&mut a, &mac, &wrong).await
        };
        let accept = async move {
            let mut b = b;
            noise_accept(&mut b, &pc, &known).await
        };
        let (rd, ra) = tokio::join!(dial, accept);
        assert!(rd.is_err());
        assert!(ra.is_err());
    }
}
