//! dono: R1
//! Sentido inverso (D24, D25): o teclado do Windows controla o Mac. O motor é o mesmo da ida
//! (`Controller::set_reverse` / `Receiver::set_machine`); aqui fica só a tradução posicional dos
//! modificadores, que o `MacInjector` completa com a tabela scan → keycode (C5a).

use crate::keymap::translator::modifier_slot;
use crate::types::{Modifier, ReverseModifiers};

const SC_LCTRL: u16 = 0x1D;
const SC_RCTRL: u16 = 0xE01D;
const SC_LALT: u16 = 0x38;
const SC_RALT: u16 = 0xE038;
const SC_LSHIFT: u16 = 0x2A;
const SC_RSHIFT: u16 = 0x36;
const SC_LWIN: u16 = 0xE05B;
const SC_RWIN: u16 = 0xE05C;

/// Scan (set 1) do modificador `m`, no lado esquerdo ou direito.
fn scan_of_modifier(m: Modifier, right: bool) -> u16 {
    match (m, right) {
        (Modifier::Ctrl, false) => SC_LCTRL,
        (Modifier::Ctrl, true) => SC_RCTRL,
        (Modifier::Option, false) => SC_LALT,
        (Modifier::Option, true) => SC_RALT,
        (Modifier::Shift, false) => SC_LSHIFT,
        (Modifier::Shift, true) => SC_RSHIFT,
        // No protocolo o Cmd do Mac viaja como a tecla Win (tabela C5a).
        (Modifier::Cmd, false) => SC_LWIN,
        (Modifier::Cmd, true) => SC_RWIN,
    }
}

/// Tecla do Windows (scan) → scan que o Mac deve receber. Só Ctrl, Win e Alt mudam (D25); o
/// restante segue posicional e o lado (esquerdo/direito) é preservado.
pub fn map_scan(scan: u16, mods: &ReverseModifiers) -> u16 {
    if modifier_slot(scan).is_none() {
        return scan;
    }
    let right = matches!(scan, SC_RCTRL | SC_RALT | SC_RSHIFT | SC_RWIN);
    match scan {
        SC_LCTRL | SC_RCTRL => scan_of_modifier(mods.ctrl, right),
        SC_LWIN | SC_RWIN => scan_of_modifier(mods.win, right),
        SC_LALT | SC_RALT => scan_of_modifier(mods.alt, right),
        _ => scan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padrao_ctrl_cmd_win_ctrl_alt_option() {
        let m = ReverseModifiers::default();
        assert_eq!(map_scan(SC_LCTRL, &m), SC_LWIN);
        assert_eq!(map_scan(SC_RCTRL, &m), SC_RWIN);
        assert_eq!(map_scan(SC_LWIN, &m), SC_LCTRL);
        assert_eq!(map_scan(SC_RWIN, &m), SC_RCTRL);
        assert_eq!(map_scan(SC_LALT, &m), SC_LALT);
        assert_eq!(map_scan(SC_RALT, &m), SC_RALT);
    }

    #[test]
    fn shift_e_teclas_comuns_ficam() {
        let m = ReverseModifiers::default();
        assert_eq!(map_scan(SC_LSHIFT, &m), SC_LSHIFT);
        assert_eq!(map_scan(0x2E, &m), 0x2E);
        assert_eq!(map_scan(0xE04B, &m), 0xE04B);
    }

    #[test]
    fn configuravel() {
        let m = ReverseModifiers {
            ctrl: Modifier::Ctrl,
            win: Modifier::Cmd,
            alt: Modifier::Cmd,
        };
        assert_eq!(map_scan(SC_LCTRL, &m), SC_LCTRL);
        assert_eq!(map_scan(SC_LWIN, &m), SC_LWIN);
        assert_eq!(map_scan(SC_RALT, &m), SC_RWIN);
    }
}

/// Loopback do sentido inverso: `Controller` do Windows contra `Receiver` do Mac, só com mocks.
#[cfg(test)]
mod loopback {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use rtrb::{Consumer, RingBuffer};

    use crate::engine::testutil::*;
    use crate::engine::{Controller, Receiver};
    use crate::platform::InputCapture;
    use crate::types::{
        CapturedEvent, Config, InputEvent, KeyCode, Machine, Message, ReverseModifiers, ScrollPhase,
    };

    const CTRL: u16 = 0x1D;
    const ALT: u16 = 0x38;
    const WIN: u16 = 0xE05B;
    const KEY_C: u16 = 0x2E;
    const DIGIT_2: u16 = 0x03;

    struct Pair {
        win: Controller<MockCapture, MockCursor, MockInjector>,
        mac: Receiver<MockInjector, MockCursor>,
        win_log: Log,
        mac_inj: Arc<Mutex<Vec<InputEvent>>>,
        mac_pos: Arc<Mutex<(i32, i32)>>,
        win_inj: Arc<Mutex<Vec<InputEvent>>>,
        fast: Consumer<InputEvent>,
        control: Consumer<Message>,
        /// Mensagens que o Windows enviou, em ordem (para checar a ordem das liberações).
        wire: Vec<Message>,
        t0: Instant,
    }

    fn pair(dwell_ms: u32) -> Pair {
        let mut cfg = Config::default();
        cfg.transition.dwell_ms = dwell_ms;
        let win_log = new_log();
        let (fast_p, fast) = RingBuffer::new(4096);
        let (ctrl_p, control) = RingBuffer::new(4096);
        let win_inj = MockInjector::new();
        let win_events = win_inj.events.clone();
        let mut win = Controller::new(
            &cfg,
            profile(1.0),
            vec![dell_display()],
            MockCapture::new(&win_log),
            MockCursor::new(&win_log),
            win_inj,
            fast_p.into(),
            ctrl_p,
        );
        win.set_reverse(ReverseModifiers::default());
        let t0 = Instant::now();
        win.on_message(Message::Displays(vec![mac_display()]), t0);

        let mac_pos = Arc::new(Mutex::new((0, 0)));
        let mut mac_inj = MockInjector::new();
        mac_inj.cursor = Some((mac_pos.clone(), (0, 0, 1440, 900)));
        let mac_events = mac_inj.events.clone();
        let mac_log = new_log();
        let mut cursor = MockCursor::new(&mac_log);
        cursor.pos = mac_pos.clone();
        let mut mac = Receiver::new(&cfg, vec![mac_display()], mac_inj, cursor);
        mac.set_machine(Machine::Mac);
        mac.set_profile(profile(1.0));
        mac.on_message(Message::Displays(vec![dell_display()]), t0);
        Pair {
            win,
            mac,
            win_log,
            mac_inj: mac_events,
            mac_pos,
            win_inj: win_events,
            fast,
            control,
            wire: Vec::new(),
            t0,
        }
    }

    impl Pair {
        /// Entrega ao Mac tudo o que o Windows enfileirou e devolve as respostas ao Windows.
        fn pump(&mut self) {
            let now = self.t0;
            loop {
                let mut idle = true;
                while let Ok(ev) = self.fast.pop() {
                    idle = false;
                    let back = self.mac.on_input(ev, now);
                    self.back(back);
                }
                while let Ok(msg) = self.control.pop() {
                    idle = false;
                    self.wire.push(msg.clone());
                    let back = self.mac.on_message(msg, now);
                    self.back(back);
                }
                if idle {
                    break;
                }
            }
        }

        fn back(&mut self, msgs: Vec<Message>) {
            for m in msgs {
                self.win.on_message(m, self.t0);
            }
        }

        fn key(&mut self, scan: u16, down: bool) {
            self.win.handle_at(
                CapturedEvent::Key {
                    code: KeyCode::Scan(scan),
                    down,
                    flags: 0,
                    autorepeat: false,
                },
                self.t0,
            );
            self.pump();
        }

        fn mv(&mut self, dx: i32, dy: i32, pos: (f64, f64)) {
            self.win
                .handle_at(CapturedEvent::MouseMove { dx, dy, pos }, self.t0);
            self.pump();
        }

        /// Empurra a borda esquerda do Dell (ligada ao Mac) na altura de 830 px.
        fn enter_mac(&mut self) {
            self.mv(-5, 0, (0.0, 830.0));
            assert!(self.win.is_remote());
            assert!(self.mac.is_active());
        }

        fn mac_events(&self) -> Vec<InputEvent> {
            self.mac_inj.lock().unwrap().clone()
        }
    }

    fn key(scan: u16, down: bool) -> InputEvent {
        InputEvent::Key { scan, down }
    }

    #[test]
    fn windows_assume_pela_borda_e_o_mac_posiciona_o_cursor() {
        let mut p = pair(0);
        p.enter_mac();
        let (x, y) = *p.mac_pos.lock().unwrap();
        assert_eq!(x, 1439);
        assert!((y - 449).abs() <= 3, "y = {y}");
        assert!(calls(&p.win_log).contains(&"swallow(true)".to_string()));
        assert!(calls(&p.win_log).contains(&"hide".to_string()));
    }

    #[test]
    fn nao_assume_enquanto_o_mac_controla_o_windows() {
        let mut p = pair(0);
        // O AgentCore pausa o Controller reverso enquanto o Receiver do agente está ativo (D24).
        p.win.set_paused(true, p.t0);
        p.mv(-5, 0, (0.0, 830.0));
        assert!(!p.win.is_remote());
        assert!(!p.mac.is_active());
        assert!(p.wire.is_empty());
    }

    #[test]
    fn devolve_pela_borda_soltando_as_teclas_antes() {
        let mut p = pair(0);
        p.enter_mac();
        p.key(CTRL, true);
        p.wire.clear();
        // Move o cursor do Mac até a borda direita (ligada ao Dell).
        *p.mac_pos.lock().unwrap() = (1430, 449);
        p.mac_inj.lock().unwrap().clear();
        p.mv(9, 0, (0.0, 830.0));
        assert!(!p.win.is_remote(), "voltou para o Windows");
        assert!(!p.mac.is_active());
        assert!(!p.win.capture().is_swallowing());
        let order: Vec<_> = p
            .wire
            .iter()
            .map(|m| match m {
                Message::Input {
                    event: InputEvent::Key { scan, down },
                    ..
                } => format!("key {scan:x} {down}"),
                Message::FocusReturn { .. } => "return".into(),
                _ => "outro".into(),
            })
            .collect();
        assert_eq!(order, ["key e05b false", "return"], "{order:?}");
        // E o Mac soltou o Cmd.
        assert!(p.mac_events().contains(&key(WIN, false)));
    }

    #[test]
    fn devolve_por_hotkey() {
        let mut p = pair(0);
        p.enter_mac();
        p.key(CTRL, true);
        p.key(ALT, true);
        p.key(DIGIT_2, true);
        assert!(!p.win.is_remote());
        assert!(!p.mac.is_active());
        let ev = p.mac_events();
        assert!(ev.contains(&key(WIN, false)), "{ev:?}");
        assert!(ev.contains(&key(ALT, false)), "{ev:?}");
        // A tecla do atalho não vazou para o Mac.
        assert!(!ev.contains(&key(DIGIT_2, true)));
    }

    #[test]
    fn ctrl_c_do_pc_chega_como_cmd_c_no_mac() {
        let mut p = pair(0);
        p.enter_mac();
        p.mac_inj.lock().unwrap().clear();
        p.key(CTRL, true);
        p.key(KEY_C, true);
        // Repetição do sistema (down sem up) não vai para o Mac: ele gera a sua (D12).
        p.key(KEY_C, true);
        p.key(KEY_C, false);
        p.key(CTRL, false);
        let ev: Vec<_> = p
            .mac_events()
            .into_iter()
            .filter(|e| matches!(e, InputEvent::Key { .. }))
            .collect();
        assert_eq!(
            ev,
            [
                key(WIN, true),
                key(KEY_C, true),
                key(KEY_C, false),
                key(WIN, false)
            ]
        );
    }

    #[test]
    fn win_vira_control_e_alt_continua_option() {
        let mut p = pair(0);
        p.enter_mac();
        p.mac_inj.lock().unwrap().clear();
        p.key(WIN, true);
        p.key(ALT, true);
        p.key(ALT, false);
        p.key(WIN, false);
        assert_eq!(
            p.mac_events(),
            [
                key(CTRL, true),
                key(ALT, true),
                key(ALT, false),
                key(CTRL, false)
            ]
        );
    }

    #[test]
    fn teclas_seguradas_ao_cruzar_a_borda_sao_soltas_no_windows() {
        let mut p = pair(0);
        p.key(KEY_C, true);
        p.enter_mac();
        assert!(p.win_inj.lock().unwrap().contains(&key(KEY_C, false)));
        // O up físico depois não deixa tecla presa no Mac.
        p.key(KEY_C, false);
    }

    #[test]
    fn altgr_nao_dispara_atalhos_do_relay() {
        let mut p = pair(0);
        p.enter_mac();
        p.mac_inj.lock().unwrap().clear();
        // AltGr = LCtrl + RAlt; AltGr+2 não pode devolver o foco (Ctrl+Alt+2 é JumpTo(2)).
        p.key(CTRL, true);
        p.key(0xE038, true);
        p.key(DIGIT_2, true);
        p.key(DIGIT_2, false);
        assert!(p.win.is_remote());
        assert!(p.mac.is_active());
        assert!(p.mac_events().contains(&key(DIGIT_2, true)));
        p.key(0xE038, false);
        p.key(CTRL, false);
        // Ctrl+LAlt+2 continua sendo o atalho.
        p.key(CTRL, true);
        p.key(ALT, true);
        p.key(DIGIT_2, true);
        assert!(!p.win.is_remote());
    }

    #[test]
    fn receiver_so_oferece_o_clipboard_ao_perder_o_foco() {
        use crate::engine::EngineEvent;
        let mut p = pair(0);
        let ev: Arc<Mutex<Vec<EngineEvent>>> = Arc::default();
        let sink = ev.clone();
        p.mac
            .set_event_handler(Box::new(move |e| sink.lock().unwrap().push(e)));
        p.enter_mac();
        assert!(
            !ev.lock().unwrap().contains(&EngineEvent::ClipboardSwitch),
            "ganhar o foco não oferece"
        );
        p.key(CTRL, true);
        p.key(ALT, true);
        p.key(DIGIT_2, true);
        assert!(ev.lock().unwrap().contains(&EngineEvent::ClipboardSwitch));
    }

    #[test]
    fn devolve_solta_o_cursor_antes_do_warp() {
        let mut p = pair(0);
        p.enter_mac();
        p.win_log.lock().unwrap().clear();
        p.key(CTRL, true);
        p.key(ALT, true);
        p.key(DIGIT_2, true);
        let c = calls(&p.win_log);
        let a = c.iter().position(|s| s == "associate(true)").unwrap();
        let w = c.iter().position(|s| s.starts_with("warp(")).unwrap();
        assert!(a < w, "{c:?}");
    }

    #[test]
    fn roda_e_movimento_chegam_ao_mac() {
        let mut p = pair(0);
        p.enter_mac();
        p.mac_inj.lock().unwrap().clear();
        p.mv(-3, 2, (0.0, 830.0));
        p.win.handle_at(
            CapturedEvent::Scroll {
                dx_px: 0.0,
                dy_px: 1.0,
                phase: ScrollPhase::None,
                continuous: false,
            },
            p.t0,
        );
        p.pump();
        let ev = p.mac_events();
        assert!(
            ev.contains(&InputEvent::MouseMove { dx: -3, dy: 2 }),
            "{ev:?}"
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, InputEvent::Scroll { dy: 120, dx: 0, .. })),
            "{ev:?}"
        );
    }
}
