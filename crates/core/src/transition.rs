//! dono: C7
//! Motor de transição de borda: entrada proporcional, parede, empurrão/modificador, zona morta,
//! retorno no receptor, pulo direto e trava.
//!
//! O relógio é injetável: todo método que depende de tempo recebe o `Instant` atual, então os
//! testes usam um relógio sintético (base + `Duration`).

use std::time::{Duration, Instant};

use crate::layout::{px_to_mm, shared_edges};
use crate::types::{DisplayInfo, LayoutProfile, Machine, Modifier, RectMm, Side, TransitionConfig};

#[derive(Debug, Clone, PartialEq)]
pub enum LocalDecision {
    Stay,
    /// Entra na tela `display_id` (do outro lado, ou do Mac em `jump_to`) em `entry_px`, no
    /// espaço da própria tela.
    Enter {
        display_id: String,
        entry_px: (i32, i32),
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoteDecision {
    Stay,
    Return {
        mac_display: String,
        point_px: (i32, i32),
    },
}

/// Tela com geometria em px (espaço da máquina) e no plano do layout (mm).
#[derive(Debug)]
struct Disp {
    id: String,
    machine: Machine,
    origin: (i32, i32),
    size: (i32, i32),
    rect: RectMm,
    mm_per_px: f32,
    builtin: bool,
}

/// Borda compartilhada com índices nas telas.
#[derive(Debug)]
struct Link {
    from: usize,
    to: usize,
    side: Side,
    seg: (f32, f32),
}

/// Empurrão em andamento: (índice do link, início).
type Push = Option<(usize, Instant)>;

#[derive(Debug)]
pub struct TransitionEngine {
    config: TransitionConfig,
    disps: Vec<Disp>,
    links: Vec<Link>,
    locked: bool,
    local_push: Push,
    remote_push: Push,
}

fn horizontal(side: Side) -> bool {
    matches!(side, Side::Left | Side::Right)
}

/// `true` quando o empurrão já dura `dwell_ms`; inicia/zera o contador conforme o link.
fn dwell_elapsed(push: &mut Push, link: usize, now: Instant, dwell_ms: u32) -> bool {
    if dwell_ms == 0 {
        *push = None;
        return true;
    }
    let start = match *push {
        Some((l, s)) if l == link => s,
        _ => {
            *push = Some((link, now));
            now
        }
    };
    if now.saturating_duration_since(start) >= Duration::from_millis(u64::from(dwell_ms)) {
        *push = None;
        true
    } else {
        false
    }
}

impl TransitionEngine {
    pub fn new(
        config: TransitionConfig,
        profile: LayoutProfile,
        displays: Vec<DisplayInfo>,
    ) -> Self {
        let mut disps = Vec::new();
        for d in &displays {
            if let Some(p) = profile.displays.iter().find(|p| p.display_id == d.id) {
                disps.push(Disp {
                    id: d.id.clone(),
                    machine: d.machine,
                    origin: d.origin_px,
                    size: (d.size_px.0 as i32, d.size_px.1 as i32),
                    rect: p.rect_mm,
                    mm_per_px: px_to_mm(d, 1.0),
                    builtin: d.is_builtin,
                });
            }
        }
        let idx = |id: &str| disps.iter().position(|d| d.id == id);
        let links = shared_edges(&profile)
            .into_iter()
            .filter_map(|e| {
                Some(Link {
                    from: idx(&e.from_display)?,
                    to: idx(&e.to_display)?,
                    side: e.side,
                    seg: e.segment_mm,
                })
            })
            .collect();
        Self {
            config,
            disps,
            links,
            locked: false,
            local_push: None,
            remote_push: None,
        }
    }

    /// Posição ao longo da borda `side` de `d` (px) → mm absolutos no plano do layout.
    fn along_mm(d: &Disp, side: Side, pos: (i32, i32)) -> f32 {
        if horizontal(side) {
            d.rect.y + (pos.1 - d.origin.1) as f32 * d.mm_per_px
        } else {
            d.rect.x + (pos.0 - d.origin.0) as f32 * d.mm_per_px
        }
    }

    /// `true` se `pos` está na zona morta de canto da borda `side` de `d`.
    fn in_dead_zone(&self, d: &Disp, side: Side, pos: (i32, i32)) -> bool {
        let dz = self.config.corner_dead_zone_mm;
        if dz <= 0.0 {
            return false;
        }
        let (off, len) = if horizontal(side) {
            ((pos.1 - d.origin.1) as f32 * d.mm_per_px, d.rect.h)
        } else {
            ((pos.0 - d.origin.0) as f32 * d.mm_per_px, d.rect.w)
        };
        off < dz || len - off < dz
    }

    /// Ponto de chegada em `to` para o mm `along` ao cruzar `side` (borda de saída da origem).
    fn entry_point(to: &Disp, side: Side, along: f32) -> (i32, i32) {
        let clamp = |v: f32, n: i32| (v.round() as i32).clamp(0, (n - 1).max(0));
        let (w, h) = to.size;
        match side {
            Side::Right | Side::Left => {
                let y = clamp((along - to.rect.y) / to.mm_per_px, h);
                let x = if side == Side::Right { 0 } else { w - 1 };
                (to.origin.0 + x, to.origin.1 + y)
            }
            Side::Bottom | Side::Top => {
                let x = clamp((along - to.rect.x) / to.mm_per_px, w);
                let y = if side == Side::Bottom { 0 } else { h - 1 };
                (to.origin.0 + x, to.origin.1 + y)
            }
        }
    }

    /// Movimento local no Mac. Sem alocação no caminho `Stay`.
    pub fn on_local_move(
        &mut self,
        display_id: &str,
        pos_px: (i32, i32),
        delta: (i32, i32),
        modifiers: &[Modifier],
        now: Instant,
    ) -> LocalDecision {
        let Some(from) = self.disps.iter().position(|d| d.id == display_id) else {
            self.local_push = None;
            return LocalDecision::Stay;
        };
        if self.locked || self.disps[from].machine != Machine::Mac {
            self.local_push = None;
            return LocalDecision::Stay;
        }
        let d = &self.disps[from];
        let (nx, ny) = (pos_px.0 + delta.0, pos_px.1 + delta.1);
        let (x1, y1) = (d.origin.0 + d.size.0, d.origin.1 + d.size.1);
        let mut hit = None;
        for (i, l) in self.links.iter().enumerate() {
            if l.from != from {
                continue;
            }
            let exits = match l.side {
                Side::Right => delta.0 > 0 && nx >= x1,
                Side::Left => delta.0 < 0 && nx < d.origin.0,
                Side::Bottom => delta.1 > 0 && ny >= y1,
                Side::Top => delta.1 < 0 && ny < d.origin.1,
            };
            let along = Self::along_mm(d, l.side, pos_px);
            if exits && along >= l.seg.0 && along <= l.seg.1 {
                hit = Some((i, along));
                break;
            }
        }
        let Some((li, along)) = hit else {
            self.local_push = None;
            return LocalDecision::Stay;
        };
        let side = self.links[li].side;
        let modifier_ok = self
            .config
            .require_modifier
            .is_none_or(|m| modifiers.contains(&m));
        if !modifier_ok || self.in_dead_zone(d, side, pos_px) {
            self.local_push = None;
            return LocalDecision::Stay;
        }
        if !dwell_elapsed(&mut self.local_push, li, now, self.config.dwell_ms) {
            return LocalDecision::Stay;
        }
        let to = &self.disps[self.links[li].to];
        LocalDecision::Enter {
            display_id: to.id.clone(),
            entry_px: Self::entry_point(to, side, along),
        }
    }

    /// Lado receptor: posição do cursor no desktop virtual do Windows. O receptor não vê o
    /// modificador do Mac; vale dwell e zona morta.
    pub fn on_remote_cursor(&mut self, pos_px: (i32, i32), now: Instant) -> RemoteDecision {
        if self.locked {
            self.remote_push = None;
            return RemoteDecision::Stay;
        }
        let mut hit = None;
        for (i, l) in self.links.iter().enumerate() {
            let d = &self.disps[l.from];
            if d.machine != Machine::Windows {
                continue;
            }
            let (x0, y0) = d.origin;
            let (x1, y1) = (x0 + d.size.0, y0 + d.size.1);
            if pos_px.0 < x0 || pos_px.0 >= x1 || pos_px.1 < y0 || pos_px.1 >= y1 {
                continue;
            }
            let at_edge = match l.side {
                Side::Right => pos_px.0 >= x1 - 1,
                Side::Left => pos_px.0 <= x0,
                Side::Bottom => pos_px.1 >= y1 - 1,
                Side::Top => pos_px.1 <= y0,
            };
            let along = Self::along_mm(d, l.side, pos_px);
            if at_edge && along >= l.seg.0 && along <= l.seg.1 {
                hit = Some((i, along));
                break;
            }
        }
        let Some((li, along)) = hit else {
            self.remote_push = None;
            return RemoteDecision::Stay;
        };
        let l = &self.links[li];
        if self.in_dead_zone(&self.disps[l.from], l.side, pos_px) {
            self.remote_push = None;
            return RemoteDecision::Stay;
        }
        if !dwell_elapsed(&mut self.remote_push, li, now, self.config.dwell_ms) {
            return RemoteDecision::Stay;
        }
        let to = &self.disps[l.to];
        RemoteDecision::Return {
            mac_display: to.id.clone(),
            point_px: Self::entry_point(to, l.side, along),
        }
    }

    /// Centro da tela de número `n` (1-based, D19): 1 é a tela embutida do Mac; as demais, da
    /// esquerda para a direita e de cima para baixo no layout. `None` se `n` não existe.
    /// Travado, devolve `Some(Stay)`. Para o Mac, `Enter` traz o id e o centro da tela do Mac.
    pub fn jump_to(&self, n: u8) -> Option<LocalDecision> {
        let mut order: Vec<&Disp> = self.disps.iter().collect();
        order.sort_by(|a, b| {
            a.rect
                .x
                .total_cmp(&b.rect.x)
                .then(a.rect.y.total_cmp(&b.rect.y))
        });
        if let Some(i) = order.iter().position(|d| d.builtin) {
            let mac = order.remove(i);
            order.insert(0, mac);
        }
        let d = *order.get(usize::from(n).checked_sub(1)?)?;
        if self.locked {
            return Some(LocalDecision::Stay);
        }
        Some(LocalDecision::Enter {
            display_id: d.id.clone(),
            entry_px: (d.origin.0 + d.size.0 / 2, d.origin.1 + d.size.1 / 2),
        })
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.local_push = None;
        self.remote_push = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_alloc::measure;
    use crate::types::{PlacedDisplay, ProfileKey};

    const MAC: &str = "mac";
    const DELL: &str = "dell";

    fn display(
        id: &str,
        machine: Machine,
        size: (u32, u32),
        mm: (f32, f32),
        b: bool,
    ) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            name: id.into(),
            machine,
            origin_px: (0, 0),
            size_px: size,
            scale: 1.0,
            size_mm: Some(mm),
            is_primary: b,
            is_builtin: b,
        }
    }

    fn engine_with(cfg: TransitionConfig, dell_y: f32, dell_h: f32) -> TransitionEngine {
        let displays = vec![
            display(MAC, Machine::Mac, (1440, 900), (300.0, 190.0), true),
            display(DELL, Machine::Windows, (2560, 1440), (597.0, 336.0), false),
        ];
        let placed = |id: &str, machine, x, y, w, h| PlacedDisplay {
            display_id: id.into(),
            machine,
            rect_mm: RectMm { x, y, w, h },
            sensitivity: 1.0,
        };
        let profile = LayoutProfile {
            key: ProfileKey::default(),
            name: "t".into(),
            displays: vec![
                placed(MAC, Machine::Mac, 0.0, 0.0, 300.0, 190.0),
                placed(DELL, Machine::Windows, 300.0, dell_y, 597.0, dell_h),
            ],
            windows_detached: false,
        };
        TransitionEngine::new(cfg, profile, displays)
    }

    fn cfg(dwell_ms: u32) -> TransitionConfig {
        TransitionConfig {
            dwell_ms,
            require_modifier: None,
            corner_dead_zone_mm: 10.0,
        }
    }

    /// Dell mais alto que o Mac: o segmento compartilhado é a altura inteira do Mac.
    fn engine(dwell_ms: u32) -> TransitionEngine {
        engine_with(cfg(dwell_ms), -100.0, 336.0)
    }

    /// Mac a 456 px de y = 95 mm = 50% do segmento (0..190 mm).
    fn push_right(
        e: &mut TransitionEngine,
        y: i32,
        mods: &[Modifier],
        t: Instant,
    ) -> LocalDecision {
        e.on_local_move(MAC, (1439, y), (5, 0), mods, t)
    }

    #[test]
    fn entrada_proporcional() {
        let mut e = engine(0);
        let d = push_right(&mut e, 450, &[], Instant::now());
        // 450 px * (190/900... eixo x: 300/1440 mm/px) = 93,75 mm -> 193,75 mm no Dell -> ~830 px
        match d {
            LocalDecision::Enter {
                display_id,
                entry_px,
            } => {
                assert_eq!(display_id, DELL);
                assert_eq!(entry_px.0, 0);
                assert!((entry_px.1 - 830).abs() <= 2, "{entry_px:?}");
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn parede_sem_vizinho() {
        // Dell com 100 mm de altura (0..100): abaixo disso o Mac tem parede.
        let mut e = engine_with(cfg(0), 0.0, 100.0);
        let t = Instant::now();
        assert_ne!(push_right(&mut e, 200, &[], t), LocalDecision::Stay);
        assert_eq!(push_right(&mut e, 600, &[], t), LocalDecision::Stay);
    }

    #[test]
    fn dwell_zera_ao_sair_da_borda() {
        let mut e = engine(100);
        let t = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(push_right(&mut e, 450, &[], t), LocalDecision::Stay);
        assert_eq!(
            push_right(&mut e, 450, &[], t + ms(99)),
            LocalDecision::Stay
        );
        // sai da borda: zera
        assert_eq!(
            e.on_local_move(MAC, (1000, 450), (5, 0), &[], t + ms(150)),
            LocalDecision::Stay
        );
        assert_eq!(
            push_right(&mut e, 450, &[], t + ms(160)),
            LocalDecision::Stay
        );
        assert_eq!(
            push_right(&mut e, 450, &[], t + ms(259)),
            LocalDecision::Stay
        );
        assert!(matches!(
            push_right(&mut e, 450, &[], t + ms(260)),
            LocalDecision::Enter { .. }
        ));
    }

    #[test]
    fn exige_modificador() {
        let mut c = cfg(0);
        c.require_modifier = Some(Modifier::Option);
        let mut e = engine_with(c, -100.0, 336.0);
        let t = Instant::now();
        assert_eq!(push_right(&mut e, 450, &[], t), LocalDecision::Stay);
        assert_eq!(
            push_right(&mut e, 450, &[Modifier::Cmd], t),
            LocalDecision::Stay
        );
        assert!(matches!(
            push_right(&mut e, 450, &[Modifier::Option], t),
            LocalDecision::Enter { .. }
        ));
    }

    #[test]
    fn zona_morta_de_canto() {
        let mut e = engine(0);
        let t = Instant::now();
        // 40 px = 8,3 mm do topo; 880 px = 8,3 mm da base; 60 px = 12,5 mm
        assert_eq!(push_right(&mut e, 40, &[], t), LocalDecision::Stay);
        assert_eq!(push_right(&mut e, 880, &[], t), LocalDecision::Stay);
        assert!(matches!(
            push_right(&mut e, 60, &[], t),
            LocalDecision::Enter { .. }
        ));
    }

    #[test]
    fn retorno_no_receptor() {
        let mut e = engine(100);
        let t = Instant::now();
        let ms = Duration::from_millis;
        // Dell y=836 -> 195 mm relativos -> 95 mm absolutos -> Mac y ~456
        assert_eq!(e.on_remote_cursor((0, 836), t), RemoteDecision::Stay);
        assert_eq!(
            e.on_remote_cursor((0, 836), t + ms(99)),
            RemoteDecision::Stay
        );
        match e.on_remote_cursor((0, 836), t + ms(100)) {
            RemoteDecision::Return {
                mac_display,
                point_px,
            } => {
                assert_eq!(mac_display, MAC);
                assert_eq!(point_px.0, 1439);
                assert!((point_px.1 - 456).abs() <= 2, "{point_px:?}");
            }
            o => panic!("{o:?}"),
        }
        // fora da borda ligada ao Mac, e fora do segmento (parede)
        assert_eq!(e.on_remote_cursor((500, 836), t), RemoteDecision::Stay);
        let mut e0 = engine(0);
        assert_eq!(e0.on_remote_cursor((0, 300), t), RemoteDecision::Stay);
        // Dell alinhado ao topo do Mac: y=10 px = 2,3 mm do canto do Dell (zona morta)
        let mut e1 = engine_with(cfg(0), 0.0, 336.0);
        assert_eq!(e1.on_remote_cursor((0, 10), t), RemoteDecision::Stay);
        assert!(matches!(
            e1.on_remote_cursor((0, 100), t),
            RemoteDecision::Return { .. }
        ));
    }

    #[test]
    fn pulo_direto_e_trava() {
        let mut e = engine(0);
        let c = |d: Option<LocalDecision>| d;
        assert_eq!(
            c(e.jump_to(1)),
            Some(LocalDecision::Enter {
                display_id: MAC.into(),
                entry_px: (720, 450)
            })
        );
        assert_eq!(
            c(e.jump_to(2)),
            Some(LocalDecision::Enter {
                display_id: DELL.into(),
                entry_px: (1280, 720)
            })
        );
        assert_eq!(e.jump_to(3), None);
        assert_eq!(e.jump_to(0), None);
        e.set_locked(true);
        let t = Instant::now();
        assert_eq!(e.jump_to(2), Some(LocalDecision::Stay));
        assert_eq!(push_right(&mut e, 450, &[], t), LocalDecision::Stay);
        assert_eq!(e.on_remote_cursor((0, 836), t), RemoteDecision::Stay);
        e.set_locked(false);
        assert!(matches!(
            push_right(&mut e, 450, &[], t),
            LocalDecision::Enter { .. }
        ));
    }

    #[test]
    fn on_local_move_stay_nao_aloca() {
        let mut e = engine(100);
        let t = Instant::now();
        let (r, allocs, _) = measure(|| {
            let a = e.on_local_move(MAC, (500, 450), (3, 1), &[], t);
            let b = push_right(&mut e, 450, &[Modifier::Cmd], t);
            let c = push_right(&mut e, 600, &[], t);
            let d = push_right(&mut e, 450, &[], t);
            (a, b, c, d)
        });
        assert_eq!(allocs, 0);
        assert!(matches!(
            r,
            (
                LocalDecision::Stay,
                LocalDecision::Stay,
                LocalDecision::Stay,
                LocalDecision::Stay
            )
        ));
    }
}
