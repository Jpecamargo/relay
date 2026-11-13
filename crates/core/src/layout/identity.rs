//! Identidade física de um monitor (fabricante + número de série do EDID), igual vista do Mac e
//! do PC. O código de modelo não entra: o mesmo monitor informa um por entrada (DP, HDMI, USB-C).

use crate::types::DisplayInfo;

/// Sigla PnP de 3 letras do fabricante (bytes 8-9 do EDID, big-endian; `CGDisplayVendorNumber`).
pub fn pnp_vendor(code: u16) -> Option<String> {
    let letter = |shift: u16| {
        let v = ((code >> shift) & 0x1F) as u8;
        (1..=26).contains(&v).then(|| char::from(b'A' + v - 1))
    };
    Some([letter(10)?, letter(5)?, letter(0)?].iter().collect())
}

/// `"DEL:4JZ4614"`. `None` sem fabricante válido ou sem número de série alfanumérico (o numérico
/// costuma ser igual entre unidades do mesmo modelo e casaria monitores diferentes).
pub fn display_identity(vendor: u16, serial: &str) -> Option<String> {
    let serial = serial.trim();
    if serial.is_empty() {
        return None;
    }
    Some(format!("{}:{serial}", pnp_vendor(vendor)?))
}

/// Identidade de um EDID base: fabricante (bytes 8-9) e o descritor de número de série (tag
/// 0xFF). `None` se o EDID for curto, sem cabeçalho ou sem esse descritor.
pub fn edid_identity(edid: &[u8]) -> Option<String> {
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if edid.len() < 128 || edid[..8] != HEADER {
        return None;
    }
    let vendor = u16::from_be_bytes([edid[8], edid[9]]);
    let serial = [54, 72, 90, 108].into_iter().find_map(|start| {
        let d = &edid[start..start + 18];
        (d[..3] == [0, 0, 0] && d[3] == 0xFF).then(|| {
            let text = &d[5..];
            let end = text.iter().position(|&b| b == 0x0A).unwrap_or(text.len());
            String::from_utf8_lossy(&text[..end]).into_owned()
        })
    })?;
    display_identity(vendor, &serial)
}

/// Tira de `local` as telas cuja identidade (`identities`: id -> identidade) o peer informou
/// como ativa nele: o monitor está mostrando o peer, não esta máquina.
pub fn without_peer_shown(
    local: Vec<DisplayInfo>,
    identities: &[(String, String)],
    peer_shown: &[String],
) -> Vec<DisplayInfo> {
    if peer_shown.is_empty() {
        return local;
    }
    local
        .into_iter()
        .filter(|d| {
            !identities
                .iter()
                .any(|(id, ident)| *id == d.id && peer_shown.contains(ident))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Machine;

    /// EDID com fabricante DEL (0x10AC) e descritor de série "4JZ4614\n".
    fn edid(serial: Option<&[u8]>) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        e[8] = 0x10;
        e[9] = 0xAC;
        e[54] = 0x02; // timing, não é descritor de texto
        if let Some(s) = serial {
            let d = &mut e[90..108];
            d[3] = 0xFF;
            d[5..5 + s.len()].copy_from_slice(s);
        }
        e
    }

    fn display(id: &str) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            name: id.into(),
            machine: Machine::Windows,
            origin_px: (0, 0),
            size_px: (1920, 1080),
            scale: 1.0,
            size_mm: None,
            is_primary: false,
            is_builtin: false,
        }
    }

    #[test]
    fn vendor_dell_e_apple() {
        assert_eq!(pnp_vendor(0x10AC).as_deref(), Some("DEL"));
        assert_eq!(pnp_vendor(0x0610).as_deref(), Some("APP"));
        assert_eq!(pnp_vendor(0), None);
    }

    #[test]
    fn identidade_do_edid_igual_a_do_mac() {
        let from_edid = edid_identity(&edid(Some(b"4JZ4614\n     ")));
        assert_eq!(from_edid.as_deref(), Some("DEL:4JZ4614"));
        assert_eq!(from_edid, display_identity(0x10AC, "4JZ4614"));
    }

    #[test]
    fn sem_serie_alfanumerica_nao_tem_identidade() {
        assert_eq!(edid_identity(&edid(None)), None);
        assert_eq!(edid_identity(&edid(None)[..100]), None);
        assert_eq!(display_identity(0x10AC, "  "), None);
    }

    #[test]
    fn tira_so_a_tela_que_o_peer_mostra() {
        let local = vec![display("A"), display("B")];
        let ids = vec![("A".into(), "DEL:1".into()), ("B".into(), "DEL:2".into())];
        let out = without_peer_shown(local.clone(), &ids, &["DEL:2".into()]);
        assert_eq!(out, vec![display("A")]);
        assert_eq!(without_peer_shown(local.clone(), &ids, &[]), local);
        assert_eq!(
            without_peer_shown(local.clone(), &[], &["DEL:2".into()]),
            local
        );
    }
}
