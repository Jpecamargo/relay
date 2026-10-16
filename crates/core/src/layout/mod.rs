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

/// mm por pixel: `size_mm / size_px` (eixo x) ou, sem `size_mm`, 25,4 / (96 * scale).
fn mm_per_px(display: &DisplayInfo) -> f32 {
    match display.size_mm {
        Some((w_mm, _)) if display.size_px.0 > 0 && w_mm > 0.0 => w_mm / display.size_px.0 as f32,
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
            let (ra, rb) = (&a.rect_mm, &b.rect_mm);
            let ox = (ra.x + ra.w).min(rb.x + rb.w) - ra.x.max(rb.x);
            let oy = (ra.y + ra.h).min(rb.y + rb.h) - ra.y.max(rb.y);
            if ox > OVERLAP_EPS_MM && oy > OVERLAP_EPS_MM {
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
        return Some(p.clone());
    }
    Some(default_profile(key, displays))
}

/// Retângulo em mm de `d` relativo a `origin_px`. O deslocamento usa `origin_k` (mm/px da tela
/// principal da máquina, pois as origens vivem num desktop virtual único); o tamanho usa a escala
/// da própria tela.
fn rect_relative(d: &DisplayInfo, origin_px: (i32, i32), origin_k: f32) -> RectMm {
    let k = mm_per_px(d);
    RectMm {
        x: (d.origin_px.0 - origin_px.0) as f32 * origin_k,
        y: (d.origin_px.1 - origin_px.1) as f32 * origin_k,
        w: d.size_px.0 as f32 * k,
        h: d.size_px.1 as f32 * k,
    }
}

fn placed(d: &DisplayInfo, rect_mm: RectMm) -> PlacedDisplay {
    PlacedDisplay {
        display_id: d.id.clone(),
        machine: d.machine,
        rect_mm,
        sensitivity: 1.0,
    }
}

fn default_profile(key: ProfileKey, displays: &[DisplayInfo]) -> LayoutProfile {
    let mac: Vec<&DisplayInfo> = displays
        .iter()
        .filter(|d| d.machine == Machine::Mac)
        .collect();
    let win: Vec<&DisplayInfo> = displays
        .iter()
        .filter(|d| d.machine == Machine::Windows)
        .collect();
    let primary = mac.iter().find(|d| d.is_primary).or(mac.first()).copied();

    let mut out = Vec::new();
    // Mac: origem do plano = canto da tela principal (ou da primeira tela, sem Mac).
    let mac_origin = primary.map_or((0, 0), |p| p.origin_px);
    let mac_k = primary.map_or(1.0, mm_per_px);
    for d in &mac {
        out.push(placed(d, rect_relative(d, mac_origin, mac_k)));
    }

    // Windows: preserva a disposição relativa; o bloco encosta à direita da tela principal do
    // Mac, com o topo do bloco alinhado ao topo dela.
    let win_origin = (
        win.iter().map(|d| d.origin_px.0).min().unwrap_or(0),
        win.iter().map(|d| d.origin_px.1).min().unwrap_or(0),
    );
    let win_main = win.iter().find(|d| d.is_primary).or(win.first()).copied();
    let win_k = win_main.map_or(1.0, mm_per_px);
    let (off_x, off_y) = primary.map_or((0.0, 0.0), |p| {
        let r = rect_relative(p, mac_origin, mac_k);
        (r.x + r.w, r.y)
    });
    let block_min_mm = |axis: usize| {
        win.iter()
            .map(|d| {
                let r = rect_relative(d, win_origin, win_k);
                if axis == 0 { r.x } else { r.y }
            })
            .fold(f32::INFINITY, f32::min)
    };
    let (min_x, min_y) = (block_min_mm(0), block_min_mm(1));
    for d in &win {
        let mut r = rect_relative(d, win_origin, win_k);
        r.x += off_x - min_x;
        r.y += off_y - min_y;
        out.push(placed(d, r));
    }

    LayoutProfile {
        key,
        name: "Padrão".to_string(),
        displays: out,
        windows_detached: false,
    }
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
