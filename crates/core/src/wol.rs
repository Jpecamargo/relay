//! dono: C4
//! Pacote mágico de Wake-on-LAN.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

/// Porta UDP "discard", a usual para Wake-on-LAN.
const WOL_PORT: u16 = 9;

#[derive(Debug, thiserror::Error)]
pub enum WolError {
    #[error("MAC inválido: {0}")]
    InvalidMac(String),
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for WolError {
    fn from(e: std::io::Error) -> Self {
        WolError::Io(e.to_string())
    }
}

/// Pacote mágico de 102 bytes (6x0xFF + 16xMAC).
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut p = [0xFFu8; 102];
    for chunk in p[6..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac);
    }
    p
}

/// Aceita separadores `:` e `-` (um só tipo por MAC), seis grupos de dois dígitos hex.
pub fn parse_mac(input: &str) -> Result<[u8; 6], WolError> {
    let err = || WolError::InvalidMac(input.to_string());
    let s = input.trim();
    let sep = if s.contains(':') { ':' } else { '-' };
    if s.contains(if sep == ':' { '-' } else { ':' }) {
        return Err(err());
    }
    let mut mac = [0u8; 6];
    let mut parts = s.split(sep);
    for byte in &mut mac {
        let part = parts.next().ok_or_else(err)?;
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        *byte = u8::from_str_radix(part, 16).map_err(|_| err())?;
    }
    if parts.next().is_some() {
        return Err(err());
    }
    Ok(mac)
}

/// Broadcast da sub-rede de `ip`, assumindo /24 (a std não expõe a máscara da interface
/// e o core não tem dependência de enumeração de interfaces).
pub fn subnet_broadcast(ip: Ipv4Addr) -> Ipv4Addr {
    let [a, b, c, _] = ip.octets();
    Ipv4Addr::new(a, b, c, 255)
}

/// IPv4 local da rota padrão (o `connect` em UDP não envia nada).
fn local_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    match sock.local_addr().ok()? {
        SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()),
        _ => None,
    }
}

/// Envia o pacote para 255.255.255.255:9 e para o broadcast da sub-rede.
/// Sucesso se ao menos um envio funcionar.
pub fn send_wol(mac: [u8; 6]) -> Result<(), WolError> {
    let packet = magic_packet(mac);
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    let mut targets = vec![Ipv4Addr::BROADCAST];
    if let Some(b) = local_ipv4().map(subnet_broadcast) {
        targets.push(b);
    }
    let mut last_err = None;
    let mut sent = false;
    for ip in targets {
        match sock.send_to(&packet, SocketAddrV4::new(ip, WOL_PORT)) {
            Ok(_) => sent = true,
            Err(e) => last_err = Some(e),
        }
    }
    match (sent, last_err) {
        (true, _) => Ok(()),
        (false, Some(e)) => Err(e.into()),
        (false, None) => Err(WolError::Io("nenhum destino".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_has_102_bytes_and_layout() {
        let mac = [1, 2, 3, 4, 5, 6];
        let p = magic_packet(mac);
        assert_eq!(p.len(), 102);
        assert!(p[..6].iter().all(|&b| b == 0xFF));
        for i in 0..16 {
            assert_eq!(&p[6 + i * 6..12 + i * 6], &mac);
        }
    }

    #[test]
    fn parse_mac_accepts_both_separators() {
        let want = [0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x0F];
        assert_eq!(parse_mac("aa:bb:cc:01:02:0f").unwrap(), want);
        assert_eq!(parse_mac("AA-BB-CC-01-02-0F").unwrap(), want);
    }

    #[test]
    fn parse_mac_rejects_invalid() {
        for bad in [
            "",
            "aa:bb:cc:01:02",
            "aa:bb:cc:01:02:0f:10",
            "aa:bb:cc:01:02:0g",
            "aa:bb-cc:01:02:0f",
            "aabbcc01020f",
            "a:b:c:1:2:3",
            "aa:bb:cc:01:02:+f",
        ] {
            assert!(parse_mac(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn subnet_broadcast_is_slash_24() {
        assert_eq!(
            subnet_broadcast(Ipv4Addr::new(192, 168, 0, 17)),
            Ipv4Addr::new(192, 168, 0, 255)
        );
    }
}
