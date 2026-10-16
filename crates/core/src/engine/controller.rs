//! dono: C10
//! Controller: do evento capturado até a saída (canal rápido, fila de controle, cursor local).

use std::time::{Duration, Instant};

use arrayvec::ArrayVec;
use rtrb::Producer;

use super::reverse::map_scan;
use super::{EngineEvent, EventHandler, LayoutState, sat_i16, scan_of};
use crate::input_shaping::{PinchTracker, RotationMapper, ScrollAccumulator};
use crate::keymap::hotkeys::{HotkeyMatcher, HotkeyOutcome};
use crate::keymap::pressed::{MAX_PRESSED, PressedTracker};
use crate::keymap::translator::{
    KeyEvent, KeymapTranslator, MAX_TRANSLATED, combo_to_scan, modifier_slot, mods_from_slots,
};
use crate::platform::{CursorControl, InputCapture, InputInjector};
use crate::transition::{LocalDecision, TransitionEngine};
use crate::types::{
    CapturedEvent, ClipboardConfig, ClipboardMode, Config, DisplayInfo, HotkeyAction,
    HotkeyBinding, InputEvent, KeyCombo, KeymapConfig, LayoutProfile, Machine, Message, Modifier,
    ReverseModifiers, ScrollPhase, TransitionConfig,
};

const SC_LCTRL: u16 = 0x1D;
const SC_LALT: u16 = 0x38;
const SC_LSHIFT: u16 = 0x2A;
const SC_LWIN: u16 = 0xE05B;

/// Sem `Magnify` por este tempo, o gesto de pinça terminou (o evento não traz fase).
const PINCH_IDLE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Local,
    Remote,
}

/// Lado que captura e envia: o Mac, ou o agente Windows no sentido inverso (`set_reverse`, D24).
///
/// `handle`, `on_message` e `tick` devem rodar na mesma thread (a da captura). O `handle` não
/// aloca fora das trocas de foco.
pub struct Controller<Cap, Cur, Inj> {
    capture: Cap,
    cursor: Cur,
    /// Injetor do Mac: solta as teclas seguradas quando o foco sai daqui.
    injector: Inj,
    fast: Producer<InputEvent>,
    control: Producer<Message>,
    layout: LayoutState,
    transition_cfg: TransitionConfig,
    transition: TransitionEngine,
    keymap_cfg: KeymapConfig,
    translator: KeymapTranslator,
    matcher: HotkeyMatcher,
    /// Mesmos atalhos sem Cmd, alimentado sem os eventos de Cmd: Ctrl+Option+1 funciona com Cmd
    /// segurado (o `HotkeyMatcher` exige modificadores exatos).
    matcher_no_cmd: HotkeyMatcher,
    tracker: PressedTracker,
    scroll_x: ScrollAccumulator,
    scroll_y: ScrollAccumulator,
    pinch: PinchTracker,
    pinch_last: Option<Instant>,
    rotation: RotationMapper,
    focus: Focus,
    paused: bool,
    locked: bool,
    /// Tela do Mac sob o cursor (índice em `layout.local`) e última posição conhecida.
    cur_display: usize,
    last_pos: (i32, i32),
    /// Slots de modificador físicos segurados (para `require_modifier` e o Cmd do matcher).
    mod_slots: u8,
    /// Teclas físicas seguradas no Mac enquanto o foco é local.
    local_held: ArrayVec<u16, MAX_PRESSED>,
    foreground: Option<String>,
    clipboard: ClipboardConfig,
    on_event: Option<EventHandler>,
    seq: u64,
    started: Instant,
    dropped_fast: u64,
    dropped_control: u64,
    /// Máquina que captura. No sentido inverso (Windows) as teclas não passam pelo tradutor Mac→
    /// Windows: só trocam de modificador (D25).
    local_machine: Machine,
    reverse: Option<ReverseModifiers>,
}

fn matchers(hotkeys: &[HotkeyBinding]) -> (HotkeyMatcher, HotkeyMatcher) {
    let no_cmd = hotkeys
        .iter()
        .filter(|b| !b.combo.modifiers.contains(&Modifier::Cmd))
        .cloned()
        .collect();
    (
        HotkeyMatcher::new(hotkeys.to_vec()),
        HotkeyMatcher::new(no_cmd),
    )
}

impl<Cap: InputCapture, Cur: CursorControl, Inj: InputInjector> Controller<Cap, Cur, Inj> {
    /// `fast`: producer de `Session::take_input_producer`. `control`: fila que o chamador esvazia
    /// para `Session::send`. `capture` pode já estar iniciada.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &Config,
        profile: LayoutProfile,
        local_displays: Vec<DisplayInfo>,
        capture: Cap,
        cursor: Cur,
        injector: Inj,
        fast: Producer<InputEvent>,
        control: Producer<Message>,
    ) -> Self {
        let layout = LayoutState::new(profile, local_displays);
        let (matcher, matcher_no_cmd) = matchers(&config.hotkeys);
        let transition = TransitionEngine::new(
            config.transition.clone(),
            layout.profile.clone(),
            layout.combined(),
        );
        Self {
            capture,
            cursor,
            injector,
            fast,
            control,
            transition,
            transition_cfg: config.transition.clone(),
            layout,
            keymap_cfg: config.keymap.clone(),
            translator: KeymapTranslator::new(config.keymap.clone()),
            matcher,
            matcher_no_cmd,
            tracker: PressedTracker::default(),
            scroll_x: ScrollAccumulator::default(),
            scroll_y: ScrollAccumulator::default(),
            pinch: PinchTracker::default(),
            pinch_last: None,
            rotation: RotationMapper::default(),
            focus: Focus::Local,
            paused: false,
            locked: false,
            cur_display: 0,
            last_pos: (0, 0),
            mod_slots: 0,
            local_held: ArrayVec::new(),
            foreground: None,
            clipboard: config.clipboard.clone(),
            on_event: None,
            seq: 0,
            started: Instant::now(),
            dropped_fast: 0,
            dropped_control: 0,
            local_machine: Machine::Mac,
            reverse: None,
        }
    }

    /// Sentido inverso (D24, D25): este Controller roda no Windows e controla o Mac. As telas
    /// locais são as do Windows; Ctrl/Win/Alt viram os modificadores do Mac configurados.
    pub fn set_reverse(&mut self, modifiers: ReverseModifiers) {
        self.local_machine = Machine::Windows;
        self.reverse = Some(modifiers);
        self.cur_display = 0;
        self.rebuild_transition();
    }

    pub fn set_event_handler(&mut self, handler: EventHandler) {
        self.on_event = Some(handler);
    }

    /// Atalho de rotação (começa sem mapeamento, D26).
    pub fn set_rotation_combo(&mut self, combo: Option<KeyCombo>) {
        self.rotation.combo = combo;
    }

    /// Aplica mudanças de config (atalhos, keymap, transição, clipboard). Só no foco local ou
    /// pausado reinicia o tradutor sem perder estado de teclas do destino.
    pub fn apply_config(&mut self, config: &Config) {
        let (m, n) = matchers(&config.hotkeys);
        self.matcher = m;
        self.matcher_no_cmd = n;
        self.transition_cfg = config.transition.clone();
        self.keymap_cfg = config.keymap.clone();
        if self.reverse.is_some() {
            self.reverse = Some(config.keymap.reverse_modifiers.clone());
        }
        if self.focus == Focus::Local {
            self.translator = KeymapTranslator::new(config.keymap.clone());
        }
        self.clipboard = config.clipboard.clone();
        self.rebuild_transition();
    }

    pub fn set_profile(&mut self, profile: LayoutProfile) {
        self.layout.profile = profile;
        self.rebuild_transition();
    }

    pub fn set_local_displays(&mut self, displays: Vec<DisplayInfo>) {
        self.layout.local = displays;
        self.cur_display = 0;
        self.rebuild_transition();
    }

    fn rebuild_transition(&mut self) {
        self.transition = TransitionEngine::new(
            self.transition_cfg.clone(),
            self.layout.profile.clone(),
            self.layout.combined(),
        );
        self.transition.set_locked(self.locked);
        self.transition.set_local_machine(self.local_machine);
    }

    pub fn is_remote(&self) -> bool {
        self.focus == Focus::Remote
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Eventos descartados por fila cheia: (canal rápido, controle).
    pub fn dropped(&self) -> (u64, u64) {
        (self.dropped_fast, self.dropped_control)
    }

    pub fn capture(&self) -> &Cap {
        &self.capture
    }

    // ---- saída ----

    fn emit(&mut self, event: EngineEvent) {
        if let Some(h) = self.on_event.as_mut() {
            h(event);
        }
    }

    fn send_control(&mut self, msg: Message) {
        if self.control.push(msg).is_err() {
            self.dropped_control += 1;
        }
    }

    fn send_fast(&mut self, ev: InputEvent) {
        if self.fast.push(ev).is_err() {
            self.dropped_fast += 1;
        }
    }

    /// Key/Button/Media vão pela fila de controle, em ordem e sem perda silenciosa.
    fn send_input_reliable(&mut self, event: InputEvent, now: Instant) {
        self.seq = self.seq.wrapping_add(1);
        let ts_us = now.saturating_duration_since(self.started).as_micros() as u64;
        self.send_control(Message::Input {
            seq: self.seq,
            ts_us,
            event,
        });
    }

    /// Tecla já em scan do Windows: registra no que o destino vê e envia.
    fn send_key(&mut self, ev: KeyEvent, now: Instant) {
        self.tracker.track(ev);
        self.send_input_reliable(
            InputEvent::Key {
                scan: ev.scan,
                down: ev.down,
            },
            now,
        );
    }

    // ---- entrada ----

    /// Processa um evento da captura. Roda na thread de captura.
    pub fn handle(&mut self, event: CapturedEvent) {
        self.handle_at(event, Instant::now());
    }

    pub fn handle_at(&mut self, event: CapturedEvent, now: Instant) {
        self.end_pinch_if_idle(now);
        match event {
            CapturedEvent::Key {
                code,
                down,
                autorepeat,
                ..
            } => self.on_key(code, down, autorepeat, now),
            CapturedEvent::MouseMove { dx, dy, pos } => self.on_move(dx, dy, pos, now),
            CapturedEvent::Button { button, down } => {
                if self.focus == Focus::Remote {
                    self.send_input_reliable(InputEvent::Button { button, down }, now);
                }
            }
            CapturedEvent::Media { key, down } => {
                if self.focus == Focus::Remote {
                    self.send_input_reliable(InputEvent::Media { key, down }, now);
                }
            }
            CapturedEvent::Scroll {
                dx_px,
                dy_px,
                phase,
                continuous,
            } => {
                if self.focus == Focus::Remote {
                    self.on_scroll(dx_px, dy_px, phase, continuous);
                }
            }
            CapturedEvent::Magnify { delta } => {
                if self.focus == Focus::Remote {
                    self.on_magnify(delta, now);
                }
            }
            CapturedEvent::Rotate { delta } => {
                if self.focus == Focus::Remote {
                    self.on_rotate(delta, now);
                }
            }
        }
    }

    /// Relógio do laço da captura: fecha pinça parada. Chame também quando não há eventos.
    pub fn tick(&mut self, now: Instant) {
        self.end_pinch_if_idle(now);
    }

    fn on_move(&mut self, dx: i32, dy: i32, pos: (f64, f64), now: Instant) {
        match self.focus {
            Focus::Remote => {
                self.send_fast(InputEvent::MouseMove {
                    dx: sat_i16(dx),
                    dy: sat_i16(dy),
                });
            }
            Focus::Local => {
                self.last_pos = (pos.0 as i32, pos.1 as i32);
                if self.paused {
                    return;
                }
                let Some(idx) = self.display_under(self.last_pos) else {
                    return;
                };
                self.cur_display = idx;
                let mut mods: ArrayVec<Modifier, 4> = ArrayVec::new();
                let cats = mods_from_slots(self.mod_slots);
                for (bit, m) in [
                    (1, Modifier::Ctrl),
                    (2, Modifier::Option),
                    (4, Modifier::Shift),
                    (8, Modifier::Cmd),
                ] {
                    if cats & bit != 0 {
                        mods.push(m);
                    }
                }
                let decision = self.transition.on_local_move(
                    &self.layout.local[idx].id,
                    self.last_pos,
                    (dx, dy),
                    &mods,
                    now,
                );
                if let LocalDecision::Enter {
                    display_id,
                    entry_px,
                } = decision
                {
                    self.enter_remote(display_id, entry_px);
                }
            }
        }
    }

    /// Tela do Mac que contém `p`; mantém a anterior se o ponto cair na borda externa.
    fn display_under(&self, p: (i32, i32)) -> Option<usize> {
        let inside = |d: &DisplayInfo| {
            d.machine == self.local_machine
                && p.0 >= d.origin_px.0
                && p.0 < d.origin_px.0 + d.size_px.0 as i32
                && p.1 >= d.origin_px.1
                && p.1 < d.origin_px.1 + d.size_px.1 as i32
        };
        let cur = self.layout.local.get(self.cur_display)?;
        if inside(cur) {
            return Some(self.cur_display);
        }
        self.layout
            .local
            .iter()
            .position(inside)
            .or_else(|| (cur.machine == self.local_machine).then_some(self.cur_display))
    }

    fn on_key(&mut self, code: crate::types::KeyCode, down: bool, autorepeat: bool, now: Instant) {
        let scan = scan_of(code);
        if let Some(slot) = scan.and_then(modifier_slot) {
            if down {
                self.mod_slots |= 1 << slot;
            } else {
                self.mod_slots &= !(1 << slot);
            }
        }
        let is_cmd = scan.and_then(modifier_slot).is_some_and(|s| s / 2 == 3);
        // AltGr (ABNT2 etc.) chega como LCtrl+RAlt: no reverso não pode disparar atalhos do Relay.
        let altgr = self.reverse.is_some()
            && self.mod_slots & (1 << 3) != 0
            && scan.and_then(modifier_slot).is_none();
        let (main, alt) = if altgr {
            (HotkeyOutcome::Pass, HotkeyOutcome::Pass)
        } else {
            let main = self.matcher.feed(code, down);
            let alt = if is_cmd {
                HotkeyOutcome::Pass
            } else {
                self.matcher_no_cmd.feed(code, down)
            };
            (main, alt)
        };
        let outcome = if main == HotkeyOutcome::Pass {
            alt
        } else {
            main
        };
        match outcome {
            HotkeyOutcome::Consumed(Some(action)) => self.on_action(action, now),
            HotkeyOutcome::Consumed(None) => {}
            HotkeyOutcome::Pass => {
                let Some(scan) = scan else { return };
                match self.focus {
                    Focus::Local => {
                        self.track_local(scan, down);
                    }
                    Focus::Remote if self.reverse.is_some() => {
                        if self.track_local(scan, down) {
                            return;
                        }
                        let mapped = self.reverse.as_ref().map_or(scan, |m| map_scan(scan, m));
                        let ev = KeyEvent {
                            scan: mapped,
                            down,
                            autorepeat,
                        };
                        self.send_key(ev, now);
                    }
                    Focus::Remote => {
                        let mut out: ArrayVec<KeyEvent, MAX_TRANSLATED> = ArrayVec::new();
                        self.translator.translate_at(
                            KeyEvent {
                                scan,
                                down,
                                autorepeat,
                            },
                            self.foreground.as_deref(),
                            now,
                            &mut out,
                        );
                        for ev in out {
                            self.send_key(ev, now);
                        }
                    }
                }
            }
        }
    }

    /// Registra a tecla física; `true` quando é um down de tecla que já estava segurada (os hooks
    /// do Windows não distinguem a repetição do sistema, que o receptor gera sozinho, D12).
    fn track_local(&mut self, scan: u16, down: bool) -> bool {
        let pos = self.local_held.iter().position(|&s| s == scan);
        match (down, pos) {
            (true, None) => {
                let _ = self.local_held.try_push(scan);
            }
            (false, Some(i)) => {
                self.local_held.remove(i);
            }
            (true, Some(_)) => return true,
            _ => {}
        }
        false
    }

    fn on_scroll(&mut self, dx_px: f64, dy_px: f64, phase: ScrollPhase, continuous: bool) {
        if phase == ScrollPhase::Began {
            self.scroll_x.reset();
            self.scroll_y.reset();
        }
        let (dx, dy) = if continuous {
            (self.scroll_x.push(dx_px), self.scroll_y.push(dy_px))
        } else {
            (
                ScrollAccumulator::push_lines(dx_px),
                ScrollAccumulator::push_lines(dy_px),
            )
        };
        if matches!(phase, ScrollPhase::Ended) {
            self.scroll_x.reset();
            self.scroll_y.reset();
        }
        if dx != 0 || dy != 0 {
            self.send_fast(InputEvent::Scroll { dx, dy, phase });
        }
    }

    fn on_magnify(&mut self, delta: f64, now: Instant) {
        let step = self.pinch.push(delta);
        self.pinch_last = Some(now);
        if step.ctrl_down {
            self.send_key(
                KeyEvent {
                    scan: SC_LCTRL,
                    down: true,
                    autorepeat: false,
                },
                now,
            );
        }
        if step.wheel != 0 {
            self.send_fast(InputEvent::Scroll {
                dx: 0,
                dy: step.wheel,
                phase: ScrollPhase::Changed,
            });
        }
    }

    /// Fim do gesto de pinça: solta o Ctrl que a pinça segurou.
    fn end_pinch(&mut self, now: Instant) {
        self.pinch_last = None;
        if self.pinch.end() {
            self.send_key(
                KeyEvent {
                    scan: SC_LCTRL,
                    down: false,
                    autorepeat: false,
                },
                now,
            );
        }
    }

    fn end_pinch_if_idle(&mut self, now: Instant) {
        if self
            .pinch_last
            .is_some_and(|t| now.saturating_duration_since(t) >= PINCH_IDLE)
        {
            self.end_pinch(now);
        }
    }

    fn on_rotate(&mut self, delta: f64, now: Instant) {
        let Some((combo, times)) = self.rotation.push(delta) else {
            return;
        };
        let Some((mask, key)) = combo_to_scan(combo) else {
            return;
        };
        for _ in 0..times.min(8) {
            self.tap_combo(mask, key, now);
        }
    }

    fn tap_combo(&mut self, mask: u8, key: u16, now: Instant) {
        let mods = [(1, SC_LCTRL), (2, SC_LALT), (4, SC_LSHIFT), (8, SC_LWIN)];
        let key_ev = |down| KeyEvent {
            scan: key,
            down,
            autorepeat: false,
        };
        for (bit, scan) in mods {
            if mask & bit != 0 {
                self.send_key(
                    KeyEvent {
                        scan,
                        down: true,
                        autorepeat: false,
                    },
                    now,
                );
            }
        }
        self.send_key(key_ev(true), now);
        self.send_key(key_ev(false), now);
        for (bit, scan) in mods.into_iter().rev() {
            if mask & bit != 0 {
                self.send_key(
                    KeyEvent {
                        scan,
                        down: false,
                        autorepeat: false,
                    },
                    now,
                );
            }
        }
    }

    // ---- atalhos ----

    fn on_action(&mut self, action: HotkeyAction, now: Instant) {
        match action {
            HotkeyAction::JumpTo(n) => self.jump_to(n, now),
            HotkeyAction::ToggleLock => self.set_locked(!self.locked),
            HotkeyAction::TogglePause => self.set_paused(!self.paused, now),
            other => self.emit(EngineEvent::Hotkey(other)),
        }
    }

    /// Pula para a tela de número `n` (D19). Ignorado pausado ou travado.
    pub fn jump_to(&mut self, n: u8, now: Instant) {
        if self.paused {
            return;
        }
        let Some(LocalDecision::Enter {
            display_id,
            entry_px,
        }) = self.transition.jump_to(n)
        else {
            return;
        };
        let target_is_local = self
            .layout
            .local
            .iter()
            .any(|d| d.id == display_id && d.machine == self.local_machine);
        match (self.focus, target_is_local) {
            (Focus::Local, false) => self.enter_remote(display_id, entry_px),
            (Focus::Local, true) => {
                let _ = self.cursor.warp(entry_px.0, entry_px.1);
                self.last_pos = entry_px;
            }
            (Focus::Remote, true) => self.leave_remote(display_id, entry_px, now),
            (Focus::Remote, false) => self.send_control(Message::FocusEnter {
                display_id,
                entry_px,
            }),
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.transition.set_locked(locked);
        self.emit(EngineEvent::LockChanged(locked));
    }

    /// Pausar devolve o foco ao Mac e para de encaminhar.
    pub fn set_paused(&mut self, paused: bool, now: Instant) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if paused && self.focus == Focus::Remote {
            let id = self
                .layout
                .local
                .get(self.cur_display)
                .map(|d| d.id.clone())
                .unwrap_or_default();
            self.leave_remote(id, self.last_pos, now);
        }
        self.emit(EngineEvent::PauseChanged(paused));
    }

    // ---- mensagens do par ----

    /// Mensagem recebida da sessão (mesma thread da captura).
    pub fn on_message(&mut self, msg: Message, now: Instant) {
        match msg {
            Message::FocusReturn {
                display_id,
                point_px,
            } => {
                // O Mac é a autoridade da trava (D11 fica com o receptor, a trava com o Mac).
                if self.focus == Focus::Remote && !self.locked {
                    self.leave_remote(display_id, point_px, now);
                }
            }
            Message::ForegroundApp { exe } => self.foreground = Some(exe),
            Message::Displays(d) => {
                self.layout.peer = d;
                self.rebuild_transition();
            }
            _ => {}
        }
    }

    // ---- troca de foco ----

    fn focus_changed(&mut self, remote: bool) {
        self.emit(EngineEvent::FocusChanged { remote });
        // Só o lado que PERDE o foco (entrou no remoto) oferece o clipboard.
        if remote && self.clipboard.enabled && self.clipboard.mode == ClipboardMode::OnSwitch {
            self.emit(EngineEvent::ClipboardSwitch);
        }
    }

    fn enter_remote(&mut self, display_id: String, entry_px: (i32, i32)) {
        // O Mac perde o foco: solta o que estava segurado aqui.
        let held = std::mem::take(&mut self.local_held);
        for &scan in held.iter().rev() {
            let _ = self.injector.inject(&InputEvent::Key { scan, down: false });
        }
        self.translator = KeymapTranslator::new(self.keymap_cfg.clone());
        self.scroll_x.reset();
        self.scroll_y.reset();
        self.pinch.end();
        self.pinch_last = None;
        self.send_control(Message::FocusEnter {
            display_id,
            entry_px,
        });
        self.capture.set_swallow(true);
        let _ = self.cursor.hide();
        let _ = self.cursor.associate(false);
        self.focus = Focus::Remote;
        self.focus_changed(true);
    }

    /// Devolve o foco ao Mac em `point` (espaço do Mac) da tela `mac_display`.
    fn leave_remote(&mut self, mac_display: String, point: (i32, i32), now: Instant) {
        // O Windows perde o foco: solta tudo o que ele vê pressionado, na ordem inversa.
        self.end_pinch(now);
        let ups = self.tracker.release_all();
        for ev in ups {
            self.send_input_reliable(
                InputEvent::Key {
                    scan: ev.scan,
                    down: false,
                },
                now,
            );
        }
        self.translator = KeymapTranslator::new(self.keymap_cfg.clone());
        // Confirma ao receptor (que também para a repetição de tecla).
        self.send_control(Message::FocusReturn {
            display_id: mac_display,
            point_px: point,
        });
        self.capture.set_swallow(false);
        // Solta o cursor antes do warp: no Windows o ClipCursor de 1 px ainda prenderia o
        // SetCursorPos.
        let _ = self.cursor.associate(true);
        let _ = self.cursor.warp(point.0, point.1);
        let _ = self.cursor.show();
        self.last_pos = point;
        self.local_held.clear();
        self.focus = Focus::Local;
        self.focus_changed(false);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use rtrb::{Consumer, RingBuffer};

    use super::*;
    use crate::engine::testutil::*;
    use crate::test_alloc::measure;
    use crate::types::{KeyCode, MediaKey, MouseButton};

    const CTRL: u16 = 0x1D;
    const ALT: u16 = 0x38;
    const LWIN: u16 = 0xE05B;
    const KEY_A: u16 = 0x1E;
    const KEY_TAB: u16 = 0x0F;
    const KEY_C: u16 = 0x2E;
    const KEY_L: u16 = 0x26;
    const KEY_P: u16 = 0x19;
    const DIGIT_1: u16 = 0x02;
    const DIGIT_2: u16 = 0x03;

    type Ctl = Controller<MockCapture, MockCursor, MockInjector>;

    struct Rig {
        ctl: Ctl,
        log: Log,
        fast: Consumer<InputEvent>,
        control: Consumer<Message>,
        injected: Arc<Mutex<Vec<InputEvent>>>,
        events: Arc<Mutex<Vec<EngineEvent>>>,
        t0: Instant,
    }

    fn config(dwell_ms: u32, mode: ClipboardMode) -> Config {
        let mut c = Config::default();
        c.transition.dwell_ms = dwell_ms;
        c.clipboard.mode = mode;
        c
    }

    fn rig_with(cfg: &Config) -> Rig {
        let log = new_log();
        let (fast_p, fast) = RingBuffer::new(32768);
        let (ctrl_p, control) = RingBuffer::new(32768);
        let inj = MockInjector::new();
        let injected = inj.events.clone();
        let mut ctl = Controller::new(
            cfg,
            profile(1.0),
            vec![mac_display()],
            MockCapture::new(&log),
            MockCursor::new(&log),
            inj,
            fast_p,
            ctrl_p,
        );
        let events: Arc<Mutex<Vec<EngineEvent>>> = Arc::default();
        let sink = events.clone();
        ctl.set_event_handler(Box::new(move |e| sink.lock().unwrap().push(e)));
        ctl.on_message(Message::Displays(vec![dell_display()]), Instant::now());
        Rig {
            ctl,
            log,
            fast,
            control,
            injected,
            events,
            t0: Instant::now(),
        }
    }

    fn rig() -> Rig {
        rig_with(&config(0, ClipboardMode::OnSwitch))
    }

    impl Rig {
        fn key(&mut self, scan: u16, down: bool) {
            self.ctl.handle_at(
                CapturedEvent::Key {
                    code: KeyCode::Scan(scan),
                    down,
                    flags: 0,
                    autorepeat: false,
                },
                self.t0,
            );
        }

        fn tap(&mut self, scan: u16) {
            self.key(scan, true);
            self.key(scan, false);
        }

        fn mv(&mut self, dx: i32, dy: i32, pos: (f64, f64)) {
            self.ctl
                .handle_at(CapturedEvent::MouseMove { dx, dy, pos }, self.t0);
        }

        /// Empurra a borda direita do Mac na altura 450 e entra no Dell.
        fn enter_dell(&mut self) {
            self.mv(5, 0, (1439.0, 450.0));
            assert!(self.ctl.is_remote());
            self.log.lock().unwrap().clear();
            self.drain_control();
        }

        fn drain_control(&mut self) -> Vec<Message> {
            let mut v = Vec::new();
            while let Ok(m) = self.control.pop() {
                v.push(m);
            }
            v
        }

        fn drain_fast(&mut self) -> Vec<InputEvent> {
            let mut v = Vec::new();
            while let Ok(m) = self.fast.pop() {
                v.push(m);
            }
            v
        }
    }

    fn keys(msgs: &[Message]) -> Vec<(u16, bool)> {
        msgs.iter()
            .filter_map(|m| match m {
                Message::Input {
                    event: InputEvent::Key { scan, down },
                    ..
                } => Some((*scan, *down)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn borda_gera_focus_enter_e_depois_so_mouse_move() {
        let mut r = rig();
        r.mv(5, 0, (1439.0, 450.0));
        let msgs = r.drain_control();
        match msgs.as_slice() {
            [
                Message::FocusEnter {
                    display_id,
                    entry_px,
                },
            ] => {
                assert_eq!(display_id, DELL);
                assert_eq!(entry_px.0, 0);
                assert!((entry_px.1 - 830).abs() <= 2, "{entry_px:?}");
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(calls(&r.log), ["swallow(true)", "hide", "associate(false)"]);
        assert!(r.drain_fast().is_empty(), "o movimento da troca não vaza");
        r.mv(3, -2, (0.0, 0.0));
        r.mv(100_000, -100_000, (0.0, 0.0));
        assert_eq!(
            r.drain_fast(),
            [
                InputEvent::MouseMove { dx: 3, dy: -2 },
                InputEvent::MouseMove {
                    dx: i16::MAX,
                    dy: i16::MIN
                }
            ]
        );
    }

    #[test]
    fn focus_return_solta_teclas_e_devolve_o_cursor() {
        let mut r = rig();
        r.enter_dell();
        r.key(KEY_A, true);
        let down = r.drain_control();
        assert_eq!(keys(&down), [(KEY_A, true)]);
        r.ctl.on_message(
            Message::FocusReturn {
                display_id: MAC.into(),
                point_px: (1439, 450),
            },
            r.t0,
        );
        let msgs = r.drain_control();
        // keyUp antes de qualquer outra coisa, depois a confirmação ao receptor.
        assert_eq!(keys(&msgs), [(KEY_A, false)]);
        assert!(matches!(msgs[0], Message::Input { .. }));
        assert!(matches!(
            msgs.last(),
            Some(Message::FocusReturn {
                point_px: (1439, 450),
                ..
            })
        ));
        assert_eq!(
            calls(&r.log),
            [
                "swallow(false)",
                "associate(true)",
                "warp(1439,450)",
                "show"
            ]
        );
        assert!(!r.ctl.is_remote());
        // volta a seguir o mouse local: nada vai para a fila rápida
        r.mv(2, 2, (100.0, 100.0));
        assert!(r.drain_fast().is_empty());
    }

    #[test]
    fn hotkey_de_volta_com_cmd_segurado_nao_deixa_tecla_presa() {
        let mut r = rig();
        r.enter_dell();
        let mut msgs = Vec::new();
        r.key(LWIN, true);
        r.key(KEY_TAB, true); // Cmd+Tab: o Alt fica segurado no destino enquanto Cmd estiver
        r.key(CTRL, true);
        r.key(ALT, true);
        msgs.extend(r.drain_control());
        let held: std::collections::HashSet<u16> = keys(&msgs)
            .iter()
            .fold(Vec::new(), |mut v: Vec<(u16, bool)>, k| {
                v.push(*k);
                v
            })
            .into_iter()
            .filter(|k| k.1)
            .map(|k| k.0)
            .collect();
        assert!(held.contains(&CTRL) && held.contains(&KEY_TAB), "{held:?}");
        // Ctrl+Option+1 com Cmd ainda pressionado.
        r.key(DIGIT_1, true);
        msgs.extend(r.drain_control());
        let mut state = std::collections::HashSet::new();
        for (scan, down) in keys(&msgs) {
            if down {
                state.insert(scan);
            } else {
                state.remove(&scan);
            }
        }
        assert!(state.is_empty(), "teclas presas no destino: {state:?}");
        assert!(matches!(msgs.last(), Some(Message::FocusReturn { .. })));
        assert!(!r.ctl.is_remote());
        assert_eq!(
            &calls(&r.log)[..3],
            ["swallow(false)", "associate(true)", "warp(720,450)"],
            "a hotkey 1 volta ao centro da tela do Mac"
        );
        // o dígito da hotkey nunca vaza para o destino
        assert!(!keys(&msgs).iter().any(|k| k.0 == DIGIT_1));
    }

    #[test]
    fn lado_que_perde_o_foco_ao_sair_do_mac_recebe_keyup() {
        let mut r = rig();
        r.key(LWIN, true);
        r.key(CTRL, true);
        r.key(ALT, true);
        r.key(KEY_A, true);
        r.key(KEY_A, false);
        r.key(DIGIT_2, true);
        assert!(r.ctl.is_remote());
        let ups: Vec<_> = r
            .injected
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                InputEvent::Key { scan, down: false } => *scan,
                o => panic!("{o:?}"),
            })
            .collect();
        assert_eq!(ups, [ALT, CTRL, LWIN]);
        let msgs = r.drain_control();
        assert!(matches!(
            msgs.as_slice(),
            [Message::FocusEnter { display_id, entry_px: (1280, 720) }] if display_id == DELL
        ));
    }

    #[test]
    fn clipboard_on_switch_so_ao_perder_o_foco() {
        let mut r = rig();
        r.enter_dell();
        r.ctl.on_message(
            Message::FocusReturn {
                display_id: MAC.into(),
                point_px: (1439, 450),
            },
            r.t0,
        );
        let ev = r.events.lock().unwrap().clone();
        assert_eq!(
            ev,
            [
                EngineEvent::FocusChanged { remote: true },
                EngineEvent::ClipboardSwitch,
                EngineEvent::FocusChanged { remote: false },
            ]
        );
    }

    #[test]
    fn clipboard_on_copy_nao_dispara_na_troca() {
        let mut r = rig_with(&config(0, ClipboardMode::OnCopy));
        r.enter_dell();
        assert!(
            !r.events
                .lock()
                .unwrap()
                .contains(&EngineEvent::ClipboardSwitch)
        );
    }

    #[test]
    fn travar_impede_a_entrada_e_o_pulo() {
        let mut r = rig();
        r.key(CTRL, true);
        r.key(ALT, true);
        r.tap(KEY_L);
        r.key(ALT, false);
        r.key(CTRL, false);
        assert!(r.ctl.is_locked());
        r.mv(5, 0, (1439.0, 450.0));
        assert!(!r.ctl.is_remote());
        assert!(r.drain_control().is_empty());
        r.key(CTRL, true);
        r.key(ALT, true);
        r.tap(DIGIT_2);
        assert!(!r.ctl.is_remote(), "o pulo também respeita a trava");
        // destravar volta ao normal
        r.tap(KEY_L);
        assert!(!r.ctl.is_locked());
        r.mv(5, 0, (1439.0, 450.0));
        assert!(r.ctl.is_remote());
    }

    #[test]
    fn pausar_devolve_o_foco_e_para_de_encaminhar() {
        let mut r = rig();
        r.mv(1, 1, (1000.0, 300.0));
        r.mv(5, 0, (1439.0, 450.0));
        assert!(r.ctl.is_remote());
        r.drain_control();
        r.log.lock().unwrap().clear();
        r.key(CTRL, true);
        r.key(ALT, true);
        r.tap(KEY_P);
        assert!(r.ctl.is_paused());
        assert!(!r.ctl.is_remote());
        assert_eq!(
            calls(&r.log),
            [
                "swallow(false)",
                "associate(true)",
                "warp(1439,450)",
                "show"
            ]
        );
        assert!(matches!(
            r.drain_control().last(),
            Some(Message::FocusReturn { .. })
        ));
        // pausado: nem a borda nem teclas/botões/rolagem encaminham
        r.mv(5, 0, (1439.0, 450.0));
        r.tap(KEY_A);
        r.ctl.handle_at(
            CapturedEvent::Button {
                button: MouseButton::Left,
                down: true,
            },
            r.t0,
        );
        assert!(!r.ctl.is_remote());
        assert!(r.drain_control().is_empty() && r.drain_fast().is_empty());
        // despausar
        r.tap(KEY_P);
        assert!(!r.ctl.is_paused());
        r.mv(5, 0, (1439.0, 450.0));
        assert!(r.ctl.is_remote());
    }

    #[test]
    fn focus_return_e_ignorado_com_o_mac_travado() {
        let mut r = rig();
        r.enter_dell();
        r.ctl.set_locked(true);
        r.ctl.on_message(
            Message::FocusReturn {
                display_id: MAC.into(),
                point_px: (1439, 450),
            },
            r.t0,
        );
        assert!(r.ctl.is_remote());
        assert!(calls(&r.log).is_empty());
    }

    #[test]
    fn dwell_conta_com_o_cursor_preso_sem_filtrar_deltas() {
        let mut r = rig_with(&config(100, ClipboardMode::OnSwitch));
        let ms = Duration::from_millis;
        for t in [0, 40, 80] {
            r.ctl.handle_at(
                CapturedEvent::MouseMove {
                    dx: 5,
                    dy: 0,
                    pos: (1439.0, 450.0),
                },
                r.t0 + ms(t),
            );
            assert!(!r.ctl.is_remote(), "t={t}");
        }
        r.ctl.handle_at(
            CapturedEvent::MouseMove {
                dx: 5,
                dy: 0,
                pos: (1439.0, 450.0),
            },
            r.t0 + ms(120),
        );
        assert!(r.ctl.is_remote());
    }

    #[test]
    fn scroll_botao_e_midia_seguem_os_canais_certos() {
        let mut r = rig();
        r.enter_dell();
        r.ctl.handle_at(
            CapturedEvent::Scroll {
                dx_px: 0.0,
                dy_px: 40.0,
                phase: ScrollPhase::Changed,
                continuous: true,
            },
            r.t0,
        );
        r.ctl.handle_at(
            CapturedEvent::Button {
                button: MouseButton::Right,
                down: true,
            },
            r.t0,
        );
        r.ctl.handle_at(
            CapturedEvent::Media {
                key: MediaKey::Mute,
                down: true,
            },
            r.t0,
        );
        assert_eq!(
            r.drain_fast(),
            [InputEvent::Scroll {
                dx: 0,
                dy: 120,
                phase: ScrollPhase::Changed
            }]
        );
        let kinds: Vec<_> = r
            .drain_control()
            .into_iter()
            .map(|m| match m {
                Message::Input { event, .. } => event,
                o => panic!("{o:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                InputEvent::Button {
                    button: MouseButton::Right,
                    down: true
                },
                InputEvent::Media {
                    key: MediaKey::Mute,
                    down: true
                }
            ]
        );
    }

    #[test]
    fn pinca_vira_ctrl_scroll_e_solta_o_ctrl_ao_parar() {
        let mut r = rig();
        r.enter_dell();
        r.ctl
            .handle_at(CapturedEvent::Magnify { delta: 0.25 }, r.t0);
        assert_eq!(keys(&r.drain_control()), [(CTRL, true)]);
        assert_eq!(
            r.drain_fast(),
            [InputEvent::Scroll {
                dx: 0,
                dy: 240,
                phase: ScrollPhase::Changed
            }]
        );
        r.ctl.tick(r.t0 + Duration::from_millis(50));
        assert!(r.drain_control().is_empty());
        r.ctl.tick(r.t0 + Duration::from_millis(200));
        assert_eq!(keys(&r.drain_control()), [(CTRL, false)]);
    }

    #[test]
    fn dez_mil_eventos_sem_alocar() {
        let mut r = rig();
        r.enter_dell();
        let seq = |i: u32| -> [CapturedEvent; 8] {
            let key = |scan, down| CapturedEvent::Key {
                code: KeyCode::Scan(scan),
                down,
                flags: 0,
                autorepeat: false,
            };
            [
                CapturedEvent::MouseMove {
                    dx: (i % 7) as i32 - 3,
                    dy: 2,
                    pos: (10.0, 10.0),
                },
                CapturedEvent::Scroll {
                    dx_px: 1.0,
                    dy_px: 7.5,
                    phase: ScrollPhase::Changed,
                    continuous: true,
                },
                CapturedEvent::Button {
                    button: MouseButton::Left,
                    down: i % 2 == 0,
                },
                key(LWIN, true),
                key(KEY_C, true),
                key(KEY_C, false),
                key(LWIN, false),
                CapturedEvent::Media {
                    key: MediaKey::PlayPause,
                    down: true,
                },
            ]
        };
        let mut n = 0u32;
        let t0 = r.t0;
        let (_, allocs, _) = measure(|| {
            while n < 10_000 {
                for ev in seq(n) {
                    r.ctl.handle_at(ev, t0);
                    n += 1;
                }
            }
        });
        assert_eq!(allocs, 0);
        assert_eq!(r.ctl.dropped(), (0, 0));
        // e no foco local também (movimento que não cruza a borda)
        let mut r = rig();
        let t0 = r.t0;
        let (_, allocs, _) = measure(|| {
            for i in 0..10_000 {
                r.ctl.handle_at(
                    CapturedEvent::MouseMove {
                        dx: 1,
                        dy: 1,
                        pos: (100.0 + f64::from(i % 500), 300.0),
                    },
                    t0,
                );
            }
        });
        assert_eq!(allocs, 0);
    }
}
