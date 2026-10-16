//! dono: W3
//! Parser puro de EDID (`parse_edid_size_mm`), testado no Mac.

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
/// Os quatro descritores de 18 bytes do bloco base começam aqui.
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];

/// Tamanho físico da imagem em mm, `(largura, altura)`.
///
/// Usa o primeiro descritor de timing detalhado (bytes 12-14 do descritor, com 12 bits por eixo);
/// se ele não existir ou vier zerado, cai para os bytes 21-22 do bloco base (cm, ×10). `None` se o
/// EDID for curto, sem cabeçalho válido ou sem tamanho (projetores e telas virtuais zeram os dois).
pub fn parse_edid_size_mm(bytes: &[u8]) -> Option<(f32, f32)> {
    if bytes.len() < 128 || bytes[..8] != HEADER {
        return None;
    }
    for start in DESCRIPTORS {
        let d = &bytes[start..start + 18];
        // Pixel clock zero marca descritor de outro tipo (nome, faixa, texto...), não de timing.
        if d[0] == 0 && d[1] == 0 {
            continue;
        }
        let w = u32::from(d[12]) | (u32::from(d[14] & 0xF0) << 4);
        let h = u32::from(d[13]) | (u32::from(d[14] & 0x0F) << 8);
        if w > 0 && h > 0 {
            return Some((w as f32, h as f32));
        }
        break; // só o timing preferido (o primeiro) vale; zerado, cai para os bytes em cm
    }
    let (w_cm, h_cm) = (bytes[21], bytes[22]);
    (w_cm > 0 && h_cm > 0).then(|| (f32::from(w_cm) * 10.0, f32::from(h_cm) * 10.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EDID base de 128 bytes: timing preferido de 527x296 mm (monitor 24"), 53x30 cm nos bytes 21-22.
    fn sample() -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&HEADER);
        e[21] = 53;
        e[22] = 30;
        let d = &mut e[54..72];
        d[0] = 0x02; // pixel clock != 0
        d[1] = 0x3A;
        d[12] = (527 & 0xFF) as u8;
        d[13] = (296 & 0xFF) as u8;
        d[14] = (((527 >> 8) as u8) << 4) | ((296 >> 8) as u8);
        e
    }

    #[test]
    fn usa_descritor_de_timing_detalhado() {
        assert_eq!(parse_edid_size_mm(&sample()), Some((527.0, 296.0)));
    }

    #[test]
    fn descritor_com_mais_de_8_bits_por_eixo() {
        let mut e = sample();
        // 1_010 x 568 mm: precisa dos nibbles altos do byte 14.
        e[54 + 12] = (1010 & 0xFF) as u8;
        e[54 + 13] = (568 & 0xFF) as u8;
        e[54 + 14] = (((1010 >> 8) as u8) << 4) | ((568 >> 8) as u8);
        assert_eq!(parse_edid_size_mm(&e), Some((1010.0, 568.0)));
    }

    #[test]
    fn cai_para_bytes_em_cm_quando_timing_zerado() {
        let mut e = sample();
        e[54 + 12] = 0;
        e[54 + 13] = 0;
        e[54 + 14] = 0;
        assert_eq!(parse_edid_size_mm(&e), Some((530.0, 300.0)));
    }

    #[test]
    fn cai_para_bytes_em_cm_sem_descritor_de_timing() {
        let mut e = sample();
        e[54] = 0;
        e[55] = 0;
        assert_eq!(parse_edid_size_mm(&e), Some((530.0, 300.0)));
    }

    #[test]
    fn sem_tamanho_devolve_none() {
        let mut e = sample();
        e[21] = 0;
        e[22] = 0;
        e[54 + 12] = 0;
        e[54 + 13] = 0;
        e[54 + 14] = 0;
        assert_eq!(parse_edid_size_mm(&e), None);
    }

    #[test]
    fn rejeita_curto_e_sem_cabecalho() {
        assert_eq!(parse_edid_size_mm(&[]), None);
        assert_eq!(parse_edid_size_mm(&sample()[..127]), None);
        let mut e = sample();
        e[0] = 1;
        assert_eq!(parse_edid_size_mm(&e), None);
    }
}
