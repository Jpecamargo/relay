//! dono: C4
//! Descoberta mDNS (anúncio e busca) e endereço manual.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::types::DiscoveredPeer;
use crate::types::consts::{MDNS_SERVICE_TYPE, PORT_CONTROL, PORT_INPUT, PROTOCOL_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("endereço inválido: {0}")]
    InvalidAddress(String),
    #[error("{0}")]
    Mdns(String),
}

impl From<mdns_sd::Error> for DiscoveryError {
    fn from(e: mdns_sd::Error) -> Self {
        DiscoveryError::Mdns(e.to_string())
    }
}

/// Hostname da máquina (variáveis de ambiente, depois o comando `hostname`); `relay` se falhar.
pub fn local_hostname() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .or_else(|| {
            let out = std::process::Command::new("hostname").output().ok()?;
            String::from_utf8(out.stdout).ok()
        })
        .map(|h| h.trim().trim_end_matches(".local").to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "relay".to_string())
}

/// Anuncia `_relay._tcp.local.` (instância = hostname; TXT id, pv, udp). Para ao ser descartado.
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl std::fmt::Debug for Advertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advertiser")
            .field("fullname", &self.fullname)
            .finish()
    }
}

impl Advertiser {
    /// Anuncia com o hostname da máquina como nome da instância.
    pub fn start(device_id: &str) -> Result<Self, DiscoveryError> {
        Self::start_named(&local_hostname(), device_id)
    }

    /// Como `start`, com nome de instância explícito.
    pub fn start_named(instance: &str, device_id: &str) -> Result<Self, DiscoveryError> {
        Self::start_with_ports(instance, device_id, PORT_CONTROL, PORT_INPUT)
    }

    /// Como `start_named`, anunciando as portas reais de escuta (TCP de controle e UDP de input).
    pub fn start_with_ports(
        instance: &str,
        device_id: &str,
        control_port: u16,
        input_port: u16,
    ) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        let props = [
            ("id", device_id.to_string()),
            ("pv", PROTOCOL_VERSION.to_string()),
            ("udp", input_port.to_string()),
        ];
        let host = format!("{instance}.local.");
        let info = ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            instance,
            &host,
            "",
            control_port,
            &props[..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Procura agentes e emite `DiscoveredPeer`. Para ao ser descartado.
pub struct Browser {
    daemon: ServiceDaemon,
    events: mdns_sd::Receiver<ServiceEvent>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser").finish_non_exhaustive()
    }
}

impl Browser {
    pub fn start() -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        let events = daemon.browse(MDNS_SERVICE_TYPE)?;
        Ok(Self { daemon, events })
    }

    /// Próximo par resolvido, ou `None` se nada chegar dentro de `timeout`.
    pub fn next_peer(&self, timeout: Duration) -> Option<DiscoveredPeer> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match self.events.recv_timeout(left).ok()? {
                ServiceEvent::ServiceResolved(svc) => {
                    if let Some(peer) = peer_from_service(&svc) {
                        return Some(peer);
                    }
                }
                _ => continue,
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(MDNS_SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

fn peer_from_service(svc: &ResolvedService) -> Option<DiscoveredPeer> {
    let device_id = svc.get_property_val_str("id")?.to_string();
    let protocol_version = svc.get_property_val_str("pv")?.parse().ok()?;
    let suffix = format!(".{}", svc.ty_domain);
    let name = svc
        .fullname
        .strip_suffix(&suffix)
        .unwrap_or(&svc.fullname)
        .to_string();
    let mut addrs: Vec<String> = svc
        .addresses
        .iter()
        .map(|a| a.to_ip_addr())
        .filter(|a| !a.is_unspecified())
        .map(|a| a.to_string())
        .collect();
    addrs.sort();
    Some(DiscoveredPeer {
        name,
        addrs,
        port: svc.port,
        device_id,
        protocol_version,
    })
}

/// Aceita `192.168.0.10`, `192.168.0.10:47800`, `pc.local` e `[fe80::1]:47800`; devolve
/// (host, porta) com a porta padrão quando falta. IPv6 volta sem colchetes.
pub fn parse_manual_address(input: &str) -> Result<(String, u16), DiscoveryError> {
    let err = || DiscoveryError::InvalidAddress(input.to_string());
    let s = input.trim();
    let parse_port = |p: &str| match p.parse::<u16>() {
        Ok(n) if n != 0 && p.bytes().all(|b| b.is_ascii_digit()) => Ok(n),
        _ => Err(err()),
    };

    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or_else(err)?;
        h.parse::<Ipv6Addr>().map_err(|_| err())?;
        let port = match after {
            "" => PORT_CONTROL,
            _ => parse_port(after.strip_prefix(':').ok_or_else(err)?)?,
        };
        (h, port)
    } else if s.parse::<Ipv6Addr>().is_ok() {
        (s, PORT_CONTROL)
    } else if let Some((h, p)) = s.split_once(':') {
        (h, parse_port(p)?)
    } else {
        (s, PORT_CONTROL)
    };

    let valid = if host.contains(':') {
        true
    } else if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        host.parse::<Ipv4Addr>().is_ok()
    } else {
        valid_hostname(host)
    };
    if !valid {
        return Err(err());
    }
    Ok((host.to_string(), port))
}

fn valid_hostname(h: &str) -> bool {
    let h = h.strip_suffix('.').unwrap_or(h);
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_address_valid() {
        let d = PORT_CONTROL;
        let ok = |i: &str| parse_manual_address(i).unwrap();
        assert_eq!(ok("192.168.0.10"), ("192.168.0.10".into(), d));
        assert_eq!(ok("192.168.0.10:47800"), ("192.168.0.10".into(), 47800));
        assert_eq!(ok("192.168.0.10:5000"), ("192.168.0.10".into(), 5000));
        assert_eq!(ok("pc.local"), ("pc.local".into(), d));
        assert_eq!(ok("pc.local:1234"), ("pc.local".into(), 1234));
        assert_eq!(ok("[fe80::1]:47800"), ("fe80::1".into(), 47800));
        assert_eq!(ok("[fe80::1]"), ("fe80::1".into(), d));
        assert_eq!(ok("fe80::1"), ("fe80::1".into(), d));
    }

    #[test]
    fn manual_address_garbage() {
        for bad in [
            "",
            " ",
            ":47800",
            "a b",
            "pc.local:",
            "pc.local:abc",
            "pc.local:0",
            "pc.local:70000",
            "256.1.1.1",
            "1.2.3",
            "[fe80::1",
            "[fe80::1]x",
            "[pc]:1",
            "-a.local",
            "a..local",
            "a:b:c",
            "http://pc",
        ] {
            assert!(parse_manual_address(bad).is_err(), "{bad:?}");
        }
    }

    /// Depende de multicast na máquina (pode falhar em CI/sandbox): `cargo test -- --ignored`.
    #[test]
    #[ignore = "precisa de multicast na rede local"]
    fn loopback_advertise_and_browse() {
        let name = format!("relay-test-{}", std::process::id());
        let start = Instant::now();
        let _adv = Advertiser::start_named(&name, "dev-123").unwrap();
        let browser = Browser::start().unwrap();
        let peer = loop {
            let p = browser
                .next_peer(Duration::from_secs(5).saturating_sub(start.elapsed()))
                .expect("não encontrou em 5 s");
            if p.name == name {
                break p;
            }
        };
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(peer.device_id, "dev-123");
        assert_eq!(peer.port, PORT_CONTROL);
        assert_eq!(peer.protocol_version, PROTOCOL_VERSION);
        assert!(!peer.addrs.is_empty());
    }
}
