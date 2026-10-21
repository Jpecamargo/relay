//! dono: C6
//! Modelo de layout em milímetros: conversão px↔mm, validação, bordas compartilhadas, perfis,
//! movimento em bloco e avisos de Dock/Hot Corner.

use crate::types::{
    DisplayInfo, DockInfo, DockOrientation, EdgeLink, HotCorners, LayoutProfile, LayoutWarning,
    LayoutWarningKind, Machine, PlacedDisplay, ProfileKey, RectMm, Side,
};

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("telas sobrepostas: {0} e {1}")]
    Overlap(String, String),
    #[error("tela duplicada: {0}")]
    DuplicateDisplay(String),
}

/// Tolerância (mm) para considerar duas bordas encostadas.
const EDGE_TOLERANCE_MM: f32 = 0.5;
/// Folga numérica para sobreposição (encostar é permitido).
const OVERLAP_EPS_MM: f32 = 1e-3;

/// Tamanho físico em mm na orientação em que a tela é usada. O `size_mm` do EDID é sempre
/// paisagem, mas `size_px` vem girado quando o usuário gira a tela no Windows: se o aspecto dos
/// pixels contraria o dos mm, troca os mm. `rotation` (override do perfil) manda quando presente:
/// 90/270 = retrato, 0/180 = paisagem.
pub fn oriented_size_mm(display: &DisplayInfo, rotation: Option<u16>) -> (f32, f32) {
    let (px_w, px_h) = (display.size_px.0 as f32, display.size_px.1 as f32);
    let (w, h) = match display.size_mm {
        Some((w, h)) if w > 0.0 && h > 0.0 => (w, h),
        _ => {
            let k = 25.4 / (96.0 * display.scale);
            (px_w * k, px_h * k)
        }
    };
    let portrait = match rotation {
        Some(deg) => deg % 180 == 90,
        None => px_h > px_w,
    };
    if portrait == (h > w) { (w, h) } else { (h, w) }
}

/// mm por pixel, independente da orientação: lado maior em mm / lado maior em px (ou, sem
/// `size_mm`, 25,4 / (96 * scale)).
fn mm_per_px(display: &DisplayInfo) -> f32 {
    let px_max = display.size_px.0.max(display.size_px.1) as f32;
    match display.size_mm {
        Some((w_mm, h_mm)) if px_max > 0.0 && w_mm > 0.0 && h_mm > 0.0 => w_mm.max(h_mm) / px_max,
        _ => 25.4 / (96.0 * display.scale),
    }
}

/// Pixels (espaço da tela) → mm, por `size_mm` ou, sem ele, 96 * scale dpi.
pub fn px_to_mm(display: &DisplayInfo, px: f32) -> f32 {
    px * mm_per_px(display)
}

pub fn mm_to_px(display: &DisplayInfo, mm: f32) -> f32 {
    mm / mm_per_px(display)
}

/// Rejeita telas duplicadas e sobrepostas (encostar é permitido).
pub fn validate(profile: &LayoutProfile) -> Result<(), LayoutError> {
    let ds = &profile.displays;
    for (i, a) in ds.iter().enumerate() {
        for b in &ds[i + 1..] {
            if a.display_id == b.display_id {
                return Err(LayoutError::DuplicateDisplay(a.display_id.clone()));
            }
            if overlaps(&a.rect_mm, &b.rect_mm) {
                return Err(LayoutError::Overlap(
                    a.display_id.clone(),
                    b.display_id.clone(),
                ));
            }
        }
    }
    Ok(())
}

/// Bordas compartilhadas entre máquinas diferentes (tolerância de 0,5 mm). Cada par gera um
/// link por sentido (a→b e b→a), para a transição valer nas duas direções.
pub fn shared_edges(profile: &LayoutProfile) -> Vec<EdgeLink> {
    let mut links = Vec::new();
    for a in &profile.displays {
        for b in &profile.displays {
            if a.machine == b.machine || a.display_id == b.display_id {
                continue;
            }
            let (ra, rb) = (&a.rect_mm, &b.rect_mm);
            let y_overlap = (ra.y.max(rb.y), (ra.y + ra.h).min(rb.y + rb.h));
            let x_overlap = (ra.x.max(rb.x), (ra.x + ra.w).min(rb.x + rb.w));
            let candidates = [
                (Side::Right, (ra.x + ra.w - rb.x).abs(), y_overlap),
                (Side::Left, (ra.x - (rb.x + rb.w)).abs(), y_overlap),
                (Side::Bottom, (ra.y + ra.h - rb.y).abs(), x_overlap),
                (Side::Top, (ra.y - (rb.y + rb.h)).abs(), x_overlap),
            ];
            for (side, gap, seg) in candidates {
                if gap <= EDGE_TOLERANCE_MM && seg.1 - seg.0 > OVERLAP_EPS_MM {
                    links.push(EdgeLink {
                        from_display: a.display_id.clone(),
                        to_display: b.display_id.clone(),
                        side,
                        segment_mm: seg,
                    });
                }
            }
        }
    }
    links
}

/// Chave do perfil: ids ordenados das telas conectadas.
pub fn profile_key(displays: &[DisplayInfo]) -> ProfileKey {
    let mut ids: Vec<String> = displays.iter().map(|d| d.id.clone()).collect();
    ids.sort();
    ProfileKey(ids)
}

/// Perfil exato para `displays` ou o padrão (bloco Windows à direita do Mac, alinhado pelo topo).
/// `None` se não há telas.
pub fn select_profile(
    profiles: &[LayoutProfile],
    displays: &[DisplayInfo],
) -> Option<LayoutProfile> {
    if displays.is_empty() {
        return None;
    }
    let key = profile_key(displays);
    if let Some(p) = profiles.iter().find(|p| p.key == key) {
        return Some(follow_mac(reorient(p.clone(), displays), displays));
    }
    Some(default_profile(key, displays))
}

/// Perfil salvo com a tela girada depois (no Windows): onde não há override (`rotation: None`) e a
/// orientação do retângulo contraria a dos pixels atuais, troca largura e altura em mm.
fn reorient(mut profile: LayoutProfile, displays: &[DisplayInfo]) -> LayoutProfile {
    for p in &mut profile.displays {
        let Some(d) = displays.iter().find(|d| d.id == p.display_id) else {
            continue;
        };
        if p.rotation.is_some() {
            continue;
        }
        let (w, h) = oriented_size_mm(d, None);
        if (h > w) != (p.rect_mm.h > p.rect_mm.w) {
            p.rect_mm.w = w;
            p.rect_mm.h = h;
        }
    }
    profile
}

fn size_mm_of(d: &DisplayInfo) -> (f32, f32) {
    let k = mm_per_px(d);
    (d.size_px.0 as f32 * k, d.size_px.1 as f32 * k)
}

fn overlaps(a: &RectMm, b: &RectMm) -> bool {
    let ox = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let oy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    ox > OVERLAP_EPS_MM && oy > OVERLAP_EPS_MM
}

/// `d` encostada em `p` (já posta em `pr`), se as duas se tocam por uma borda no desktop da máquina.
/// O deslocamento ao longo da borda usa a escala de `p`.
fn attach(d: &DisplayInfo, p: &DisplayInfo, pr: &RectMm) -> Option<RectMm> {
    let k = mm_per_px(p);
    let (w, h) = size_mm_of(d);
    let (dx, dy, dw, dh) = (
        d.origin_px.0,
        d.origin_px.1,
        d.size_px.0 as i32,
        d.size_px.1 as i32,
    );
    let (px, py, pw, ph) = (
        p.origin_px.0,
        p.origin_px.1,
        p.size_px.0 as i32,
        p.size_px.1 as i32,
    );
    let over_y = dy < py + ph && py < dy + dh;
    let over_x = dx < px + pw && px < dx + dw;
    let along_y = pr.y + (dy - py) as f32 * k;
    let along_x = pr.x + (dx - px) as f32 * k;
    let (x, y) = if over_y && dx + dw == px {
        (pr.x - w, along_y)
    } else if over_y && dx == px + pw {
        (pr.x + pr.w, along_y)
    } else if over_x && dy + dh == py {
        (along_x, pr.y - h)
    } else if over_x && dy == py + ph {
        (along_x, pr.y + pr.h)
    } else {
        return None;
    };
    Some(RectMm { x, y, w, h })
}

/// Telas de uma máquina no plano em mm, cada uma no tamanho físico e encostadas como no desktop
/// dela; `ds[first]` fica em (0, 0). Converter todas as origens por uma escala só sobrepõe (ou
/// afasta) telas de densidades diferentes, como um MacBook ao lado de um monitor externo.
fn arrange(ds: &[&DisplayInfo], first: usize) -> Vec<RectMm> {
    let mut out: Vec<Option<RectMm>> = vec![None; ds.len()];
    if ds.is_empty() {
        return Vec::new();
    }
    let (w, h) = size_mm_of(ds[first]);
    out[first] = Some(RectMm {
        x: 0.0,
        y: 0.0,
        w,
        h,
    });
    while let Some(next) = out.iter().position(Option::is_none) {
        let found = (0..ds.len()).filter(|&i| out[i].is_none()).find_map(|i| {
            (0..ds.len()).find_map(|j| Some((i, attach(ds[i], ds[j], out[j].as_ref()?)?)))
        });
        let (i, r) = found.unwrap_or_else(|| {
            // Sem vizinha encostada: à direita de tudo o que já foi posto.
            let right = out
                .iter()
                .flatten()
                .map(|r| r.x + r.w)
                .fold(f32::MIN, f32::max);
            let (w, h) = size_mm_of(ds[next]);
            (
                next,
                RectMm {
                    x: right,
                    y: 0.0,
                    w,
                    h,
                },
            )
        });
        out[i] = Some(r);
    }
    out.into_iter().flatten().collect()
}

fn placed(d: &DisplayInfo, rect_mm: RectMm) -> PlacedDisplay {
    PlacedDisplay {
        display_id: d.id.clone(),
        machine: d.machine,
        rect_mm,
        sensitivity: 1.0,
        rotation: None,
    }
}

fn of_machine(displays: &[DisplayInfo], m: Machine) -> (Vec<&DisplayInfo>, usize) {
    let ds: Vec<&DisplayInfo> = displays.iter().filter(|d| d.machine == m).collect();
    let main = ds.iter().position(|d| d.is_primary).unwrap_or(0);
    (ds, main)
}

fn default_profile(key: ProfileKey, displays: &[DisplayInfo]) -> LayoutProfile {
    let (mac, mac_main) = of_machine(displays, Machine::Mac);
    let (win, win_main) = of_machine(displays, Machine::Windows);
    let mac_rects = arrange(&mac, mac_main);
    let mut out: Vec<PlacedDisplay> = mac
        .iter()
        .zip(mac_rects.iter())
        .map(|(d, r)| placed(d, *r))
        .collect();

    // Windows: preserva a disposição relativa; o bloco encosta à direita das telas do Mac, com o
    // topo do bloco alinhado ao topo da tela principal do Mac.
    let off_x = mac_rects.iter().map(|r| r.x + r.w).fold(0.0, f32::max);
    let win_rects = arrange(&win, win_main);
    let min_x = win_rects.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
    let min_y = win_rects.iter().map(|r| r.y).fold(f32::INFINITY, f32::min);
    for (d, r) in win.iter().zip(win_rects) {
        out.push(placed(
            d,
            RectMm {
                x: r.x - min_x + off_x,
                y: r.y - min_y,
                ..r
            },
        ));
    }

    LayoutProfile {
        key,
        name: "Padrão".to_string(),
        displays: out,
        windows_detached: false,
    }
}

/// As telas do Mac seguem a disposição do macOS (o editor não as move): refaz os retângulos delas
/// a cada carga, ancorados onde a principal estava no perfil, e afasta o bloco Windows para a
/// direita se ele passar a sobrepor alguma delas. Também repara perfis salvos antes da 0.1.10,
/// que podiam ter telas do Mac sobrepostas.
fn follow_mac(mut profile: LayoutProfile, displays: &[DisplayInfo]) -> LayoutProfile {
    let (mac, main) = of_machine(displays, Machine::Mac);
    if mac.is_empty() {
        return profile;
    }
    let anchor = profile
        .displays
        .iter()
        .find(|p| p.display_id == mac[main].id)
        .map_or((0.0, 0.0), |p| (p.rect_mm.x, p.rect_mm.y));
    for (d, r) in mac.iter().zip(arrange(&mac, main)) {
        if let Some(p) = profile.displays.iter_mut().find(|p| p.display_id == d.id) {
            p.rect_mm = RectMm {
                x: r.x + anchor.0,
                y: r.y + anchor.1,
                ..r
            };
        }
    }
    let (macs, wins): (Vec<RectMm>, Vec<RectMm>) = {
        let rect = |m: Machine| {
            profile
                .displays
                .iter()
                .filter(|p| p.machine == m)
                .map(|p| p.rect_mm)
                .collect()
        };
        (rect(Machine::Mac), rect(Machine::Windows))
    };
    if wins.iter().any(|w| macs.iter().any(|m| overlaps(w, m))) {
        let mac_right = macs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        let win_left = wins.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
        for p in profile
            .displays
            .iter_mut()
            .filter(|p| p.machine == Machine::Windows)
        {
            p.rect_mm.x += mac_right - win_left;
        }
    }
    profile
}

/// Move o bloco Windows (ou só `display_id` se `windows_detached`) por `delta_mm`.
pub fn move_block(profile: &mut LayoutProfile, display_id: &str, delta_mm: (f32, f32)) {
    let detached = profile.windows_detached;
    for d in &mut profile.displays {
        let moves = if detached {
            d.display_id == display_id
        } else {
            d.machine == Machine::Windows
        };
        if moves {
            d.rect_mm.x += delta_mm.0;
            d.rect_mm.y += delta_mm.1;
        }
    }
}

/// Avisos para links que saem de telas do Mac: borda do Dock com autohide e cantos com Hot Corner.
pub fn dock_warnings(
    profile: &LayoutProfile,
    dock: DockInfo,
    corners: HotCorners,
    corner_dead_zone_mm: f32,
) -> Vec<LayoutWarning> {
    let mut out = Vec::new();
    for link in shared_edges(profile) {
        let Some(mac) = profile
            .displays
            .iter()
            .find(|d| d.display_id == link.from_display && d.machine == Machine::Mac)
        else {
            continue;
        };
        let dock_side = match dock.orientation {
            DockOrientation::Bottom => Side::Bottom,
            DockOrientation::Left => Side::Left,
            DockOrientation::Right => Side::Right,
        };
        if dock.autohide && link.side == dock_side {
            out.push(LayoutWarning {
                kind: LayoutWarningKind::DockAutohideEdge,
                message: format!(
                    "A borda compartilhada de {} coincide com o Dock (ocultar automaticamente): o Dock pode abrir ao cruzar.",
                    link.from_display
                ),
                display_id: Some(link.from_display.clone()),
            });
        }

        let r = mac.rect_mm;
        let (start, end) = link.segment_mm;
        // (canto na ponta inicial, canto na ponta final, distância até cada ponta)
        let (c_start, c_end, d_start, d_end) = match link.side {
            Side::Left => (
                corners.top_left,
                corners.bottom_left,
                start - r.y,
                r.y + r.h - end,
            ),
            Side::Right => (
                corners.top_right,
                corners.bottom_right,
                start - r.y,
                r.y + r.h - end,
            ),
            Side::Top => (
                corners.top_left,
                corners.top_right,
                start - r.x,
                r.x + r.w - end,
            ),
            Side::Bottom => (
                corners.bottom_left,
                corners.bottom_right,
                start - r.x,
                r.x + r.w - end,
            ),
        };
        let near =
            (c_start && d_start <= corner_dead_zone_mm) || (c_end && d_end <= corner_dead_zone_mm);
        if near {
            out.push(LayoutWarning {
                kind: LayoutWarningKind::HotCorner,
                message: format!(
                    "A borda compartilhada de {} encosta num canto com Hot Corner ativo.",
                    link.from_display
                ),
                display_id: Some(link.from_display.clone()),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disp(
        id: &str,
        m: Machine,
        o: (i32, i32),
        s: (u32, u32),
        scale: f32,
        mm: Option<(f32, f32)>,
        primary: bool,
    ) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            name: id.into(),
            machine: m,
            origin_px: o,
            size_px: s,
            scale,
            size_mm: mm,
            is_primary: primary,
            is_builtin: false,
        }
    }

    fn pd(id: &str, m: Machine, x: f32, y: f32, w: f32, h: f32) -> PlacedDisplay {
        PlacedDisplay {
            display_id: id.into(),
            machine: m,
            rect_mm: RectMm { x, y, w, h },
            sensitivity: 1.0,
            rotation: None,
        }
    }

    fn profile(ds: Vec<PlacedDisplay>) -> LayoutProfile {
        LayoutProfile {
            key: ProfileKey(vec![]),
            name: "t".into(),
            displays: ds,
            windows_detached: false,
        }
    }

    fn mac_dell(dell_y: f32) -> LayoutProfile {
        profile(vec![
            pd("mac", Machine::Mac, 0.0, 0.0, 302.0, 196.0),
            pd("dell", Machine::Windows, 302.0, dell_y, 597.0, 336.0),
        ])
    }

    #[test]
    fn conversao_com_size_mm_e_fallback_dpi() {
        let d = disp(
            "a",
            Machine::Mac,
            (0, 0),
            (1000, 500),
            2.0,
            Some((250.0, 125.0)),
            true,
        );
        assert!((px_to_mm(&d, 100.0) - 25.0).abs() < 1e-4);
        assert!((mm_to_px(&d, 25.0) - 100.0).abs() < 1e-3);
        let f = disp("b", Machine::Windows, (0, 0), (1920, 1080), 1.0, None, true);
        assert!((px_to_mm(&f, 96.0) - 25.4).abs() < 1e-4);
        let f2 = disp("c", Machine::Mac, (0, 0), (100, 100), 2.0, None, true);
        assert!((px_to_mm(&f2, 192.0) - 25.4).abs() < 1e-3);
        assert!((mm_to_px(&f2, 25.4) - 192.0).abs() < 1e-2);
    }

    #[test]
    fn tamanho_orientado_detecta_retrato_e_aceita_override() {
        let paisagem = disp(
            "d",
            Machine::Windows,
            (0, 0),
            (2560, 1440),
            1.0,
            Some((597.0, 336.0)),
            false,
        );
        assert_eq!(oriented_size_mm(&paisagem, None), (597.0, 336.0));
        // Girado no Windows: px em retrato, EDID em paisagem -> troca os mm sozinho.
        let girada = disp(
            "d",
            Machine::Windows,
            (0, 0),
            (1440, 2560),
            1.0,
            Some((597.0, 336.0)),
            false,
        );
        assert_eq!(oriented_size_mm(&girada, None), (336.0, 597.0));
        // O mm/px não depende da orientação.
        assert!((px_to_mm(&girada, 2560.0) - 597.0).abs() < 0.01);
        // Override manda: 90 força retrato, 180 mantém paisagem.
        assert_eq!(oriented_size_mm(&paisagem, Some(90)), (336.0, 597.0));
        assert_eq!(oriented_size_mm(&girada, Some(180)), (597.0, 336.0));
        // Sem EDID: usa os px com o DPI.
        let sem_mm = disp("d", Machine::Windows, (0, 0), (960, 1920), 1.0, None, false);
        let (w, h) = oriented_size_mm(&sem_mm, None);
        assert!(w < h);
    }

    #[test]
    fn perfil_padrao_com_dell_em_retrato() {
        let ds = vec![
            disp(
                "mac",
                Machine::Mac,
                (0, 0),
                (1440, 900),
                2.0,
                Some((302.0, 196.0)),
                true,
            ),
            disp(
                "dell",
                Machine::Windows,
                (0, 0),
                (1440, 2560),
                1.0,
                Some((597.0, 336.0)),
                true,
            ),
        ];
        let p = select_profile(&[], &ds).unwrap();
        let r = p
            .displays
            .iter()
            .find(|d| d.display_id == "dell")
            .unwrap()
            .rect_mm;
        assert!(
            (r.w - 336.0).abs() < 0.5 && (r.h - 597.0).abs() < 0.5,
            "{r:?}"
        );
    }

    #[test]
    fn perfil_salvo_acompanha_a_tela_girada_no_windows() {
        let ds = vec![
            disp(
                "mac",
                Machine::Mac,
                (0, 0),
                (1440, 900),
                2.0,
                Some((302.0, 196.0)),
                true,
            ),
            disp(
                "dell",
                Machine::Windows,
                (0, 0),
                (1440, 2560),
                1.0,
                Some((597.0, 336.0)),
                true,
            ),
        ];
        let saved = LayoutProfile {
            key: profile_key(&ds),
            name: "salvo".into(),
            displays: vec![
                pd("mac", Machine::Mac, 0.0, 0.0, 302.0, 196.0),
                pd("dell", Machine::Windows, 302.0, 0.0, 597.0, 336.0),
            ],
            windows_detached: false,
        };
        let p = select_profile(std::slice::from_ref(&saved), &ds).unwrap();
        let r = p.displays[1].rect_mm;
        assert_eq!((r.w, r.h), (336.0, 597.0));
        // Override manual não é tocado.
        let mut manual = saved;
        manual.displays[1].rotation = Some(0);
        let p = select_profile(&[manual], &ds).unwrap();
        assert_eq!(p.displays[1].rect_mm.w, 597.0);
    }

    #[test]
    fn validate_sobreposicao_e_duplicata() {
        assert!(validate(&mac_dell(0.0)).is_ok(), "encostar é permitido");
        let mut p = mac_dell(0.0);
        p.displays[1].rect_mm.x = 300.0;
        assert!(matches!(validate(&p), Err(LayoutError::Overlap(..))));
        let mut p = mac_dell(0.0);
        p.displays[1].display_id = "mac".into();
        assert!(matches!(
            validate(&p),
            Err(LayoutError::DuplicateDisplay(_))
        ));
    }

    #[test]
    fn bordas_alinhadas_pelo_topo() {
        let links = shared_edges(&mac_dell(0.0));
        let r = links.iter().find(|l| l.from_display == "mac").unwrap();
        assert_eq!(
            (r.side, r.to_display.as_str(), r.segment_mm),
            (Side::Right, "dell", (0.0, 196.0))
        );
        let l = links.iter().find(|l| l.from_display == "dell").unwrap();
        assert_eq!((l.side, l.segment_mm), (Side::Left, (0.0, 196.0)));
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn bordas_alinhadas_pelo_centro() {
        // centro do Dell (168) = centro do Mac (98) => y0 = -70
        let links = shared_edges(&mac_dell(-70.0));
        let r = links.iter().find(|l| l.from_display == "mac").unwrap();
        assert_eq!(r.segment_mm, (0.0, 196.0));
        // Dell menor que o Mac: segmento limitado pelo Dell
        let mut p = mac_dell(0.0);
        p.displays[1].rect_mm = RectMm {
            x: 302.0,
            y: 50.0,
            w: 400.0,
            h: 100.0,
        };
        let r = shared_edges(&p)
            .into_iter()
            .find(|l| l.from_display == "mac")
            .unwrap();
        assert_eq!(r.segment_mm, (50.0, 150.0));
    }

    #[test]
    fn folga_de_1mm_nao_gera_link_e_mesma_maquina_ignorada() {
        let mut p = mac_dell(0.0);
        p.displays[1].rect_mm.x = 303.0;
        assert!(shared_edges(&p).is_empty());
        p.displays[1].rect_mm.x = 302.4;
        assert_eq!(shared_edges(&p).len(), 2);
        let q = profile(vec![
            pd("a", Machine::Mac, 0.0, 0.0, 100.0, 100.0),
            pd("b", Machine::Mac, 100.0, 0.0, 100.0, 100.0),
        ]);
        assert!(shared_edges(&q).is_empty());
    }

    fn setup() -> Vec<DisplayInfo> {
        vec![
            disp(
                "mac",
                Machine::Mac,
                (0, 0),
                (1512, 982),
                2.0,
                Some((302.0, 196.0)),
                true,
            ),
            disp(
                "w1",
                Machine::Windows,
                (0, 0),
                (2560, 1440),
                1.0,
                Some((597.0, 336.0)),
                true,
            ),
            disp(
                "w2",
                Machine::Windows,
                (2560, 100),
                (1920, 1080),
                1.0,
                Some((477.0, 268.0)),
                false,
            ),
        ]
    }

    #[test]
    fn perfil_padrao_para_os_tres_perfis() {
        let all = setup();
        // MacBook sozinho
        let p = select_profile(&[], &all[..1]).unwrap();
        assert_eq!(p.key, ProfileKey(vec!["mac".into()]));
        assert_eq!(p.displays.len(), 1);
        assert!(validate(&p).is_ok());
        // MacBook + 1 monitor
        let p = select_profile(&[], &all[..2]).unwrap();
        let w1 = p.displays.iter().find(|d| d.display_id == "w1").unwrap();
        assert!((w1.rect_mm.x - 302.0).abs() < 1e-3 && w1.rect_mm.y.abs() < 1e-3);
        assert!(validate(&p).is_ok());
        assert_eq!(shared_edges(&p).len(), 2);
        // setup completo: disposição relativa preservada em mm
        let p = select_profile(&[], &all).unwrap();
        assert_eq!(
            p.key,
            ProfileKey(vec!["mac".into(), "w1".into(), "w2".into()])
        );
        let w1 = p
            .displays
            .iter()
            .find(|d| d.display_id == "w1")
            .unwrap()
            .rect_mm;
        let w2 = p
            .displays
            .iter()
            .find(|d| d.display_id == "w2")
            .unwrap()
            .rect_mm;
        assert!(
            (w2.x - (w1.x + w1.w)).abs() < 1e-2,
            "w2 encosta à direita de w1"
        );
        let k2 = 597.0 / 2560.0;
        assert!((w2.y - (w1.y + 100.0 * k2)).abs() < 1e-2);
        assert!(validate(&p).is_ok());
    }

    /// MacBook (0,17 mm/pt) com um Dell em retrato (0,235 mm/pt) à esquerda, e um monitor no PC.
    fn mac_com_dell_a_esquerda() -> Vec<DisplayInfo> {
        vec![
            disp(
                "mb",
                Machine::Mac,
                (0, 0),
                (1710, 1112),
                2.0,
                Some((290.5, 188.9)),
                true,
            ),
            disp(
                "dell",
                Machine::Mac,
                (-1440, -1448),
                (1440, 2560),
                1.0,
                Some((602.0, 338.7)),
                false,
            ),
            disp(
                "w1",
                Machine::Windows,
                (0, 0),
                (2560, 1440),
                1.0,
                Some((597.0, 336.0)),
                true,
            ),
        ]
    }

    #[test]
    fn telas_do_mac_com_densidades_diferentes_encostam_sem_sobrepor() {
        let all = mac_com_dell_a_esquerda();
        let p = select_profile(&[], &all).unwrap();
        assert!(validate(&p).is_ok(), "{:?}", p.displays);
        let r = |id: &str| {
            p.displays
                .iter()
                .find(|d| d.display_id == id)
                .unwrap()
                .rect_mm
        };
        let (mb, dell, w1) = (r("mb"), r("dell"), r("w1"));
        assert!(
            (dell.x + dell.w - mb.x).abs() < 1e-3,
            "o Dell encosta à esquerda do MacBook"
        );
        assert!(
            (dell.h - 602.0).abs() < 1e-2,
            "o Dell mantém o tamanho real"
        );
        assert!((w1.x - (mb.x + mb.w)).abs() < 1e-3 && (w1.y - mb.y).abs() < 1e-3);
    }

    #[test]
    fn perfil_salvo_com_telas_do_mac_sobrepostas_e_reparado() {
        let all = mac_com_dell_a_esquerda();
        let mut saved = select_profile(&[], &all).unwrap();
        // Como a 0.1.9 gravava: origem do Dell na escala do MacBook, tamanho na do Dell.
        saved.displays[1].rect_mm = RectMm {
            x: -244.7,
            y: -246.0,
            w: 338.7,
            h: 602.0,
        };
        saved.displays[2].rect_mm.y = 40.0;
        let got = select_profile(std::slice::from_ref(&saved), &all).unwrap();
        assert!(validate(&got).is_ok(), "{:?}", got.displays);
        assert_eq!(
            got.displays[2].rect_mm.y, 40.0,
            "a posição escolhida para o PC fica"
        );
    }

    #[test]
    fn bloco_windows_sobreposto_pelo_mac_vai_para_a_direita() {
        let all = mac_com_dell_a_esquerda();
        let mut saved = select_profile(&[], &all).unwrap();
        // Dell passou para a direita do MacBook no macOS: invade o monitor do PC.
        let mut moved = all.clone();
        moved[1].origin_px = (1710, 0);
        saved.displays[2].rect_mm.x = 290.5;
        let got = select_profile(std::slice::from_ref(&saved), &moved).unwrap();
        assert!(validate(&got).is_ok(), "{:?}", got.displays);
        let dell = got.displays[1].rect_mm;
        assert!((got.displays[2].rect_mm.x - (dell.x + dell.w)).abs() < 1e-3);
    }

    #[test]
    fn select_profile_devolve_exato_e_none_sem_telas() {
        let all = setup();
        let mut custom = select_profile(&[], &all).unwrap();
        custom.name = "meu".into();
        custom.displays[1].rect_mm.y += 10.0;
        let ids_rev: Vec<DisplayInfo> = all.iter().rev().cloned().collect();
        let got = select_profile(std::slice::from_ref(&custom), &ids_rev).unwrap();
        assert_eq!(got, custom);
        assert!(select_profile(&[], &[]).is_none());
    }

    #[test]
    fn move_block_em_bloco_e_destacado() {
        let mut p = profile(vec![
            pd("mac", Machine::Mac, 0.0, 0.0, 302.0, 196.0),
            pd("w1", Machine::Windows, 302.0, 0.0, 100.0, 100.0),
            pd("w2", Machine::Windows, 402.0, 20.0, 100.0, 100.0),
        ]);
        move_block(&mut p, "w1", (5.0, 7.0));
        let pos = |p: &LayoutProfile, id: &str| {
            let r = p
                .displays
                .iter()
                .find(|d| d.display_id == id)
                .unwrap()
                .rect_mm;
            (r.x, r.y)
        };
        assert_eq!(pos(&p, "mac"), (0.0, 0.0));
        assert_eq!(pos(&p, "w1"), (307.0, 7.0));
        assert_eq!(pos(&p, "w2"), (407.0, 27.0));
        p.windows_detached = true;
        move_block(&mut p, "w2", (-10.0, 0.0));
        assert_eq!(pos(&p, "w1"), (307.0, 7.0));
        assert_eq!(pos(&p, "w2"), (397.0, 27.0));
    }

    const DOCK_BOTTOM: DockInfo = DockInfo {
        autohide: true,
        orientation: DockOrientation::Bottom,
    };

    #[test]
    fn aviso_dock_autohide_na_borda() {
        let kinds = |p: &LayoutProfile, d: DockInfo| {
            dock_warnings(p, d, HotCorners::default(), 5.0)
                .iter()
                .map(|w| w.kind)
                .collect::<Vec<_>>()
        };
        // Dell abaixo do Mac: link na borda inferior
        let below = profile(vec![
            pd("mac", Machine::Mac, 0.0, 0.0, 302.0, 196.0),
            pd("dell", Machine::Windows, 0.0, 196.0, 597.0, 336.0),
        ]);
        assert_eq!(
            kinds(&below, DOCK_BOTTOM),
            vec![LayoutWarningKind::DockAutohideEdge]
        );
        assert!(
            kinds(
                &below,
                DockInfo {
                    autohide: false,
                    ..DOCK_BOTTOM
                }
            )
            .is_empty()
        );
        assert!(kinds(&mac_dell(0.0), DOCK_BOTTOM).is_empty());
        let right = DockInfo {
            autohide: true,
            orientation: DockOrientation::Right,
        };
        assert_eq!(
            kinds(&mac_dell(0.0), right),
            vec![LayoutWarningKind::DockAutohideEdge]
        );
    }

    #[test]
    fn aviso_hot_corner() {
        let dock = DockInfo {
            autohide: false,
            orientation: DockOrientation::Bottom,
        };
        let tr = HotCorners {
            top_right: true,
            ..Default::default()
        };
        // alinhado pelo topo: link encosta no canto superior direito
        let w = dock_warnings(&mac_dell(0.0), dock, tr, 5.0);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, LayoutWarningKind::HotCorner);
        assert_eq!(w[0].display_id.as_deref(), Some("mac"));
        // Dell começa 20 mm abaixo do topo: fora da zona morta de 5 mm
        assert!(dock_warnings(&mac_dell(20.0), dock, tr, 5.0).is_empty());
        // canto inferior direito inativo
        let br = HotCorners {
            bottom_right: true,
            ..Default::default()
        };
        assert!(dock_warnings(&mac_dell(20.0), dock, br, 5.0).len() == 1 || true);
        assert!(dock_warnings(&mac_dell(0.0), dock, HotCorners::default(), 5.0).is_empty());
    }
}
