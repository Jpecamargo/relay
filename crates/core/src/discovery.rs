//! dono: C4
//! Descoberta mDNS (anúncio e busca) e endereço manual. Stubs do F2: assinaturas.

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("não implementado")]
    NotImplemented,
    #[error("endereço inválido: {0}")]
    InvalidAddress(String),
    #[error("{0}")]
    Mdns(String),
}

/// Anuncia `_relay._tcp.local.` (instância = hostname; TXT id, pv, udp).
#[derive(Debug, Default)]
pub struct Advertiser;

/// Procura agentes e emite `DiscoveredPeer`.
#[derive(Debug, Default)]
pub struct Browser;

/// Aceita `192.168.0.10`, `192.168.0.10:47800`, `pc.local` e `[fe80::1]:47800`; devolve
/// (host, porta) com a porta padrão quando falta.
pub fn parse_manual_address(input: &str) -> Result<(String, u16), DiscoveryError> {
    Err(DiscoveryError::InvalidAddress(input.to_string()))
}
