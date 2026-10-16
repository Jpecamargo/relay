//! dono: W5
//! Funções puras de formato (CF_HTML, DIB↔PNG, DROPFILES, escolha de formato, anti-eco),
//! sem chamada Win32 e testadas no Mac.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use relay_core::types::ClipboardFormat;

/// Nome do formato registrado que marca o que o Relay escreveu (anti-eco, D20/spec).
pub const RELAY_REMOTE_FORMAT: &str = "RelayRemote";
pub const HTML_FORMAT: &str = "HTML Format";
pub const RTF_FORMAT: &str = "Rich Text Format";
pub const PNG_FORMAT: &str = "PNG";

/// Decisão anti-eco: a mudança de clipboard só é notificada se o conteúdo NÃO carrega o marcador
/// `RelayRemote` (ou seja, não foi escrito por `write_remote`).
pub fn should_notify_change(present_formats: &[u32], remote_marker: u32) -> bool {
    remote_marker == 0 || !present_formats.contains(&remote_marker)
}

/// Escolhe o formato a ler entre os disponíveis: arquivos primeiro; com texto disponível, o mais
/// rico (HTML > RTF > texto); sem texto, imagem antes de HTML/RTF soltos.
pub fn pick_format(available: &[ClipboardFormat]) -> Option<ClipboardFormat> {
    use ClipboardFormat::{FileList, Html, Png, Rtf, Text};
    let has = |f| available.contains(&f);
    if has(FileList) {
        return Some(FileList);
    }
    let order: [ClipboardFormat; 4] = if has(Text) {
        [Html, Rtf, Text, Png]
    } else {
        [Png, Html, Rtf, Text]
    };
    order.into_iter().find(|f| has(*f))
}

// ---------------------------------------------------------------- CF_HTML

/// Monta o formato de área de transferência "HTML Format" (versão 0.9) em torno de `fragment`.
/// Os offsets do cabeçalho são em bytes UTF-8, com 10 dígitos.
pub fn build_cf_html(fragment: &str) -> String {
    const PRE: &str = "<html>\r\n<body>\r\n<!--StartFragment-->";
    const POST: &str = "<!--EndFragment-->\r\n</body>\r\n</html>";
    let header = |sh: usize, eh: usize, sf: usize, ef: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{sh:010}\r\nEndHTML:{eh:010}\r\n\
             StartFragment:{sf:010}\r\nEndFragment:{ef:010}\r\n"
        )
    };
    let header_len = header(0, 0, 0, 0).len();
    let start_fragment = header_len + PRE.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + POST.len();
    let mut out = header(header_len, end_html, start_fragment, end_fragment);
    out.push_str(PRE);
    out.push_str(fragment);
    out.push_str(POST);
    out
}

/// Extrai o fragmento de um "HTML Format" usando `StartFragment`/`EndFragment`; se os offsets
/// forem inválidos, cai nos comentários `<!--StartFragment-->`/`<!--EndFragment-->`.
pub fn parse_cf_html(data: &[u8]) -> Option<String> {
    let data = match data.iter().position(|&b| b == 0) {
        Some(n) => &data[..n],
        None => data,
    };
    let text_head = String::from_utf8_lossy(&data[..data.len().min(512)]).into_owned();
    let field = |name: &str| -> Option<usize> {
        let rest = text_head.split(name).nth(1)?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    if let (Some(s), Some(e)) = (field("StartFragment:"), field("EndFragment:")) {
        if s <= e && e <= data.len() {
            if let Ok(f) = std::str::from_utf8(&data[s..e]) {
                return Some(f.to_owned());
            }
        }
    }
    let all = std::str::from_utf8(data).ok()?;
    const SM: &str = "<!--StartFragment-->";
    let s = all.find(SM)? + SM.len();
    let e = all[s..].find("<!--EndFragment-->")? + s;
    Some(all[s..e].to_owned())
}

// ---------------------------------------------------------------- DIB <-> PNG

fn rd_u16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn rd_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn rd_i32(b: &[u8], o: usize) -> Option<i32> {
    Some(rd_u32(b, o)? as i32)
}

/// Extrai o canal de `px` selecionado por `mask`, escalado para 8 bits.
fn channel(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    let v = (px & mask) >> shift;
    if max == 255 {
        v as u8
    } else {
        ((u64::from(v) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
    }
}

/// Converte um DIB (CF_DIB/CF_DIBV5, sem o cabeçalho de arquivo BMP) de 24 ou 32 bpp em PNG RGBA.
pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "DIB truncado ou inválido".to_owned();
    let hsize = rd_u32(dib, 0).ok_or_else(bad)? as usize;
    if hsize < 40 {
        return Err("cabeçalho DIB não suportado".into());
    }
    let width = rd_i32(dib, 4).ok_or_else(bad)?;
    let height = rd_i32(dib, 8).ok_or_else(bad)?;
    let bpp = rd_u16(dib, 14).ok_or_else(bad)?;
    let compression = rd_u32(dib, 16).ok_or_else(bad)?;
    let clr_used = rd_u32(dib, 32).ok_or_else(bad)? as usize;
    if width <= 0 || height == 0 {
        return Err("dimensões de DIB inválidas".into());
    }
    if !matches!(bpp, 24 | 32) || !matches!(compression, 0 | 3 | 6) {
        return Err(format!(
            "DIB {bpp} bpp compressão {compression} não suportado"
        ));
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let top_down = height < 0;

    // Máscaras RGBA (só 32 bpp) e onde começam os pixels.
    let mut masks = [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000];
    let mut data_off = hsize;
    if bpp == 32 && compression != 0 {
        if hsize >= 108 {
            for (i, m) in masks.iter_mut().enumerate().take(3) {
                *m = rd_u32(dib, 40 + 4 * i).ok_or_else(bad)?;
            }
            masks[3] = rd_u32(dib, 52).ok_or_else(bad)?;
        } else {
            let n = if compression == 6 { 4 } else { 3 };
            for (i, m) in masks.iter_mut().enumerate().take(n) {
                *m = rd_u32(dib, 40 + 4 * i).ok_or_else(bad)?;
            }
            if n == 3 {
                masks[3] = 0;
            }
            data_off += 4 * n;
        }
    } else if bpp == 24 && compression != 0 {
        return Err("DIB 24 bpp com máscaras não suportado".into());
    }
    data_off += clr_used * 4;

    let stride = (w * usize::from(bpp)).div_ceil(32) * 4;
    let need = data_off
        .checked_add(stride.checked_mul(h).ok_or_else(bad)?)
        .ok_or_else(bad)?;
    if dib.len() < need {
        return Err(bad());
    }

    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let src_row = if top_down { y } else { h - 1 - y };
        let row = &dib[data_off + src_row * stride..][..stride];
        for x in 0..w {
            if bpp == 24 {
                let p = &row[x * 3..x * 3 + 3];
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            } else {
                let px = u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().expect("4 bytes"));
                let a = if masks[3] == 0 {
                    255
                } else {
                    channel(px, masks[3])
                };
                rgba.extend_from_slice(&[
                    channel(px, masks[0]),
                    channel(px, masks[1]),
                    channel(px, masks[2]),
                    a,
                ]);
            }
        }
    }
    // BI_RGB de 32 bpp costuma ter o 4º byte zerado (sem alpha de verdade): torna opaco.
    if bpp == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }

    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Converte um PNG em CF_DIBV5 (BITMAPV5HEADER de 124 bytes + BGRA 32 bpp de baixo para cima).
pub fn png_to_dibv5(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut dec = png::Decoder::new(Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("PNG grande demais")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    let bgra = |i: usize| -> [u8; 4] {
        match info.color_type {
            png::ColorType::Rgba => [
                data[i * 4 + 2],
                data[i * 4 + 1],
                data[i * 4],
                data[i * 4 + 3],
            ],
            png::ColorType::Rgb => [data[i * 3 + 2], data[i * 3 + 1], data[i * 3], 255],
            png::ColorType::GrayscaleAlpha => {
                [data[i * 2], data[i * 2], data[i * 2], data[i * 2 + 1]]
            }
            // Grayscale (Indexed já foi expandido).
            _ => [data[i], data[i], data[i], 255],
        }
    };

    let image_size = w * h * 4;
    let mut out = Vec::with_capacity(124 + image_size);
    let mut put32 = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    put32(124); // bV5Size
    put32(w as u32);
    put32(h as u32); // positivo: de baixo para cima
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    let mut put32 = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    put32(3); // BI_BITFIELDS
    put32(image_size as u32);
    put32(0); // XPelsPerMeter
    put32(0); // YPelsPerMeter
    put32(0); // ClrUsed
    put32(0); // ClrImportant
    put32(0x00FF_0000); // R
    put32(0x0000_FF00); // G
    put32(0x0000_00FF); // B
    put32(0xFF00_0000); // A
    put32(0x7352_4742); // LCS_sRGB
    out.extend_from_slice(&[0u8; 36]); // endpoints
    out.extend_from_slice(&[0u8; 12]); // gamma
    let mut put32 = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    put32(4); // LCS_GM_IMAGES
    put32(0); // ProfileData
    put32(0); // ProfileSize
    put32(0); // Reserved
    debug_assert_eq!(out.len(), 124);
    for y in (0..h).rev() {
        for x in 0..w {
            out.extend_from_slice(&bgra(y * w + x));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- DROPFILES

/// Monta a estrutura DROPFILES (CF_HDROP): cabeçalho de 20 bytes (`pFiles`=20, `fWide`=1) seguido
/// dos caminhos em UTF-16 terminados em NUL, mais um NUL final (terminador duplo).
pub fn build_dropfiles<P: AsRef<Path>>(paths: &[P]) -> Vec<u8> {
    let mut out = Vec::with_capacity(22);
    out.extend_from_slice(&20u32.to_le_bytes()); // pFiles
    out.extend_from_slice(&[0u8; 8]); // pt
    out.extend_from_slice(&0u32.to_le_bytes()); // fNC
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in p.as_ref().to_string_lossy().encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Lê os caminhos de uma estrutura DROPFILES (wide ou ANSI/UTF-8).
pub fn parse_dropfiles(data: &[u8]) -> Vec<PathBuf> {
    let (Some(off), Some(wide)) = (rd_u32(data, 0), rd_u32(data, 16)) else {
        return Vec::new();
    };
    let Some(body) = data.get(off as usize..) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if wide != 0 {
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        for s in units.split(|&u| u == 0) {
            if s.is_empty() {
                break;
            }
            paths.push(PathBuf::from(String::from_utf16_lossy(s)));
        }
    } else {
        for s in body.split(|&b| b == 0) {
            if s.is_empty() {
                break;
            }
            paths.push(PathBuf::from(String::from_utf8_lossy(s).into_owned()));
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_roundtrip_and_offsets() {
        let frag = "<b>olá, mundo</b>";
        let s = build_cf_html(frag);
        let get = |name: &str| -> usize {
            let r = s.split(name).nth(1).unwrap();
            r[..10].parse().unwrap()
        };
        assert_eq!(get("StartHTML:"), s.find("<html>").unwrap());
        assert_eq!(get("EndHTML:"), s.len());
        assert_eq!(
            &s.as_bytes()[get("StartFragment:")..get("EndFragment:")],
            frag.as_bytes()
        );
        assert_eq!(parse_cf_html(s.as_bytes()).as_deref(), Some(frag));
        // com NUL final (como vem do clipboard)
        let mut with_nul = s.into_bytes();
        with_nul.push(0);
        assert_eq!(parse_cf_html(&with_nul).as_deref(), Some(frag));
    }

    #[test]
    fn cf_html_fallback_markers() {
        let raw = b"Version:0.9\r\nStartFragment:0000000000\r\nEndFragment:9999999999\r\n<html><!--StartFragment-->x<!--EndFragment--></html>";
        assert_eq!(parse_cf_html(raw).as_deref(), Some("x"));
        assert_eq!(parse_cf_html(b"lixo"), None);
    }

    fn dib32(w: i32, h: i32, px: &[[u8; 4]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&w.to_le_bytes());
        d.extend_from_slice(&h.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        for p in px {
            d.extend_from_slice(p);
        }
        d
    }

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut r = png::Decoder::new(Cursor::new(png_bytes))
            .read_info()
            .unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let i = r.next_frame(&mut buf).unwrap();
        (i.width, i.height, buf[..i.buffer_size()].to_vec())
    }

    #[test]
    fn dib_32bpp_bottom_up_to_png() {
        // BGRA; linha de baixo primeiro: (azul, verde) embaixo, (vermelho, branco) em cima.
        let dib = dib32(
            2,
            2,
            &[
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
            ],
        );
        let (w, h, px) = decode(&dib_to_png(&dib).unwrap());
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            px,
            vec![
                255, 0, 0, 255, 255, 255, 255, 255, // topo: vermelho, branco
                0, 0, 255, 255, 0, 255, 0, 255, // base: azul, verde
            ]
        );
    }

    #[test]
    fn dib_alpha_zero_becomes_opaque_and_truncated_fails() {
        let dib = dib32(1, 1, &[[1, 2, 3, 0]]);
        assert_eq!(decode(&dib_to_png(&dib).unwrap()).2, vec![3, 2, 1, 255]);
        assert!(dib_to_png(&dib[..dib.len() - 1]).is_err());
        assert!(dib_to_png(&[]).is_err());
    }

    #[test]
    fn dib_24bpp_row_padding() {
        let mut d = dib32(1, 2, &[]);
        d[14] = 24;
        // 1 px por linha = 3 bytes + 1 de padding; de baixo para cima.
        d.extend_from_slice(&[10, 20, 30, 0, 40, 50, 60, 0]);
        let (_, _, px) = decode(&dib_to_png(&d).unwrap());
        assert_eq!(px, vec![60, 50, 40, 255, 30, 20, 10, 255]);
    }

    #[test]
    fn png_dibv5_roundtrip() {
        let dib = dib32(2, 1, &[[1, 2, 3, 200], [4, 5, 6, 255]]);
        let png_bytes = dib_to_png(&dib).unwrap();
        let v5 = png_to_dibv5(&png_bytes).unwrap();
        assert_eq!(v5.len(), 124 + 8);
        assert_eq!(rd_u32(&v5, 0), Some(124));
        let back = dib_to_png(&v5).unwrap();
        assert_eq!(decode(&back), decode(&png_bytes));
    }

    #[test]
    fn dropfiles_wide_and_double_terminator() {
        let d = build_dropfiles(&[PathBuf::from("C:\\a\\é.txt"), PathBuf::from("D:\\b")]);
        assert_eq!(rd_u32(&d, 0), Some(20));
        assert_eq!(rd_u32(&d, 16), Some(1));
        assert_eq!(&d[d.len() - 4..], &[0, 0, 0, 0]);
        let units =
            ("C:\\a\\é.txt".encode_utf16().count() + 1) + ("D:\\b".encode_utf16().count() + 1) + 1;
        assert_eq!(d.len(), 20 + units * 2);
        assert_eq!(
            parse_dropfiles(&d),
            vec![PathBuf::from("C:\\a\\é.txt"), PathBuf::from("D:\\b")]
        );
        assert_eq!(build_dropfiles::<&str>(&[]), {
            let mut v = vec![20, 0, 0, 0];
            v.extend_from_slice(&[0; 12]);
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&[0, 0]);
            v
        });
    }

    #[test]
    fn dropfiles_ansi_parses() {
        let mut d = vec![0u8; 20];
        d[0] = 20;
        d.extend_from_slice(b"C:\\x\0C:\\y\0\0");
        assert_eq!(
            parse_dropfiles(&d),
            vec![PathBuf::from("C:\\x"), PathBuf::from("C:\\y")]
        );
    }

    #[test]
    fn anti_echo_decision() {
        assert!(!should_notify_change(&[13, 0xC123, 15], 0xC123));
        assert!(should_notify_change(&[13, 15], 0xC123));
        assert!(should_notify_change(&[], 0xC123));
        // marcador não registrado (0) nunca suprime
        assert!(should_notify_change(&[0], 0));
    }

    #[test]
    fn format_priority() {
        use ClipboardFormat::*;
        assert_eq!(pick_format(&[Text, Html, Png, FileList]), Some(FileList));
        assert_eq!(pick_format(&[Text, Html, Png]), Some(Html));
        assert_eq!(pick_format(&[Text, Png]), Some(Text));
        assert_eq!(pick_format(&[Html, Png]), Some(Png));
        assert_eq!(pick_format(&[Rtf]), Some(Rtf));
        assert_eq!(pick_format(&[]), None);
    }
}
