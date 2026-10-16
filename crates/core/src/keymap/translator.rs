//! dono: C5b
//! Tradutor de atalhos Mac→Windows (tabela do spec, regra geral, Cmd tocado, exceções por app,
//! overrides do config). Sem alocação em `translate`: tudo em `ArrayVec` e arrays.

use std::time::{Duration, Instant};

use arrayvec::ArrayVec;

use crate::types::{KeyCombo, KeymapConfig, KeymapRule, Modifier};

/// Capacidade da saída de uma tradução.
pub const MAX_TRANSLATED: usize = 8;

/// Evento de tecla já em scan code do Windows (Cmd chega como 0xE05B/0xE05C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub scan: u16,
    pub down: bool,
    pub autorepeat: bool,
}

// Máscaras de modificadores (ordem = índice de categoria: ctrl, alt, shift, cmd/win).
const CTRL: u8 = 1;
const ALT: u8 = 2;
const SHIFT: u8 = 4;
const CMD: u8 = 8; // na origem é o Cmd do Mac; no destino, a tecla Windows

const SC_LCTRL: u16 = 0x1D;
const SC_LALT: u16 = 0x38;
const SC_LSHIFT: u16 = 0x2A;
const SC_LWIN: u16 = 0xE05B;
const LEFT_SCANS: [u16; 4] = [SC_LCTRL, SC_LALT, SC_LSHIFT, SC_LWIN];

/// Posição (0..8) do scan entre os modificadores: categoria * 2 + (1 se direito).
pub(crate) fn modifier_slot(scan: u16) -> Option<u8> {
    Some(match scan {
        0x1D => 0,
        0xE01D => 1,
        0x38 => 2,
        0xE038 => 3,
        0x2A => 4,
        0x36 => 5,
        0xE05B => 6,
        0xE05C => 7,
        _ => return None,
    })
}

/// Converte uma máscara de teclas de modificador seguradas (bit = slot) em máscara de categorias.
pub(crate) fn mods_from_slots(slots: u8) -> u8 {
    (0..4).fold(0, |m, i| {
        if (slots >> (2 * i)) & 3 != 0 {
            m | (1 << i)
        } else {
            m
        }
    })
}

/// `KeyboardEvent.code` → scan code do Windows (set 1; 0xE0xx = estendida).
const CODE_SCANS: &[(&str, u16)] = &[
    ("Escape", 0x01),
    ("Digit1", 0x02),
    ("Digit2", 0x03),
    ("Digit3", 0x04),
    ("Digit4", 0x05),
    ("Digit5", 0x06),
    ("Digit6", 0x07),
    ("Digit7", 0x08),
    ("Digit8", 0x09),
    ("Digit9", 0x0A),
    ("Digit0", 0x0B),
    ("Minus", 0x0C),
    ("Equal", 0x0D),
    ("Backspace", 0x0E),
    ("Tab", 0x0F),
    ("KeyQ", 0x10),
    ("KeyW", 0x11),
    ("KeyE", 0x12),
    ("KeyR", 0x13),
    ("KeyT", 0x14),
    ("KeyY", 0x15),
    ("KeyU", 0x16),
    ("KeyI", 0x17),
    ("KeyO", 0x18),
    ("KeyP", 0x19),
    ("BracketLeft", 0x1A),
    ("BracketRight", 0x1B),
    ("Enter", 0x1C),
    ("KeyA", 0x1E),
    ("KeyS", 0x1F),
    ("KeyD", 0x20),
    ("KeyF", 0x21),
    ("KeyG", 0x22),
    ("KeyH", 0x23),
    ("KeyJ", 0x24),
    ("KeyK", 0x25),
    ("KeyL", 0x26),
    ("Semicolon", 0x27),
    ("Quote", 0x28),
    ("Backquote", 0x29),
    ("Backslash", 0x2B),
    ("KeyZ", 0x2C),
    ("KeyX", 0x2D),
    ("KeyC", 0x2E),
    ("KeyV", 0x2F),
    ("KeyB", 0x30),
    ("KeyN", 0x31),
    ("KeyM", 0x32),
    ("Comma", 0x33),
    ("Period", 0x34),
    ("Slash", 0x35),
    ("Space", 0x39),
    ("F1", 0x3B),
    ("F2", 0x3C),
    ("F3", 0x3D),
    ("F4", 0x3E),
    ("F5", 0x3F),
    ("F6", 0x40),
    ("F7", 0x41),
    ("F8", 0x42),
    ("F9", 0x43),
    ("F10", 0x44),
    ("F11", 0x57),
    ("F12", 0x58),
    ("Home", 0xE047),
    ("ArrowUp", 0xE048),
    ("PageUp", 0xE049),
    ("ArrowLeft", 0xE04B),
    ("ArrowRight", 0xE04D),
    ("End", 0xE04F),
    ("ArrowDown", 0xE050),
    ("PageDown", 0xE051),
    ("Insert", 0xE052),
    ("Delete", 0xE053),
];

/// `KeyboardEvent.code` ("KeyL", "Digit1") → scan code. `None` para código desconhecido.
pub(crate) fn code_to_scan(code: &str) -> Option<u16> {
    CODE_SCANS.iter().find(|(c, _)| *c == code).map(|&(_, s)| s)
}

fn modifier_bit(m: Modifier) -> u8 {
    match m {
        Modifier::Ctrl => CTRL,
        Modifier::Option => ALT,
        Modifier::Shift => SHIFT,
        Modifier::Cmd => CMD,
    }
}

/// Combo da UI/config → (máscara de modificadores, scan da tecla).
pub(crate) fn combo_to_scan(combo: &KeyCombo) -> Option<(u8, u16)> {
    let mods = combo.modifiers.iter().fold(0, |m, &x| m | modifier_bit(x));
    Some((mods, code_to_scan(&combo.key)?))
}

#[derive(Debug, Clone, Copy)]
struct Chord {
    mods: u8,
    key: u16,
}

#[derive(Debug, Clone)]
struct Rule {
    id: String,
    from_mods: u8,
    key: u16,
    out: ArrayVec<Chord, 2>,
    /// Shift extra segue junto (seleção por ←/→/Tab etc.).
    keep_shift: bool,
    /// Os modificadores da saída ficam segurados até soltar o Cmd (Cmd+Tab → Alt+Tab).
    hold: bool,
}

fn rule(id: &str, from_mods: u8, key: u16, out: &[Chord]) -> Rule {
    Rule {
        id: id.to_string(),
        from_mods,
        key,
        out: out.iter().copied().collect(),
        keep_shift: false,
        hold: false,
    }
}

fn ch(mods: u8, key: u16) -> Chord {
    Chord { mods, key }
}

fn builtin_rules() -> Vec<Rule> {
    let mut v = Vec::new();
    for (id, key) in [
        ("cmd_c", 0x2E),
        ("cmd_v", 0x2F),
        ("cmd_x", 0x2D),
        ("cmd_z", 0x2C),
        ("cmd_a", 0x1E),
        ("cmd_s", 0x1F),
        ("cmd_f", 0x21),
        ("cmd_t", 0x14),
        ("cmd_w", 0x11),
        ("cmd_n", 0x31),
        ("cmd_p", 0x19),
        ("cmd_r", 0x13),
    ] {
        v.push(rule(id, CMD, key, &[ch(CTRL, key)]));
    }
    v.push(rule("cmd_shift_z", CMD | SHIFT, 0x2C, &[ch(CTRL, 0x15)]));
    let mut tab = rule("cmd_tab", CMD, 0x0F, &[ch(ALT, 0x0F)]);
    tab.keep_shift = true;
    tab.hold = true;
    v.push(tab);
    v.push(rule("cmd_q", CMD, 0x10, &[ch(ALT, 0x3E)]));
    let mut nav = |id: &str, mods: u8, key: u16, out: Chord| {
        let mut r = rule(id, mods, key, &[out]);
        r.keep_shift = true;
        v.push(r);
    };
    nav("cmd_left", CMD, 0xE04B, ch(0, 0xE047));
    nav("cmd_right", CMD, 0xE04D, ch(0, 0xE04F));
    nav("cmd_up", CMD, 0xE048, ch(CTRL, 0xE047));
    nav("cmd_down", CMD, 0xE050, ch(CTRL, 0xE04F));
    nav("opt_left", ALT, 0xE04B, ch(CTRL, 0xE04B));
    nav("opt_right", ALT, 0xE04D, ch(CTRL, 0xE04D));
    v.push(rule(
        "cmd_backspace",
        CMD,
        0x0E,
        &[ch(SHIFT, 0xE047), ch(0, 0xE053)],
    ));
    v.push(rule("opt_backspace", ALT, 0x0E, &[ch(CTRL, 0x0E)]));
    v.push(rule(
        "cmd_shift_4",
        CMD | SHIFT,
        0x05,
        &[ch(CMD | SHIFT, 0x1F)],
    ));
    v.push(rule(
        "cmd_shift_5",
        CMD | SHIFT,
        0x06,
        &[ch(CMD | SHIFT, 0x1F)],
    ));
    v.push(rule("ctrl_cmd_q", CMD | CTRL, 0x10, &[ch(CMD, 0x26)]));
    v.push(rule(
        "cmd_opt_esc",
        CMD | ALT,
        0x01,
        &[ch(CTRL | SHIFT, 0x01)],
    ));
    // Toque de Win: a própria tecla Windows desce e sobe com a Espaço.
    v.push(rule("cmd_space", CMD, 0x39, &[ch(0, SC_LWIN)]));
    v
}

fn terminal_rules() -> Vec<Rule> {
    vec![
        rule("terminal_copy", CMD, 0x2E, &[ch(CTRL | SHIFT, 0x2E)]),
        rule("terminal_paste", CMD, 0x2F, &[ch(CTRL | SHIFT, 0x2F)]),
        rule("terminal_interrupt", CTRL, 0x2E, &[ch(CTRL, 0x2E)]),
    ]
}

/// Aplica os overrides do config: `enabled=false` remove, `to` substitui, id novo com
/// `from`+`to` adiciona (na frente, para ganhar das embutidas).
fn overlay(rules: &mut Vec<Rule>, cfg: &[KeymapRule]) {
    for c in cfg {
        let pos = rules.iter().position(|r| r.id == c.id);
        if !c.enabled {
            if let Some(p) = pos {
                rules.remove(p);
            }
            continue;
        }
        let from = c.from.as_ref().and_then(combo_to_scan);
        let to = c.to.as_ref().and_then(combo_to_scan);
        match (pos, from, to) {
            (Some(p), from, to) => {
                let r = &mut rules[p];
                if let Some((m, k)) = from {
                    r.from_mods = m;
                    r.key = k;
                }
                if let Some((m, k)) = to {
                    r.out = [ch(m, k)].into_iter().collect();
                    r.keep_shift = false;
                    r.hold = false;
                }
            }
            (None, Some((fm, fk)), Some((tm, tk))) => {
                rules.insert(0, rule(&c.id, fm, fk, &[ch(tm, tk)]));
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
struct AppRules {
    exe: String,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, Copy)]
struct Active {
    scan_in: u16,
    out_key: u16,
    hold: bool,
}

#[derive(Debug)]
pub struct KeymapTranslator {
    tap_timeout: Duration,
    rules: Vec<Rule>,
    apps: Vec<AppRules>,
    /// Modificadores físicos (ctrl, alt, shift) com o scan que o Mac mandou.
    phys: [Option<u16>; 3],
    /// O que o destino vê pressionado (ctrl, alt, shift, win).
    dest: [Option<u16>; 4],
    cmd_slots: u8,
    cmd_since: Option<Instant>,
    cmd_used: bool,
    active: ArrayVec<Active, MAX_TRANSLATED>,
}

type Out = ArrayVec<KeyEvent, MAX_TRANSLATED>;

fn push(out: &mut Out, scan: u16, down: bool) {
    let _ = out.try_push(KeyEvent {
        scan,
        down,
        autorepeat: false,
    });
}

impl KeymapTranslator {
    pub fn new(config: KeymapConfig) -> Self {
        let mut rules = builtin_rules();
        overlay(&mut rules, &config.rules);
        let mut apps = vec![AppRules {
            exe: "windowsterminal.exe".to_string(),
            rules: terminal_rules(),
        }];
        for ex in &config.app_exceptions {
            let i = match apps
                .iter()
                .position(|a| a.exe.eq_ignore_ascii_case(&ex.exe))
            {
                Some(i) => i,
                None => {
                    apps.push(AppRules {
                        exe: ex.exe.clone(),
                        rules: Vec::new(),
                    });
                    apps.len() - 1
                }
            };
            overlay(&mut apps[i].rules, &ex.rules);
        }
        Self {
            tap_timeout: Duration::from_millis(u64::from(config.cmd_tap_timeout_ms)),
            rules,
            apps,
            phys: [None; 3],
            dest: [None; 4],
            cmd_slots: 0,
            cmd_since: None,
            cmd_used: false,
            active: ArrayVec::new(),
        }
    }

    /// Traduz `event` para a sequência de eventos a injetar no destino. `foreground_exe` é o
    /// app em primeiro plano no Windows. Sem alocação.
    pub fn translate(
        &mut self,
        event: KeyEvent,
        foreground_exe: Option<&str>,
        out: &mut ArrayVec<KeyEvent, MAX_TRANSLATED>,
    ) {
        self.translate_at(event, foreground_exe, Instant::now(), out);
    }

    /// Como `translate`, com o instante do evento explícito (o Cmd tocado mede tempo).
    pub fn translate_at(
        &mut self,
        event: KeyEvent,
        foreground_exe: Option<&str>,
        now: Instant,
        out: &mut ArrayVec<KeyEvent, MAX_TRANSLATED>,
    ) {
        // D12: o autorepeat é gerado pelo receptor.
        if event.autorepeat {
            return;
        }
        match modifier_slot(event.scan).map(|s| s / 2) {
            Some(3) => self.cmd_event(event, now, out),
            Some(c) => self.modifier_event(usize::from(c), event, out),
            None => self.key_event(event, foreground_exe, out),
        }
    }

    fn cmd_event(&mut self, event: KeyEvent, now: Instant, out: &mut Out) {
        let bit: u8 = if event.scan == SC_LWIN {
            1 << 6
        } else {
            1 << 7
        };
        if event.down {
            if self.cmd_slots == 0 {
                self.cmd_since = Some(now);
                self.cmd_used = false;
            }
            self.cmd_slots |= bit;
            return;
        }
        self.cmd_slots &= !bit;
        if self.cmd_slots != 0 {
            return;
        }
        self.restore(out);
        let tap = !self.cmd_used
            && self
                .cmd_since
                .is_some_and(|t| now.saturating_duration_since(t) < self.tap_timeout);
        self.cmd_since = None;
        if tap {
            push(out, event.scan, true);
            push(out, event.scan, false);
        }
    }

    fn modifier_event(&mut self, cat: usize, event: KeyEvent, out: &mut Out) {
        if self.cmd_slots != 0 {
            self.cmd_used = true;
        }
        if event.down {
            self.phys[cat] = Some(event.scan);
            if self.dest[cat].is_none() {
                self.dest[cat] = Some(event.scan);
                push(out, event.scan, true);
            }
        } else {
            if self.phys[cat] == Some(event.scan) {
                self.phys[cat] = None;
            }
            if self.dest[cat] == Some(event.scan) {
                self.dest[cat] = None;
                push(out, event.scan, false);
            }
        }
    }

    fn key_event(&mut self, event: KeyEvent, exe: Option<&str>, out: &mut Out) {
        let scan = event.scan;
        if !event.down {
            if let Some(i) = self.active.iter().position(|a| a.scan_in == scan) {
                let a = self.active.remove(i);
                push(out, a.out_key, false);
                if !a.hold || self.cmd_slots == 0 {
                    self.restore(out);
                }
            } else {
                push(out, scan, false);
            }
            return;
        }
        if self.cmd_slots != 0 {
            self.cmd_used = true;
        }
        let mut mods = 0;
        for (i, p) in self.phys.iter().enumerate() {
            if p.is_some() {
                mods |= 1 << i;
            }
        }
        if self.cmd_slots != 0 {
            mods |= CMD;
        }
        let (chords, hold) = self.lookup(mods, scan, exe);
        if let Some(i) = self.active.iter().position(|a| a.scan_in == scan) {
            self.active.remove(i);
        }
        let last = chords.len().saturating_sub(1);
        for (i, c) in chords.iter().enumerate() {
            self.set_mods(c.mods, out);
            push(out, c.key, true);
            if i != last {
                push(out, c.key, false);
            }
        }
        if let Some(c) = chords.last() {
            if self.active.is_full() {
                self.active.remove(0);
            }
            self.active.push(Active {
                scan_in: scan,
                out_key: c.key,
                hold,
            });
        }
    }

    fn lookup(&self, mods: u8, scan: u16, exe: Option<&str>) -> (ArrayVec<Chord, 2>, bool) {
        let app = exe.and_then(|e| self.apps.iter().find(|a| a.exe.eq_ignore_ascii_case(e)));
        let sets = app
            .map(|a| a.rules.as_slice())
            .into_iter()
            .chain([self.rules.as_slice()]);
        for rules in sets {
            for r in rules {
                if r.key != scan {
                    continue;
                }
                if r.from_mods == mods {
                    return (r.out.clone(), r.hold);
                }
                if r.keep_shift && mods & SHIFT != 0 && r.from_mods == mods & !SHIFT {
                    let mut o = r.out.clone();
                    for c in &mut o {
                        c.mods |= SHIFT;
                    }
                    return (o, r.hold);
                }
            }
        }
        // Regra geral: o Cmd vira Ctrl; Control, Option e Shift passam como estão (D3).
        let m = if mods & CMD != 0 {
            (mods & !CMD) | CTRL
        } else {
            mods
        };
        ([ch(m, scan)].into_iter().collect(), false)
    }

    /// Ajusta os modificadores do destino para `mods`, soltando antes de apertar.
    fn set_mods(&mut self, mods: u8, out: &mut Out) {
        let target: [Option<u16>; 4] = std::array::from_fn(|i| {
            (mods & (1 << i) != 0).then(|| {
                if i < 3 {
                    self.phys[i].unwrap_or(LEFT_SCANS[i])
                } else {
                    SC_LWIN
                }
            })
        });
        self.apply_target(target, out);
    }

    /// Volta o destino ao que os modificadores físicos pedem.
    fn restore(&mut self, out: &mut Out) {
        self.apply_target([self.phys[0], self.phys[1], self.phys[2], None], out);
    }

    fn apply_target(&mut self, target: [Option<u16>; 4], out: &mut Out) {
        for (cur, want) in self.dest.iter_mut().zip(target) {
            if let (Some(s), true) = (*cur, *cur != want) {
                push(out, s, false);
                *cur = None;
            }
        }
        for (cur, want) in self.dest.iter_mut().zip(target) {
            if let (Some(s), true) = (want, *cur != want) {
                push(out, s, true);
                *cur = Some(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AppException;

    const CMD_L: u16 = 0xE05B;
    const CMD_R: u16 = 0xE05C;
    const CTRL_L: u16 = 0x1D;
    const ALT_L: u16 = 0x38;
    const SHIFT_L: u16 = 0x2A;
    const WIN: u16 = 0xE05B;
    const TAB: u16 = 0x0F;
    const ESC: u16 = 0x01;
    const BKSP: u16 = 0x0E;
    const SPACE: u16 = 0x39;
    const DEL: u16 = 0xE053;
    const HOME: u16 = 0xE047;
    const END: u16 = 0xE04F;
    const LEFT: u16 = 0xE04B;
    const RIGHT: u16 = 0xE04D;
    const UP: u16 = 0xE048;
    const DOWN: u16 = 0xE050;
    const K_C: u16 = 0x2E;
    const K_V: u16 = 0x2F;
    const K_X: u16 = 0x2D;
    const K_Z: u16 = 0x2C;
    const K_Y: u16 = 0x15;
    const K_Q: u16 = 0x10;
    const K_S: u16 = 0x1F;
    const K_L: u16 = 0x26;
    const K_K: u16 = 0x25;
    const F4: u16 = 0x3E;

    fn u(s: u16) -> (u16, bool) {
        (s, false)
    }
    fn d(s: u16) -> (u16, bool) {
        (s, true)
    }

    struct H {
        t: KeymapTranslator,
        exe: Option<&'static str>,
        now: Instant,
    }

    impl H {
        fn new() -> Self {
            Self::with(KeymapConfig::default())
        }
        fn with(c: KeymapConfig) -> Self {
            Self {
                t: KeymapTranslator::new(c),
                exe: None,
                now: Instant::now(),
            }
        }
        fn ev(&mut self, (scan, down): (u16, bool)) -> Vec<(u16, bool)> {
            let mut out = ArrayVec::new();
            let e = KeyEvent {
                scan,
                down,
                autorepeat: false,
            };
            self.t.translate_at(e, self.exe, self.now, &mut out);
            out.iter().map(|e| (e.scan, e.down)).collect()
        }
        /// Todos os eventos de saída, concatenados.
        fn run(&mut self, seq: &[(u16, bool)]) -> Vec<(u16, bool)> {
            seq.iter().flat_map(|&e| self.ev(e)).collect()
        }
    }

    fn cmd_chord(key: u16) -> [(u16, bool); 4] {
        [d(CMD_L), d(key), u(key), u(CMD_L)]
    }

    #[test]
    fn cmd_letra_vira_ctrl_letra() {
        for key in [
            K_C, K_V, K_X, K_Z, 0x1E, K_S, 0x21, 0x14, 0x11, 0x31, 0x19, 0x13,
        ] {
            let got = H::new().run(&cmd_chord(key));
            assert_eq!(got, [d(CTRL_L), d(key), u(key), u(CTRL_L)], "key {key:#x}");
        }
    }

    #[test]
    fn shift_cmd_z_vira_ctrl_y() {
        let got = H::new().run(&[d(SHIFT_L), d(CMD_L), d(K_Z), u(K_Z), u(CMD_L), u(SHIFT_L)]);
        assert_eq!(
            got,
            [
                d(SHIFT_L),
                u(SHIFT_L),
                d(CTRL_L),
                d(K_Y),
                u(K_Y),
                u(CTRL_L),
                d(SHIFT_L),
                u(SHIFT_L)
            ]
        );
    }

    #[test]
    fn cmd_tab_segura_alt_ate_soltar_cmd() {
        let mut h = H::new();
        assert_eq!(h.ev(d(CMD_L)), []);
        assert_eq!(h.ev(d(TAB)), [d(ALT_L), d(TAB)]);
        assert_eq!(h.ev(u(TAB)), [u(TAB)]);
        assert_eq!(h.ev(d(TAB)), [d(TAB)]);
        assert_eq!(h.ev(u(TAB)), [u(TAB)]);
        assert_eq!(h.ev(u(CMD_L)), [u(ALT_L)]);
    }

    #[test]
    fn cmd_q_vira_alt_f4() {
        let got = H::new().run(&cmd_chord(K_Q));
        assert_eq!(got, [d(ALT_L), d(F4), u(F4), u(ALT_L)]);
    }

    #[test]
    fn cmd_setas() {
        let cases = [
            (LEFT, vec![d(HOME), u(HOME)]),
            (RIGHT, vec![d(END), u(END)]),
            (UP, vec![d(CTRL_L), d(HOME), u(HOME), u(CTRL_L)]),
            (DOWN, vec![d(CTRL_L), d(END), u(END), u(CTRL_L)]),
        ];
        for (key, want) in cases {
            assert_eq!(H::new().run(&cmd_chord(key)), want, "key {key:#x}");
        }
    }

    #[test]
    fn option_setas_vira_ctrl_setas() {
        for key in [LEFT, RIGHT] {
            let got = H::new().run(&[d(ALT_L), d(key), u(key), u(ALT_L)]);
            assert_eq!(
                got,
                [
                    d(ALT_L),
                    u(ALT_L),
                    d(CTRL_L),
                    d(key),
                    u(key),
                    u(CTRL_L),
                    d(ALT_L),
                    u(ALT_L)
                ]
            );
        }
    }

    #[test]
    fn cmd_backspace_vira_shift_home_delete() {
        let got = H::new().run(&cmd_chord(BKSP));
        assert_eq!(
            got,
            [d(SHIFT_L), d(HOME), u(HOME), u(SHIFT_L), d(DEL), u(DEL)]
        );
    }

    #[test]
    fn option_backspace_vira_ctrl_backspace() {
        let got = H::new().run(&[d(ALT_L), d(BKSP), u(BKSP), u(ALT_L)]);
        assert_eq!(
            got,
            [
                d(ALT_L),
                u(ALT_L),
                d(CTRL_L),
                d(BKSP),
                u(BKSP),
                u(CTRL_L),
                d(ALT_L),
                u(ALT_L)
            ]
        );
    }

    #[test]
    fn shift_cmd_4_e_5_viram_win_shift_s() {
        for key in [0x05, 0x06] {
            let got = H::new().run(&[d(SHIFT_L), d(CMD_L), d(key), u(key), u(CMD_L), u(SHIFT_L)]);
            assert_eq!(
                got,
                [d(SHIFT_L), d(WIN), d(K_S), u(K_S), u(WIN), u(SHIFT_L)],
                "key {key:#x}"
            );
        }
    }

    #[test]
    fn ctrl_cmd_q_vira_win_l() {
        let got = H::new().run(&[d(CTRL_L), d(CMD_L), d(K_Q), u(K_Q), u(CMD_L), u(CTRL_L)]);
        assert_eq!(
            got,
            [
                d(CTRL_L),
                u(CTRL_L),
                d(WIN),
                d(K_L),
                u(K_L),
                u(WIN),
                d(CTRL_L),
                u(CTRL_L)
            ]
        );
    }

    #[test]
    fn cmd_option_esc_vira_ctrl_shift_esc() {
        let got = H::new().run(&[d(ALT_L), d(CMD_L), d(ESC), u(ESC), u(CMD_L), u(ALT_L)]);
        assert_eq!(
            got,
            [
                d(ALT_L),
                u(ALT_L),
                d(CTRL_L),
                d(SHIFT_L),
                d(ESC),
                u(ESC),
                u(CTRL_L),
                u(SHIFT_L),
                d(ALT_L),
                u(ALT_L)
            ]
        );
    }

    #[test]
    fn cmd_espaco_vira_toque_de_win() {
        let got = H::new().run(&cmd_chord(SPACE));
        assert_eq!(got, [d(WIN), u(WIN)]);
    }

    #[test]
    fn cmd_sozinho_rapido_envia_win() {
        let mut h = H::new();
        h.ev(d(CMD_L));
        h.now += Duration::from_millis(399);
        assert_eq!(h.ev(u(CMD_L)), [d(WIN), u(WIN)]);
    }

    #[test]
    fn cmd_sozinho_lento_nao_envia_nada() {
        let mut h = H::new();
        h.ev(d(CMD_R));
        h.now += Duration::from_millis(400);
        assert_eq!(h.ev(u(CMD_R)), []);
    }

    #[test]
    fn cmd_com_outra_tecla_nunca_envia_win() {
        let mut h = H::new();
        h.run(&[d(CMD_L), d(K_C), u(K_C)]);
        assert_eq!(h.ev(u(CMD_L)), []);
        // Modificador no meio também conta como "outra tecla".
        let mut h = H::new();
        h.run(&[d(CMD_L), d(SHIFT_L), u(SHIFT_L)]);
        assert_eq!(h.ev(u(CMD_L)), []);
    }

    #[test]
    fn timeout_vem_do_config() {
        let mut h = H::with(KeymapConfig {
            cmd_tap_timeout_ms: 50,
            ..KeymapConfig::default()
        });
        h.ev(d(CMD_L));
        h.now += Duration::from_millis(60);
        assert_eq!(h.ev(u(CMD_L)), []);
    }

    #[test]
    fn regra_geral_cmd_k_option_k_control_tab() {
        assert_eq!(
            H::new().run(&cmd_chord(K_K)),
            [d(CTRL_L), d(K_K), u(K_K), u(CTRL_L)]
        );
        assert_eq!(
            H::new().run(&[d(ALT_L), d(K_K), u(K_K), u(ALT_L)]),
            [d(ALT_L), d(K_K), u(K_K), u(ALT_L)]
        );
        assert_eq!(
            H::new().run(&[d(CTRL_L), d(TAB), u(TAB), u(CTRL_L)]),
            [d(CTRL_L), d(TAB), u(TAB), u(CTRL_L)]
        );
    }

    #[test]
    fn excecao_do_terminal_case_insensitive() {
        let mut h = H::new();
        h.exe = Some("windowsterminal.EXE");
        assert_eq!(
            h.run(&cmd_chord(K_C)),
            [d(CTRL_L), d(SHIFT_L), d(K_C), u(K_C), u(CTRL_L), u(SHIFT_L)]
        );
        assert_eq!(
            h.run(&[d(CTRL_L), d(K_C), u(K_C), u(CTRL_L)]),
            [d(CTRL_L), d(K_C), u(K_C), u(CTRL_L)]
        );
        let mut h = H::new();
        h.exe = Some("notepad.exe");
        assert_eq!(
            h.run(&cmd_chord(K_C)),
            [d(CTRL_L), d(K_C), u(K_C), u(CTRL_L)]
        );
        let _ = K_V;
    }

    fn combo(mods: &[Modifier], key: &str) -> KeyCombo {
        KeyCombo {
            modifiers: mods.to_vec(),
            key: key.to_string(),
        }
    }

    fn cfg_rule(
        id: &str,
        enabled: bool,
        from: Option<KeyCombo>,
        to: Option<KeyCombo>,
    ) -> KeymapRule {
        KeymapRule {
            id: id.to_string(),
            enabled,
            from,
            to,
        }
    }

    #[test]
    fn overrides_do_config_vencem_a_tabela_embutida() {
        // Desativa: Cmd+Q passa pela regra geral (Ctrl+Q).
        let mut h = H::with(KeymapConfig {
            rules: vec![cfg_rule("cmd_q", false, None, None)],
            ..KeymapConfig::default()
        });
        assert_eq!(
            h.run(&cmd_chord(K_Q)),
            [d(CTRL_L), d(K_Q), u(K_Q), u(CTRL_L)]
        );
        // Substitui: Cmd+Q vira Ctrl+W.
        let mut h = H::with(KeymapConfig {
            rules: vec![cfg_rule(
                "cmd_q",
                true,
                None,
                Some(combo(&[Modifier::Ctrl], "KeyW")),
            )],
            ..KeymapConfig::default()
        });
        assert_eq!(
            h.run(&cmd_chord(K_Q)),
            [d(CTRL_L), d(0x11), u(0x11), u(CTRL_L)]
        );
        // Adiciona: Cmd+J vira Alt+F4 (a regra geral daria Ctrl+J).
        let mut h = H::with(KeymapConfig {
            rules: vec![cfg_rule(
                "meu",
                true,
                Some(combo(&[Modifier::Cmd], "KeyJ")),
                Some(combo(&[Modifier::Option], "F4")),
            )],
            ..KeymapConfig::default()
        });
        assert_eq!(h.run(&cmd_chord(0x24)), [d(ALT_L), d(F4), u(F4), u(ALT_L)]);
    }

    #[test]
    fn overrides_de_excecao_por_app_vencem_a_embutida() {
        let mut h = H::with(KeymapConfig {
            app_exceptions: vec![AppException {
                exe: "WindowsTerminal.exe".into(),
                rules: vec![cfg_rule("terminal_copy", false, None, None)],
            }],
            ..KeymapConfig::default()
        });
        h.exe = Some("WindowsTerminal.exe");
        assert_eq!(
            h.run(&cmd_chord(K_C)),
            [d(CTRL_L), d(K_C), u(K_C), u(CTRL_L)]
        );
        let mut h = H::with(KeymapConfig {
            app_exceptions: vec![AppException {
                exe: "code.exe".into(),
                rules: vec![cfg_rule(
                    "x",
                    true,
                    Some(combo(&[Modifier::Cmd], "KeyK")),
                    Some(combo(&[Modifier::Ctrl, Modifier::Shift], "KeyK")),
                )],
            }],
            ..KeymapConfig::default()
        });
        h.exe = Some("Code.exe");
        assert_eq!(
            h.run(&cmd_chord(K_K)),
            [d(CTRL_L), d(SHIFT_L), d(K_K), u(K_K), u(CTRL_L), u(SHIFT_L)]
        );
    }

    #[test]
    fn autorepeat_e_descartado() {
        let mut h = H::new();
        h.ev(d(CMD_L));
        let mut out = ArrayVec::new();
        for scan in [K_C, SHIFT_L, CMD_L] {
            let e = KeyEvent {
                scan,
                down: true,
                autorepeat: true,
            };
            h.t.translate_at(e, None, h.now, &mut out);
        }
        assert!(out.is_empty());
    }

    #[test]
    fn release_all_depois_da_traducao_com_cmd_tab_segurado() {
        use crate::keymap::pressed::PressedTracker;
        let mut h = H::new();
        let mut tracker = PressedTracker::default();
        for e in [d(CMD_L), d(TAB)] {
            let mut out = ArrayVec::new();
            let ev = KeyEvent {
                scan: e.0,
                down: e.1,
                autorepeat: false,
            };
            h.t.translate_at(ev, None, h.now, &mut out);
            out.into_iter().for_each(|o| tracker.track(o));
        }
        let ups: Vec<_> = tracker
            .release_all()
            .iter()
            .map(|e| (e.scan, e.down))
            .collect();
        assert_eq!(ups, [u(TAB), u(ALT_L)]);
    }

    #[test]
    fn codigos_do_teclado_conhecidos() {
        assert_eq!(code_to_scan("Digit1"), Some(0x02));
        assert_eq!(code_to_scan("KeyL"), Some(0x26));
        assert_eq!(code_to_scan("Nada"), None);
    }

    /// O tradutor não aloca em `translate` (alocador contador de `test_alloc`, por thread).
    #[test]
    fn dez_mil_eventos_sem_alocar() {
        let mut t = KeymapTranslator::new(KeymapConfig::default());
        let mut out = ArrayVec::new();
        // Cmd+C, Cmd+Tab, Shift+Cmd+Z, Cmd+Backspace e uma tecla solta, em laço.
        let seq: [(u16, bool); 20] = [
            (CMD_L, true),
            (0x2E, true),
            (0x2E, false),
            (CMD_L, false),
            (CMD_L, true),
            (TAB, true),
            (TAB, false),
            (CMD_L, false),
            (SHIFT_L, true),
            (CMD_L, true),
            (0x2C, true),
            (0x2C, false),
            (CMD_L, false),
            (SHIFT_L, false),
            (CMD_L, true),
            (BKSP, true),
            (BKSP, false),
            (CMD_L, false),
            (0x1E, true),
            (0x1E, false),
        ];
        let ((), allocs, _) = crate::test_alloc::measure(|| {
            let mut n = 0;
            while n < 10_000 {
                for &(scan, down) in &seq {
                    out.clear();
                    let ev = KeyEvent {
                        scan,
                        down,
                        autorepeat: false,
                    };
                    t.translate(ev, Some("WindowsTerminal.exe"), &mut out);
                    n += 1;
                }
            }
        });
        assert_eq!(allocs, 0);
    }
}
