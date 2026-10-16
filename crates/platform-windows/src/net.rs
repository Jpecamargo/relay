//! dono: W3
//! MAC address da interface principal (`GetAdaptersAddresses`).

use relay_core::platform::PlatformError;

/// `IF_TYPE_SOFTWARE_LOOPBACK` e `IF_TYPE_TUNNEL`: nunca servem para Wake-on-LAN.
const IF_TYPE_LOOPBACK: u32 = 24;
const IF_TYPE_TUNNEL: u32 = 131;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterCandidate {
    pub mac: [u8; 6],
    pub up: bool,
    pub if_type: u32,
    pub has_gateway: bool,
}

/// Escolhe a interface principal: ativa, não loopback/túnel, MAC não nulo; a que tem gateway
/// (a que sai para a LAN) vence, senão a primeira elegível.
pub fn pick_primary(adapters: &[AdapterCandidate]) -> Option<[u8; 6]> {
    let mut eligible = adapters.iter().filter(|a| {
        a.up && a.if_type != IF_TYPE_LOOPBACK && a.if_type != IF_TYPE_TUNNEL && a.mac != [0; 6]
    });
    let first = eligible.next()?;
    if first.has_gateway {
        return Some(first.mac);
    }
    Some(
        eligible
            .find(|a| a.has_gateway)
            .map_or(first.mac, |a| a.mac),
    )
}

#[cfg(windows)]
pub fn primary_mac_address() -> Result<[u8; 6], PlatformError> {
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::AF_UNSPEC;

    let flags = GAA_FLAG_INCLUDE_GATEWAYS
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = 16 * 1024u32;
    // u64 garante o alinhamento de 8 bytes que IP_ADAPTER_ADDRESSES_LH exige.
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..4 {
        buf.resize((size as usize).div_ceil(8), 0);
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: `head` aponta para um buffer de pelo menos `size` bytes.
        let rc =
            unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(head), &mut size) };
        if rc == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if rc != ERROR_SUCCESS.0 {
            return Err(PlatformError::Platform(format!(
                "GetAdaptersAddresses falhou ({rc})"
            )));
        }
        let mut candidates = Vec::new();
        let mut cur = head as *const IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: a lista encadeada vive dentro de `buf`, que segue vivo; o SO a preencheu.
        while let Some(a) = unsafe { cur.as_ref() } {
            if a.PhysicalAddressLength == 6 {
                let mut mac = [0u8; 6];
                mac.copy_from_slice(&a.PhysicalAddress[..6]);
                candidates.push(AdapterCandidate {
                    mac,
                    up: a.OperStatus == IfOperStatusUp,
                    if_type: a.IfType,
                    has_gateway: !a.FirstGatewayAddress.is_null(),
                });
            }
            cur = a.Next;
        }
        return pick_primary(&candidates)
            .ok_or_else(|| PlatformError::Platform("nenhuma interface de rede elegível".into()));
    }
    Err(PlatformError::Platform(
        "GetAdaptersAddresses: buffer insuficiente".into(),
    ))
}

#[cfg(not(windows))]
pub fn primary_mac_address() -> Result<[u8; 6], PlatformError> {
    Err(PlatformError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(last: u8, up: bool, if_type: u32, has_gateway: bool) -> AdapterCandidate {
        AdapterCandidate {
            mac: [2, 0, 0, 0, 0, last],
            up,
            if_type,
            has_gateway,
        }
    }

    #[test]
    fn prefere_gateway() {
        let v = [a(1, true, 6, false), a(2, true, 71, true)];
        assert_eq!(pick_primary(&v), Some([2, 0, 0, 0, 0, 2]));
    }

    #[test]
    fn sem_gateway_pega_a_primeira_elegivel() {
        let v = [
            a(1, false, 6, true),
            a(2, true, 6, false),
            a(3, true, 6, false),
        ];
        assert_eq!(pick_primary(&v), Some([2, 0, 0, 0, 0, 2]));
    }

    #[test]
    fn ignora_loopback_tunel_e_mac_nulo() {
        let mut zero = a(0, true, 6, true);
        zero.mac = [0; 6];
        let v = [a(1, true, 24, true), a(2, true, 131, true), zero];
        assert_eq!(pick_primary(&v), None);
        assert_eq!(pick_primary(&[]), None);
    }
}
