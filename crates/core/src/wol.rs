//! dono: C4
//! Pacote mágico de Wake-on-LAN. Stubs do F2: assinaturas.

#[derive(Debug, thiserror::Error)]
pub enum WolError {
    #[error("não implementado")]
    NotImplemented,
    #[error("MAC inválido: {0}")]
    InvalidMac(String),
    #[error("{0}")]
    Io(String),
}

/// Pacote mágico de 102 bytes (6x0xFF + 16xMAC).
pub fn magic_packet(_mac: [u8; 6]) -> [u8; 102] {
    [0; 102]
}

/// Aceita separadores `:` e `-`.
pub fn parse_mac(input: &str) -> Result<[u8; 6], WolError> {
    Err(WolError::InvalidMac(input.to_string()))
}

/// Envia o pacote para 255.255.255.255:9 e para o broadcast da sub-rede.
pub fn send_wol(_mac: [u8; 6]) -> Result<(), WolError> {
    Err(WolError::NotImplemented)
}
