//! dono: M2
//! Injeção via CGEventPost (sentido inverso: o Mac é o destino). As funções de montagem de
//! evento (`*_params`, `ClickTracker`, `ScrollAccum`, `clamp_to_union`…) são puras e testadas;
//! só `MacInjector` chama o CoreGraphics (e precisa da permissão de Acessibilidade).
//! Scroll: dy > 0 = cima, dx > 0 = direita (convenção do core); o CG usa dx invertido.

use std::ffi::c_void;
use std::time::Instant;

use core_graphics::display::CGDisplay;
use core_graphics::geometry::CGPoint;
use core_graphics::sys::CGEventRef;
use relay_core::keymap::tables::scan_to_mac;
use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::{InputEvent, MediaKey, MouseButton, ScrollPhase};

// --- constantes do CoreGraphics -------------------------------------------------------------

const KCG_HID_EVENT_TAP: u32 = 0;
const KCG_EVENT_LEFT_DOWN: u32 = 1;
const KCG_EVENT_LEFT_UP: u32 = 2;
const KCG_EVENT_RIGHT_DOWN: u32 = 3;
const KCG_EVENT_RIGHT_UP: u32 = 4;
const KCG_EVENT_MOUSE_MOVED: u32 = 5;
const KCG_EVENT_LEFT_DRAGGED: u32 = 6;
const KCG_EVENT_RIGHT_DRAGGED: u32 = 7;
const KCG_EVENT_OTHER_DOWN: u32 = 25;
const KCG_EVENT_OTHER_UP: u32 = 26;
const KCG_EVENT_OTHER_DRAGGED: u32 = 27;

const FIELD_MOUSE_CLICK_STATE: u32 = 1;
const FIELD_MOUSE_DELTA_X: u32 = 4;
const FIELD_MOUSE_DELTA_Y: u32 = 5;
const FIELD_SCROLL_IS_CONTINUOUS: u32 = 88;
const FIELD_SCROLL_MOMENTUM_PHASE: u32 = 123;
const FIELD_SCROLL_PHASE: u32 = 99;
const KCG_SCROLL_UNIT_PIXEL: u32 = 0;

/// Janela e distância do duplo clique (padrão do macOS ≈ 500 ms; tolerância de 4 pt).
pub const DOUBLE_CLICK_MS: u64 = 500;
pub const DOUBLE_CLICK_DIST: f64 = 4.0;
/// Pixels por degrau de roda (120 unidades do protocolo).
pub const SCROLL_PX_PER_NOTCH: f64 = 30.0;
/// Máximo de unidades UTF-16 por evento de teclado com string Unicode.
const TYPE_TEXT_CHUNK: usize = 20;

// kCGEventFlagMask*
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_ALTERNATE: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;
const FLAG_CAPS_LOCK: u64 = 0x0001_0000;

// --- funções puras ---------------------------------------------------------------------------

/// Retângulo de tela em pontos: `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        }
    }

    fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x0 && p.0 < self.x1 && p.1 >= self.y0 && p.1 < self.y1
    }

    /// Ponto do retângulo mais próximo de `p` (a borda direita/inferior fica 1 pt para dentro).
    fn nearest(&self, p: (f64, f64)) -> (f64, f64) {
        (
            p.0.clamp(self.x0, (self.x1 - 1.0).max(self.x0)),
            p.1.clamp(self.y0, (self.y1 - 1.0).max(self.y0)),
        )
    }
}

/// Limita `p` à união das telas: dentro de alguma tela fica como está; fora, vai para o ponto
/// mais próximo de qualquer tela. Sem telas, devolve `p`.
pub fn clamp_to_union(p: (f64, f64), rects: &[Rect]) -> (f64, f64) {
    if rects.is_empty() || rects.iter().any(|r| r.contains(p)) {
        return p;
    }
    let dist2 = |q: (f64, f64)| (q.0 - p.0).powi(2) + (q.1 - p.1).powi(2);
    rects
        .iter()
        .map(|r| r.nearest(p))
        .min_by(|a, b| dist2(*a).total_cmp(&dist2(*b)))
        .unwrap_or(p)
}

/// Contagem de cliques (`kCGMouseEventClickState`): duplo/triplo clique por tempo e distância.
#[derive(Debug, Default)]
pub struct ClickTracker {
    last: Option<LastClick>,
    count: i64,
}

#[derive(Debug, Clone, Copy)]
struct LastClick {
    button: MouseButton,
    t_ms: u64,
    pos: (f64, f64),
}

impl ClickTracker {
    /// `down`: o botão foi pressionado em `pos` no instante `t_ms` (monotônico). Devolve o
    /// `clickState` a usar no evento de down e no up correspondente (o up passa `down = false`).
    pub fn register(&mut self, button: MouseButton, down: bool, t_ms: u64, pos: (f64, f64)) -> i64 {
        if !down {
            return self.count.max(1);
        }
        let continues = self.last.is_some_and(|l| {
            l.button == button
                && t_ms.saturating_sub(l.t_ms) <= DOUBLE_CLICK_MS
                && (pos.0 - l.pos.0).abs() <= DOUBLE_CLICK_DIST
                && (pos.1 - l.pos.1).abs() <= DOUBLE_CLICK_DIST
        });
        self.count = if continues { self.count + 1 } else { 1 };
        self.last = Some(LastClick { button, t_ms, pos });
        self.count
    }
}

/// `(tipo CGEventType, número do botão)` de um evento de botão.
pub fn button_params(button: MouseButton, down: bool) -> (u32, u32) {
    match (button, down) {
        (MouseButton::Left, true) => (KCG_EVENT_LEFT_DOWN, 0),
        (MouseButton::Left, false) => (KCG_EVENT_LEFT_UP, 0),
        (MouseButton::Right, true) => (KCG_EVENT_RIGHT_DOWN, 1),
        (MouseButton::Right, false) => (KCG_EVENT_RIGHT_UP, 1),
        (b, true) => (KCG_EVENT_OTHER_DOWN, other_button_number(b)),
        (b, false) => (KCG_EVENT_OTHER_UP, other_button_number(b)),
    }
}

fn other_button_number(b: MouseButton) -> u32 {
    match b {
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
        MouseButton::Left => 0,
        MouseButton::Right => 1,
    }
}

/// Tipo do evento de movimento: arrasto se algum botão está pressionado.
pub fn move_event_type(left: bool, right: bool, other: bool) -> u32 {
    if left {
        KCG_EVENT_LEFT_DRAGGED
    } else if right {
        KCG_EVENT_RIGHT_DRAGGED
    } else if other {
        KCG_EVENT_OTHER_DRAGGED
    } else {
        KCG_EVENT_MOUSE_MOVED
    }
}

/// Acumula o scroll em 1/120 de degrau e entrega pixels inteiros, sem perder o resto.
#[derive(Debug, Default)]
pub struct ScrollAccum {
    rx: f64,
    ry: f64,
}

impl ScrollAccum {
    /// `dy > 0` = cima, `dx > 0` = direita (core). Devolve `(vertical, horizontal)` do CG,
    /// onde o horizontal é invertido (CG: positivo = esquerda).
    pub fn convert(&mut self, dx: i16, dy: i16) -> (i32, i32) {
        self.ry += f64::from(dy) * SCROLL_PX_PER_NOTCH / 120.0;
        self.rx += f64::from(dx) * SCROLL_PX_PER_NOTCH / 120.0;
        let v = self.ry.trunc();
        let h = self.rx.trunc();
        self.ry -= v;
        self.rx -= h;
        (v as i32, -(h as i32))
    }
}

/// `(kCGScrollWheelEventScrollPhase, kCGScrollWheelEventMomentumPhase)` de uma fase.
pub fn scroll_phase_fields(phase: ScrollPhase) -> (i64, i64) {
    match phase {
        ScrollPhase::None => (0, 0),
        ScrollPhase::Began => (1, 0),
        ScrollPhase::Changed => (2, 0),
        ScrollPhase::Ended => (4, 0),
        ScrollPhase::Momentum => (0, 2),
    }
}

/// Parâmetros de um evento de tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyParams {
    pub keycode: u16,
    pub down: bool,
}

/// Scan code (set 1; 0xE0xx = estendida) → keycode do macOS pela tabela do core.
pub fn key_params(scan: u16, down: bool) -> Option<KeyParams> {
    scan_to_mac(scan).map(|keycode| KeyParams { keycode, down })
}

/// Máscara de flag que a tecla modificadora (kVK_*) liga/desliga.
pub fn modifier_flag(keycode: u16) -> Option<u64> {
    match keycode {
        0x38 | 0x3C => Some(FLAG_SHIFT),
        0x3B | 0x3E => Some(FLAG_CONTROL),
        0x3A | 0x3D => Some(FLAG_ALTERNATE),
        0x37 | 0x36 => Some(FLAG_COMMAND),
        0x39 => Some(FLAG_CAPS_LOCK),
        _ => None,
    }
}

/// NX_KEYTYPE_* de uma tecla de mídia (inversa de `tables::media_from_nx`).
pub fn media_to_nx(key: MediaKey) -> u32 {
    match key {
        MediaKey::VolumeUp => 0,
        MediaKey::VolumeDown => 1,
        MediaKey::Mute => 7,
        MediaKey::PlayPause => 16,
        MediaKey::Next => 17,
        MediaKey::Previous => 18,
    }
}

/// `data1` do evento de sistema (subtype 8): `keytype << 16 | (0xA down / 0xB up) << 8`.
pub fn media_data1(key: MediaKey, down: bool) -> isize {
    ((media_to_nx(key) as isize) << 16) | ((if down { 0xA } else { 0xB }) << 8)
}

/// Divide o texto em blocos de até `max` unidades UTF-16 sem partir um par substituto.
pub fn utf16_chunks(text: &str, max: usize) -> Vec<Vec<u16>> {
    let mut out: Vec<Vec<u16>> = Vec::new();
    let mut cur: Vec<u16> = Vec::new();
    let mut buf = [0u16; 2];
    for ch in text.chars() {
        let units = ch.encode_utf16(&mut buf);
        if cur.len() + units.len() > max {
            out.push(std::mem::take(&mut cur));
        }
        cur.extend_from_slice(units);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// --- FFI --------------------------------------------------------------------------------------

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> CGEventRef;
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        ty: u32,
        pos: CGPoint,
        button: u32,
    ) -> CGEventRef;
    fn CGEventCreateScrollWheelEvent2(
        source: *mut c_void,
        units: u32,
        count: u32,
        w1: i32,
        w2: i32,
        w3: i32,
    ) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventKeyboardSetUnicodeString(event: CGEventRef, len: usize, s: *const u16);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

/// Evento CG possuído (liberado no drop).
struct OwnedEvent(CGEventRef);

impl OwnedEvent {
    fn new(raw: CGEventRef) -> Result<Self, InjectError> {
        if raw.is_null() {
            Err(InjectError::Platform("CGEventCreate devolveu nulo".into()))
        } else {
            Ok(Self(raw))
        }
    }

    fn set_flags(&self, flags: u64) {
        // SAFETY: ponteiro válido enquanto `self` vive.
        unsafe { CGEventSetFlags(self.0, flags) }
    }

    fn set_field(&self, field: u32, value: i64) {
        // SAFETY: idem.
        unsafe { CGEventSetIntegerValueField(self.0, field, value) }
    }

    fn post(&self) {
        // SAFETY: idem; CGEventPost não toma posse do evento.
        unsafe { CGEventPost(KCG_HID_EVENT_TAP, self.0) }
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        // SAFETY: único dono do evento criado por CGEventCreate*.
        unsafe { core_foundation::base::CFRelease(self.0 as *const c_void) }
    }
}

fn current_location() -> Result<(f64, f64), InjectError> {
    // SAFETY: CGEventCreate(NULL) cria um evento com a posição atual do cursor.
    let ev = OwnedEvent::new(unsafe { CGEventCreate(std::ptr::null_mut()) })?;
    // SAFETY: ponteiro válido.
    let p = unsafe { CGEventGetLocation(ev.0) };
    Ok((p.x, p.y))
}

fn display_rects() -> Vec<Rect> {
    CGDisplay::active_displays()
        .unwrap_or_default()
        .into_iter()
        .map(|id| {
            let b = CGDisplay::new(id).bounds();
            Rect::new(b.origin.x, b.origin.y, b.size.width, b.size.height)
        })
        .collect()
}

// --- injetor ----------------------------------------------------------------------------------

/// Injeta input no Mac. Exige Acessibilidade; sem ela o CGEventPost é descartado em silêncio.
#[derive(Debug)]
pub struct MacInjector {
    clicks: ClickTracker,
    scroll: ScrollAccum,
    flags: u64,
    left: bool,
    right: bool,
    other: bool,
    epoch: Instant,
}

impl Default for MacInjector {
    fn default() -> Self {
        Self {
            clicks: ClickTracker::default(),
            scroll: ScrollAccum::default(),
            flags: 0,
            left: false,
            right: false,
            other: false,
            epoch: Instant::now(),
        }
    }
}

impl MacInjector {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn post_move(&mut self, target: (f64, f64), delta: (i64, i64)) -> Result<(), InjectError> {
        let ty = move_event_type(self.left, self.right, self.other);
        let pos = CGPoint::new(target.0, target.1);
        // SAFETY: parâmetros válidos; fonte nula = estado padrão.
        let ev =
            OwnedEvent::new(unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), ty, pos, 0) })?;
        ev.set_field(FIELD_MOUSE_DELTA_X, delta.0);
        ev.set_field(FIELD_MOUSE_DELTA_Y, delta.1);
        ev.set_flags(self.flags);
        ev.post();
        Ok(())
    }

    fn mouse_move(&mut self, dx: i16, dy: i16) -> Result<(), InjectError> {
        let (cx, cy) = current_location()?;
        let target = clamp_to_union((cx + f64::from(dx), cy + f64::from(dy)), &display_rects());
        self.post_move(target, (i64::from(dx), i64::from(dy)))
    }

    fn mouse_abs(&mut self, x: i32, y: i32) -> Result<(), InjectError> {
        let (cx, cy) = current_location()?;
        let target = clamp_to_union((f64::from(x), f64::from(y)), &display_rects());
        self.post_move(target, ((target.0 - cx) as i64, (target.1 - cy) as i64))
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<(), InjectError> {
        let pos = current_location()?;
        let state = self.clicks.register(button, down, self.now_ms(), pos);
        let (ty, num) = button_params(button, down);
        // SAFETY: parâmetros válidos.
        let ev = OwnedEvent::new(unsafe {
            CGEventCreateMouseEvent(std::ptr::null_mut(), ty, CGPoint::new(pos.0, pos.1), num)
        })?;
        ev.set_field(FIELD_MOUSE_CLICK_STATE, state);
        ev.set_flags(self.flags);
        ev.post();
        match button {
            MouseButton::Left => self.left = down,
            MouseButton::Right => self.right = down,
            _ => self.other = down,
        }
        Ok(())
    }

    fn scroll(&mut self, dx: i16, dy: i16, phase: ScrollPhase) -> Result<(), InjectError> {
        let (v, h) = self.scroll.convert(dx, dy);
        let (sp, mp) = scroll_phase_fields(phase);
        if v == 0 && h == 0 && sp == 0 && mp == 0 {
            return Ok(());
        }
        // SAFETY: parâmetros válidos.
        let ev = OwnedEvent::new(unsafe {
            CGEventCreateScrollWheelEvent2(std::ptr::null_mut(), KCG_SCROLL_UNIT_PIXEL, 2, v, h, 0)
        })?;
        ev.set_field(FIELD_SCROLL_IS_CONTINUOUS, 1);
        ev.set_field(FIELD_SCROLL_PHASE, sp);
        ev.set_field(FIELD_SCROLL_MOMENTUM_PHASE, mp);
        ev.set_flags(self.flags);
        ev.post();
        Ok(())
    }

    fn key(&mut self, scan: u16, down: bool) -> Result<(), InjectError> {
        let p = key_params(scan, down)
            .ok_or_else(|| InjectError::Platform(format!("scan 0x{scan:04X} sem keycode")))?;
        if let Some(flag) = modifier_flag(p.keycode) {
            if down {
                self.flags |= flag;
            } else {
                self.flags &= !flag;
            }
        }
        // SAFETY: parâmetros válidos.
        let ev = OwnedEvent::new(unsafe {
            CGEventCreateKeyboardEvent(std::ptr::null_mut(), p.keycode, p.down)
        })?;
        ev.set_flags(self.flags);
        ev.post();
        Ok(())
    }

    fn media(&mut self, key: MediaKey, down: bool) -> Result<(), InjectError> {
        use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
        use objc2_foundation::NSPoint;
        let flags = NSEventModifierFlags(if down { 0xA00 } else { 0xB00 });
        let ns = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::SystemDefined,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            0,
            None,
            8,
            media_data1(key, down),
            -1,
        )
        .ok_or_else(|| InjectError::Platform("NSEvent de mídia não criado".into()))?;
        let cg = ns
            .CGEvent()
            .ok_or_else(|| InjectError::Platform("NSEvent sem CGEvent".into()))?;
        // SAFETY: `cg` é um CGEventRef válido enquanto o Retained vive.
        unsafe { CGEventPost(KCG_HID_EVENT_TAP, &*cg as *const _ as CGEventRef) };
        Ok(())
    }
}

impl InputInjector for MacInjector {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        match *event {
            InputEvent::MouseMove { dx, dy } => self.mouse_move(dx, dy),
            InputEvent::MouseAbs { x, y } => self.mouse_abs(x, y),
            InputEvent::Button { button, down } => self.button(button, down),
            InputEvent::Scroll { dx, dy, phase } => self.scroll(dx, dy, phase),
            InputEvent::Key { scan, down } => self.key(scan, down),
            InputEvent::Media { key, down } => self.media(key, down),
        }
    }

    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        for chunk in utf16_chunks(text, TYPE_TEXT_CHUNK) {
            for down in [true, false] {
                // SAFETY: parâmetros válidos; keycode 0 + string Unicode sobrescreve a tecla.
                let ev = OwnedEvent::new(unsafe {
                    CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, down)
                })?;
                // SAFETY: `chunk` vive durante a chamada; o CG copia a string.
                unsafe { CGEventKeyboardSetUnicodeString(ev.0, chunk.len(), chunk.as_ptr()) };
                ev.post();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_screens() -> Vec<Rect> {
        vec![
            Rect::new(0.0, 0.0, 1440.0, 900.0),
            Rect::new(1440.0, 0.0, 1920.0, 1080.0),
        ]
    }

    #[test]
    fn clamp_dentro_de_qualquer_tela_nao_muda() {
        let r = two_screens();
        assert_eq!(clamp_to_union((1500.0, 1000.0), &r), (1500.0, 1000.0));
        assert_eq!(clamp_to_union((10.0, 10.0), &r), (10.0, 10.0));
    }

    #[test]
    fn clamp_fora_vai_para_o_ponto_mais_proximo_da_uniao() {
        let r = two_screens();
        // acima do topo
        assert_eq!(clamp_to_union((100.0, -50.0), &r), (100.0, 0.0));
        // à direita de tudo
        assert_eq!(clamp_to_union((9000.0, 500.0), &r), (3359.0, 500.0));
        // abaixo da tela 1, mas ao lado da 2 (canto vazio): fica na tela mais próxima
        assert_eq!(clamp_to_union((1000.0, 950.0), &r), (1000.0, 899.0));
        assert_eq!(clamp_to_union((5.0, 5.0), &[]), (5.0, 5.0));
    }

    #[test]
    fn duplo_e_triplo_clique_por_tempo_e_distancia() {
        let mut t = ClickTracker::default();
        let b = MouseButton::Left;
        assert_eq!(t.register(b, true, 0, (10.0, 10.0)), 1);
        assert_eq!(t.register(b, false, 40, (10.0, 10.0)), 1);
        assert_eq!(t.register(b, true, 200, (12.0, 11.0)), 2);
        assert_eq!(t.register(b, false, 230, (12.0, 11.0)), 2);
        assert_eq!(t.register(b, true, 400, (12.0, 11.0)), 3);
        // demorou demais
        assert_eq!(
            t.register(b, true, 400 + DOUBLE_CLICK_MS + 1, (12.0, 11.0)),
            1
        );
    }

    #[test]
    fn clique_longe_ou_de_outro_botao_reinicia() {
        let mut t = ClickTracker::default();
        assert_eq!(t.register(MouseButton::Left, true, 0, (0.0, 0.0)), 1);
        assert_eq!(t.register(MouseButton::Left, true, 100, (50.0, 0.0)), 1);
        assert_eq!(t.register(MouseButton::Right, true, 150, (50.0, 0.0)), 1);
        assert_eq!(t.register(MouseButton::Right, true, 200, (50.0, 0.0)), 2);
    }

    #[test]
    fn botoes_mapeiam_tipo_e_numero() {
        assert_eq!(button_params(MouseButton::Left, true), (1, 0));
        assert_eq!(button_params(MouseButton::Left, false), (2, 0));
        assert_eq!(button_params(MouseButton::Right, true), (3, 1));
        assert_eq!(button_params(MouseButton::Right, false), (4, 1));
        assert_eq!(button_params(MouseButton::Middle, true), (25, 2));
        assert_eq!(button_params(MouseButton::X1, false), (26, 3));
        assert_eq!(button_params(MouseButton::X2, true), (25, 4));
    }

    #[test]
    fn movimento_vira_arrasto_com_botao_pressionado() {
        assert_eq!(move_event_type(false, false, false), 5);
        assert_eq!(move_event_type(true, false, false), 6);
        assert_eq!(move_event_type(false, true, false), 7);
        assert_eq!(move_event_type(false, false, true), 27);
    }

    #[test]
    fn scroll_converte_para_pixels_e_inverte_dx() {
        let mut s = ScrollAccum::default();
        // um degrau para cima e para a direita
        assert_eq!(s.convert(120, 120), (30, -30));
        // dy < 0 = baixo; dx < 0 = esquerda
        assert_eq!(s.convert(-120, -120), (-30, 30));
    }

    #[test]
    fn scroll_acumula_o_resto_sem_perder() {
        let mut s = ScrollAccum::default();
        // 1/120 de degrau = 0,25 px: só emite a cada 4 chamadas
        let total: i32 = (0..8).map(|_| s.convert(0, 1).0).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn fases_de_scroll() {
        assert_eq!(scroll_phase_fields(ScrollPhase::None), (0, 0));
        assert_eq!(scroll_phase_fields(ScrollPhase::Began), (1, 0));
        assert_eq!(scroll_phase_fields(ScrollPhase::Ended), (4, 0));
        assert_eq!(scroll_phase_fields(ScrollPhase::Momentum), (0, 2));
    }

    #[test]
    fn scan_vira_keycode_pela_tabela_do_core() {
        // A: scan 0x1E → kVK_ANSI_A (0x00); Seta esquerda estendida
        assert_eq!(
            key_params(0x1E, true),
            Some(KeyParams {
                keycode: 0x00,
                down: true
            })
        );
        assert_eq!(key_params(0xE04B, false).map(|p| p.keycode), Some(0x7B));
        assert_eq!(key_params(0xFFFF, true), None);
    }

    #[test]
    fn modificadores_ligam_flags() {
        assert_eq!(modifier_flag(0x37), Some(FLAG_COMMAND));
        assert_eq!(modifier_flag(0x38), Some(FLAG_SHIFT));
        assert_eq!(modifier_flag(0x3B), Some(FLAG_CONTROL));
        assert_eq!(modifier_flag(0x3A), Some(FLAG_ALTERNATE));
        assert_eq!(modifier_flag(0x00), None);
    }

    #[test]
    fn midia_ida_e_volta_com_a_tabela_do_core() {
        use relay_core::keymap::tables::media_from_nx;
        for k in [
            MediaKey::VolumeUp,
            MediaKey::VolumeDown,
            MediaKey::Mute,
            MediaKey::PlayPause,
            MediaKey::Next,
            MediaKey::Previous,
        ] {
            assert_eq!(media_from_nx(media_to_nx(k)), Some(k));
        }
        assert_eq!(media_data1(MediaKey::PlayPause, true), (16 << 16) | 0xA00);
        assert_eq!(media_data1(MediaKey::PlayPause, false), (16 << 16) | 0xB00);
    }

    #[test]
    fn texto_em_blocos_utf16_sem_partir_par_substituto() {
        let chunks = utf16_chunks("abc😀def", 4);
        assert_eq!(chunks.len(), 3);
        assert_eq!(String::from_utf16(&chunks[0]).unwrap(), "abc");
        assert_eq!(String::from_utf16(&chunks[1]).unwrap(), "😀de");
        assert_eq!(String::from_utf16(&chunks[2]).unwrap(), "f");
        assert!(utf16_chunks("", 20).is_empty());
    }

    /// Posta eventos reais: exige Acessibilidade e move o cursor. Rodar à mão:
    /// `cargo test -p relay-platform-macos -- --ignored inject_real`.
    #[test]
    #[ignore = "exige permissão de Acessibilidade e mexe no cursor"]
    fn inject_real() {
        let mut inj = MacInjector::default();
        inj.inject(&InputEvent::MouseMove { dx: 5, dy: 5 }).unwrap();
        inj.inject(&InputEvent::MouseMove { dx: -5, dy: -5 })
            .unwrap();
    }
}
