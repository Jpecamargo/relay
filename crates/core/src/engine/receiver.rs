//! dono: C10
//! Receiver: da mensagem recebida até a injeção (sensibilidade por tela, repetição de tecla,
//! detecção do retorno e comandos remotos).

use std::time::{Duration, Instant};

use super::{EngineEvent, EventHandler, LayoutState, sat_i16};
use crate::input_shaping::SensitivityScaler;
use crate::keymap::pressed::PressedTracker;
use crate::keymap::translator::{KeyEvent, modifier_slot};
use crate::platform::{CommandExecutor, CursorControl, InputInjector};
use crate::transition::{RemoteDecision, TransitionEngine};
use crate::types::{
    ClipboardConfig, ClipboardMode, CommandResult, Config, DisplayInfo, InputEvent, LayoutProfile,
    Machine, Message, RemoteCommand, TransitionConfig, error_code,
};

/// Reenvio do `FocusReturn` enquanto o Mac não confirma (travado no Mac, ou mensagem perdida).
const RETURN_RETRY: Duration = Duration::from_millis(250);
/// Teto de repetições geradas por `tick` se o relógio ficou parado (evita rajada).
const MAX_REPEATS_PER_TICK: u32 = 8;

#[derive(Debug, Clone, Copy)]
struct Repeat {
    scan: u16,
    next_due: Instant,
    period: Duration,
}

/// Lado que recebe e injeta (agente no Windows; o Mac no sentido inverso).
///
/// `on_message`/`on_input`/`tick` rodam na mesma thread. O motor não filtra nem acumula
/// deslocamento: a posição real do cursor (D11) decide o retorno.
pub struct Receiver<Inj, Cur> {
    injector: Inj,
    cursor: Cur,
    executor: Option<Box<dyn CommandExecutor>>,
    layout: LayoutState,
    transition_cfg: TransitionConfig,
    transition: TransitionEngine,
    sensitivity: SensitivityScaler,
    active: bool,
    /// Última posição conhecida do cursor local (define a tela sob o cursor).
    last_pos: (i32, i32),
    tracker: PressedTracker,
    repeat: Option<Repeat>,
    return_sent: Option<Instant>,
    /// Última linha de diagnóstico "borda não devolveu" (no máximo 1 por segundo).
    why_logged: Option<Instant>,
    clipboard: ClipboardConfig,
    on_event: Option<EventHandler>,
    /// Máquina em que o Receiver roda (Windows; Mac no sentido inverso, D24).
    machine: Machine,
}

impl<Inj: InputInjector, Cur: CursorControl> Receiver<Inj, Cur> {
    pub fn new(
        config: &Config,
        local_displays: Vec<DisplayInfo>,
        injector: Inj,
        cursor: Cur,
    ) -> Self {
        let layout = LayoutState::new(LayoutState::empty_profile(), local_displays);
        let transition = TransitionEngine::new(
            config.transition.clone(),
            layout.profile.clone(),
            layout.combined(),
        );
        Self {
            injector,
            cursor,
            executor: None,
            layout,
            transition_cfg: config.transition.clone(),
            transition,
            sensitivity: SensitivityScaler::default(),
            active: false,
            last_pos: (0, 0),
            tracker: PressedTracker::default(),
            repeat: None,
            return_sent: None,
            why_logged: None,
            clipboard: config.clipboard.clone(),
            on_event: None,
            machine: Machine::Windows,
        }
    }

    /// Sentido inverso (D24): o Receiver roda no Mac, controlado pelo agente.
    pub fn set_machine(&mut self, machine: Machine) {
        self.machine = machine;
        self.rebuild();
    }

    pub fn set_executor(&mut self, executor: Box<dyn CommandExecutor>) {
        self.executor = Some(executor);
    }

    pub fn set_event_handler(&mut self, handler: EventHandler) {
        self.on_event = Some(handler);
    }

    pub fn set_local_displays(&mut self, displays: Vec<DisplayInfo>) {
        self.layout.local = displays;
        self.rebuild();
    }

    /// Cópia do layout (vinda do Mac por `LayoutSync`): telas Windows com sensibilidade.
    pub fn set_profile(&mut self, profile: LayoutProfile) {
        self.layout.profile = profile;
        self.rebuild();
    }

    pub fn set_clipboard_config(&mut self, clipboard: ClipboardConfig) {
        self.clipboard = clipboard;
    }

    fn rebuild(&mut self) {
        self.transition = TransitionEngine::new(
            self.transition_cfg.clone(),
            self.layout.profile.clone(),
            self.layout.combined(),
        );
        // O motor trata "local" como a máquina que controla: o receptor é o outro lado.
        self.transition.set_local_machine(match self.machine {
            Machine::Windows => Machine::Mac,
            Machine::Mac => Machine::Windows,
        });
        self.sensitivity = SensitivityScaler::default();
        for p in &self.layout.profile.displays {
            self.sensitivity.set(&p.display_id, p.sensitivity);
        }
    }

    /// `true` enquanto o Mac controla este lado.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Tecla com repetição em andamento (para diagnóstico e testes).
    pub fn repeating(&self) -> Option<u16> {
        self.repeat.map(|r| r.scan)
    }

    fn emit(&mut self, event: EngineEvent) {
        if let Some(h) = self.on_event.as_mut() {
            h(event);
        }
    }

    fn focus_changed(&mut self, remote: bool) {
        self.emit(EngineEvent::FocusChanged { remote });
        // Só o lado que PERDE o foco oferece o clipboard; quem ganha espera a oferta do outro.
        if !remote && self.clipboard.enabled && self.clipboard.mode == ClipboardMode::OnSwitch {
            self.emit(EngineEvent::ClipboardSwitch);
        }
    }

    /// Mensagem do canal confiável. Devolve o que enviar de volta (`FocusReturn`,
    /// `CommandResult`).
    pub fn on_message(&mut self, msg: Message, now: Instant) -> Vec<Message> {
        match msg {
            Message::Input { event, .. } => self.on_input(event, now),
            Message::FocusEnter {
                display_id: _,
                entry_px,
            } => {
                self.on_focus_enter(entry_px);
                Vec::new()
            }
            Message::FocusReturn { .. } => {
                self.focus_lost();
                Vec::new()
            }
            Message::Displays(d) => {
                self.layout.peer = d;
                self.rebuild();
                Vec::new()
            }
            Message::LayoutSync(profile) => {
                self.set_profile(profile);
                Vec::new()
            }
            Message::Command { id, cmd } => vec![Message::CommandResult {
                id,
                result: self.run_command(id, &cmd),
            }],
            _ => Vec::new(),
        }
    }

    fn run_command(&mut self, id: u32, cmd: &RemoteCommand) -> CommandResult {
        if let RemoteCommand::TypeText { text } = cmd {
            return match self.injector.type_text(text) {
                Ok(()) => CommandResult::ok(id, None),
                Err(e) => CommandResult::err(id, e.to_string()),
            };
        }
        match self.executor.as_mut() {
            Some(ex) => {
                let mut r = ex.execute(id, cmd);
                r.id = id;
                r
            }
            None => CommandResult::err(id, error_code::UNSUPPORTED),
        }
    }

    fn on_focus_enter(&mut self, entry_px: (i32, i32)) {
        tracing::info!(entry = ?entry_px, active = self.active, "receptor: FocusEnter");
        if self.active {
            // Pulo entre telas do Windows: só reposiciona.
            let _ = self.cursor.warp(entry_px.0, entry_px.1);
            self.last_pos = entry_px;
            return;
        }
        let _ = self.cursor.warp(entry_px.0, entry_px.1);
        self.last_pos = entry_px;
        self.transition.landed_remote(entry_px);
        self.active = true;
        self.return_sent = None;
        self.focus_changed(true);
    }

    /// O Mac tomou o foco de volta (ou a conexão caiu): solta tudo o que está pressionado e para
    /// a repetição.
    pub fn focus_lost(&mut self) {
        self.repeat = None;
        let ups = self.tracker.release_all();
        for ev in ups {
            let _ = self.injector.inject(&InputEvent::Key {
                scan: ev.scan,
                down: false,
            });
        }
        if self.active {
            tracing::info!(pos = ?self.last_pos, "receptor: foco devolvido (confirmação do Mac)");
            self.active = false;
            self.focus_changed(false);
        }
    }

    /// Evento de input (canal rápido ou `Message::Input`). Ignorado sem foco.
    pub fn on_input(&mut self, event: InputEvent, now: Instant) -> Vec<Message> {
        if !self.active {
            return Vec::new();
        }
        match event {
            InputEvent::MouseMove { dx, dy } => return self.on_move(dx, dy, now),
            InputEvent::MouseAbs { x, y } => {
                let _ = self.injector.inject(&event);
                self.last_pos = (x, y);
            }
            InputEvent::Key { scan, down } => self.on_key(scan, down, now),
            InputEvent::Button { .. } | InputEvent::Scroll { .. } | InputEvent::Media { .. } => {
                let _ = self.injector.inject(&event);
            }
        }
        Vec::new()
    }

    fn display_under(&self, p: (i32, i32)) -> Option<usize> {
        self.layout.local.iter().position(|d| {
            d.machine == self.machine
                && p.0 >= d.origin_px.0
                && p.0 < d.origin_px.0 + d.size_px.0 as i32
                && p.1 >= d.origin_px.1
                && p.1 < d.origin_px.1 + d.size_px.1 as i32
        })
    }

    fn on_move(&mut self, dx: i16, dy: i16, now: Instant) -> Vec<Message> {
        // Multiplicador da tela sob o cursor (D11); sem tela conhecida, 1,0.
        let (sx, sy) = match self.display_under(self.last_pos) {
            Some(i) => {
                self.sensitivity
                    .scale(&self.layout.local[i].id, i32::from(dx), i32::from(dy))
            }
            None => (dx, dy),
        };
        if sx != 0 || sy != 0 {
            let _ = self.injector.inject(&InputEvent::MouseMove {
                dx: sat_i16(i32::from(sx)),
                dy: sat_i16(i32::from(sy)),
            });
        }
        // Sem posição do cursor não há como detectar a borda.
        let Ok(pos) = self.cursor.position() else {
            return Vec::new();
        };
        self.last_pos = pos;
        if self
            .return_sent
            .is_some_and(|t| now.saturating_duration_since(t) < RETURN_RETRY)
        {
            return Vec::new();
        }
        match self
            .transition
            .on_remote_cursor(pos, (i32::from(dx), i32::from(dy)), now)
        {
            RemoteDecision::Stay => {
                self.log_stay(pos, (i32::from(dx), i32::from(dy)), now);
                Vec::new()
            }
            RemoteDecision::Return {
                mac_display,
                point_px,
            } => {
                // Continua ativo até o Mac confirmar com `FocusReturn` (ele pode estar travado).
                tracing::info!(pos = ?pos, point = ?point_px, retry = self.return_sent.is_some(), "receptor: borda de retorno, FocusReturn enviado");
                self.return_sent = Some(now);
                vec![Message::FocusReturn {
                    display_id: mac_display,
                    point_px,
                }]
            }
        }
    }

    /// Diz por que o cursor sobre a borda de retorno não devolveu o foco.
    fn log_stay(&mut self, pos: (i32, i32), delta: (i32, i32), now: Instant) {
        let Some(why) = self.transition.remote_stay_why() else {
            return;
        };
        if self
            .why_logged
            .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(1))
        {
            return;
        }
        self.why_logged = Some(now);
        tracing::info!(?why, pos = ?pos, delta = ?delta, "receptor: na borda de retorno, foco não devolvido");
    }

    fn on_key(&mut self, scan: u16, down: bool, now: Instant) {
        let _ = self.injector.inject(&InputEvent::Key { scan, down });
        self.tracker.track(KeyEvent {
            scan,
            down,
            autorepeat: false,
        });
        if down {
            // Modificadores não repetem; a última tecla comum segurada é a que repete (D12).
            if modifier_slot(scan).is_none() {
                let kr = self.injector.key_repeat();
                self.repeat = (kr.rate_hz > 0.0 && kr.rate_hz.is_finite()).then(|| Repeat {
                    scan,
                    next_due: now + Duration::from_millis(u64::from(kr.delay_ms)),
                    period: Duration::from_secs_f64(1.0 / f64::from(kr.rate_hz)),
                });
            }
        } else if self.repeat.is_some_and(|r| r.scan == scan) {
            self.repeat = None;
        }
    }

    /// Gera as repetições de tecla vencidas. Chame com frequência (≤ 10 ms).
    pub fn tick(&mut self, now: Instant) {
        let Some(mut r) = self.repeat else { return };
        if !self.active {
            self.repeat = None;
            return;
        }
        let mut n = 0;
        while now >= r.next_due {
            if n == MAX_REPEATS_PER_TICK {
                r.next_due = now + r.period;
                break;
            }
            let _ = self.injector.inject(&InputEvent::Key {
                scan: r.scan,
                down: true,
            });
            r.next_due += r.period;
            n += 1;
        }
        self.repeat = Some(r);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::engine::testutil::*;
    use crate::types::CommandOutput;

    const KEY_A: u16 = 0x1E;
    const CTRL: u16 = 0x1D;

    type Rx = Receiver<MockInjector, MockCursor>;

    struct Rig {
        rx: Rx,
        injected: Arc<Mutex<Vec<InputEvent>>>,
        cursor: Arc<Mutex<(i32, i32)>>,
        t0: Instant,
    }

    fn rig(sens: f32) -> Rig {
        let mut cfg = Config::default();
        cfg.transition.dwell_ms = 0;
        cfg.transition.edge_resistance_pt = 0.0;
        let log = new_log();
        let cur = MockCursor::new(&log);
        let cursor = cur.pos.clone();
        let mut inj = MockInjector::new();
        inj.cursor = Some((cursor.clone(), (0, 0, 2560, 1440)));
        let injected = inj.events.clone();
        let mut rx = Receiver::new(&cfg, vec![dell_display()], inj, cur);
        let t0 = Instant::now();
        rx.on_message(Message::Displays(vec![mac_display()]), t0);
        rx.on_message(Message::LayoutSync(profile(sens)), t0);
        rx.on_message(
            Message::FocusEnter {
                display_id: DELL.into(),
                entry_px: (0, 830),
            },
            t0,
        );
        Rig {
            rx,
            injected,
            cursor,
            t0,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn key_downs(r: &Rig, scan: u16) -> usize {
        r.injected
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == InputEvent::Key { scan, down: true })
            .count()
    }

    #[test]
    fn borda_desarmada_pelo_pouso_nao_devolve_e_informa_o_motivo() {
        let mut r = rig(1.0);
        *r.cursor.lock().unwrap() = (0, 830);
        // Entrou na borda de retorno e empurra sem sair do lugar: desarmada pelo pouso.
        let back = r.rx.on_input(InputEvent::MouseMove { dx: -3, dy: 0 }, r.t0);
        assert!(back.is_empty());
        assert_eq!(
            r.rx.transition.remote_stay_why(),
            Some(crate::transition::StayWhy::Landed)
        );
        // Afastou e voltou: devolve.
        *r.cursor.lock().unwrap() = (20, 830);
        r.rx.on_input(InputEvent::MouseMove { dx: 5, dy: 0 }, r.t0);
        *r.cursor.lock().unwrap() = (0, 830);
        let back = r.rx.on_input(InputEvent::MouseMove { dx: -3, dy: 0 }, r.t0);
        assert!(matches!(back.as_slice(), [Message::FocusReturn { .. }]));
    }

    #[test]
    fn focus_enter_posiciona_o_cursor_e_ativa() {
        let r = rig(1.0);
        assert!(r.rx.is_active());
        assert_eq!(*r.cursor.lock().unwrap(), (0, 830));
    }

    #[test]
    fn aplica_a_sensibilidade_da_tela_sob_o_cursor() {
        let mut r = rig(2.0);
        r.rx.on_input(InputEvent::MouseMove { dx: 5, dy: 3 }, r.t0);
        assert_eq!(
            r.injected.lock().unwrap().as_slice(),
            [InputEvent::MouseMove { dx: 10, dy: 6 }]
        );
        let mut r = rig(0.5);
        r.rx.on_input(InputEvent::MouseMove { dx: 1, dy: 1 }, r.t0);
        r.rx.on_input(InputEvent::MouseMove { dx: 1, dy: 1 }, r.t0);
        // resto fracionário acumulado: só o 2º movimento anda 1 px
        assert_eq!(r.injected.lock().unwrap().len(), 1);
    }

    #[test]
    fn repete_tecla_com_delay_e_taxa_do_destino() {
        let mut r = rig(1.0);
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: true,
            },
            r.t0,
        );
        for t in (10..=600).step_by(10) {
            r.rx.tick(r.t0 + ms(t));
        }
        let repeats = key_downs(&r, KEY_A) - 1;
        assert!((2..=4).contains(&repeats), "repeats={repeats}");
        // keyUp para a repetição
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: false,
            },
            r.t0 + ms(600),
        );
        let n = key_downs(&r, KEY_A);
        r.rx.tick(r.t0 + ms(900));
        assert_eq!(key_downs(&r, KEY_A), n);
        assert_eq!(r.rx.repeating(), None);
    }

    #[test]
    fn antes_do_delay_nao_repete_e_modificador_nunca_repete() {
        let mut r = rig(1.0);
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: true,
            },
            r.t0,
        );
        r.rx.tick(r.t0 + ms(499));
        assert_eq!(key_downs(&r, KEY_A), 1);
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: false,
            },
            r.t0 + ms(499),
        );
        r.rx.on_input(
            InputEvent::Key {
                scan: CTRL,
                down: true,
            },
            r.t0,
        );
        r.rx.tick(r.t0 + ms(2000));
        assert_eq!(key_downs(&r, CTRL), 1);
    }

    #[test]
    fn perda_de_foco_solta_teclas_e_para_a_repeticao() {
        let mut r = rig(1.0);
        r.rx.on_input(
            InputEvent::Key {
                scan: CTRL,
                down: true,
            },
            r.t0,
        );
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: true,
            },
            r.t0,
        );
        r.rx.on_message(
            Message::FocusReturn {
                display_id: MAC.into(),
                point_px: (1439, 450),
            },
            r.t0 + ms(100),
        );
        assert!(!r.rx.is_active());
        let ev = r.injected.lock().unwrap().clone();
        assert_eq!(
            &ev[2..],
            [
                InputEvent::Key {
                    scan: KEY_A,
                    down: false
                },
                InputEvent::Key {
                    scan: CTRL,
                    down: false
                }
            ]
        );
        r.rx.tick(r.t0 + ms(2000));
        assert_eq!(key_downs(&r, KEY_A), 1);
        // sem foco, input é ignorado
        r.rx.on_input(
            InputEvent::Key {
                scan: KEY_A,
                down: true,
            },
            r.t0,
        );
        assert_eq!(key_downs(&r, KEY_A), 1);
    }

    #[test]
    fn detecta_o_retorno_pela_posicao_real_e_emite_focus_return() {
        let mut r = rig(1.0);
        r.rx.on_input(InputEvent::MouseMove { dx: 5, dy: 0 }, r.t0);
        *r.cursor.lock().unwrap() = (0, 830);
        let out = r.rx.on_input(InputEvent::MouseMove { dx: -5, dy: 0 }, r.t0);
        match out.as_slice() {
            [
                Message::FocusReturn {
                    display_id,
                    point_px,
                },
            ] => {
                assert_eq!(display_id, MAC);
                assert_eq!(point_px.0, 1439);
                assert!((point_px.1 - 450).abs() <= 3, "{point_px:?}");
            }
            o => panic!("{o:?}"),
        }
        // o Mac ainda não confirmou: segue ativo, sem repetir a mensagem de imediato
        assert!(r.rx.is_active());
        assert!(
            r.rx.on_input(InputEvent::MouseMove { dx: -5, dy: 0 }, r.t0 + ms(50))
                .is_empty()
        );
        assert_eq!(
            r.rx.on_input(InputEvent::MouseMove { dx: -5, dy: 0 }, r.t0 + ms(300))
                .len(),
            1
        );
        // no meio da tela não há retorno
        *r.cursor.lock().unwrap() = (1000, 500);
        assert!(
            r.rx.on_input(InputEvent::MouseMove { dx: 5, dy: 0 }, r.t0 + ms(600))
                .is_empty()
        );
    }

    #[test]
    fn dwell_do_retorno_conta_com_o_cursor_preso() {
        let mut cfg = Config::default();
        cfg.transition.dwell_ms = 100;
        cfg.transition.edge_resistance_pt = f32::MAX;
        let log = new_log();
        let cur = MockCursor::new(&log);
        let cursor = cur.pos.clone();
        let mut inj = MockInjector::new();
        inj.cursor = Some((cursor.clone(), (0, 0, 2560, 1440)));
        let mut rx = Receiver::new(&cfg, vec![dell_display()], inj, cur);
        let t0 = Instant::now();
        rx.on_message(Message::Displays(vec![mac_display()]), t0);
        rx.on_message(Message::LayoutSync(profile(1.0)), t0);
        rx.on_message(
            Message::FocusEnter {
                display_id: DELL.into(),
                entry_px: (0, 830),
            },
            t0,
        );
        let mv = InputEvent::MouseMove { dx: -5, dy: 0 };
        rx.on_input(InputEvent::MouseMove { dx: 5, dy: 0 }, t0);
        *cursor.lock().unwrap() = (0, 830);
        assert!(rx.on_input(mv, t0).is_empty());
        assert!(rx.on_input(mv, t0 + ms(60)).is_empty());
        assert_eq!(rx.on_input(mv, t0 + ms(110)).len(), 1);
    }

    /// Cursor colado na borda a 125 Hz, empurrando 3 pt por evento: com a pressão padrão o
    /// retorno sai no 3º evento (16 ms); antes, só depois de `dwell_ms` = 100 ms parado na borda.
    #[test]
    fn retorno_por_pressao_sai_em_16_ms_e_nao_em_100() {
        let first_return = |cfg: &Config| {
            let log = new_log();
            let cur = MockCursor::new(&log);
            let cursor = cur.pos.clone();
            let mut inj = MockInjector::new();
            inj.cursor = Some((cursor.clone(), (0, 0, 2560, 1440)));
            let mut rx = Receiver::new(cfg, vec![dell_display()], inj, cur);
            let t0 = Instant::now();
            rx.on_message(Message::Displays(vec![mac_display()]), t0);
            rx.on_message(Message::LayoutSync(profile(1.0)), t0);
            rx.on_message(
                Message::FocusEnter {
                    display_id: DELL.into(),
                    entry_px: (0, 830),
                },
                t0,
            );
            rx.on_input(InputEvent::MouseMove { dx: 5, dy: 0 }, t0);
            *cursor.lock().unwrap() = (0, 830);
            let mv = InputEvent::MouseMove { dx: -3, dy: 0 };
            (0..200u64).find(|i| !rx.on_input(mv, t0 + ms(i * 8)).is_empty())
        };
        let mut antes = Config::default();
        antes.transition.dwell_ms = 100;
        antes.transition.edge_resistance_pt = f32::MAX;
        let mut depois = Config::default();
        depois.transition.dwell_ms = 0;
        assert_eq!(first_return(&antes), Some(13)); // 13 x 8 ms = 104 ms
        assert_eq!(first_return(&depois), Some(2)); // 3 eventos: 0, 8 e 16 ms
    }

    #[test]
    fn comandos_sao_despachados_e_respondidos_com_o_mesmo_id() {
        struct Ex;
        impl CommandExecutor for Ex {
            fn execute(&mut self, id: u32, _c: &RemoteCommand) -> CommandResult {
                CommandResult::ok(id, Some(CommandOutput::Text("ok".into())))
            }
        }
        let mut r = rig(1.0);
        let out = r.rx.on_message(
            Message::Command {
                id: 7,
                cmd: RemoteCommand::GetMetrics,
            },
            r.t0,
        );
        assert_eq!(
            out,
            [Message::CommandResult {
                id: 7,
                result: CommandResult::err(7, "unsupported")
            }]
        );
        r.rx.set_executor(Box::new(Ex));
        let out = r.rx.on_message(
            Message::Command {
                id: 8,
                cmd: RemoteCommand::GetMetrics,
            },
            r.t0,
        );
        assert!(matches!(&out[..], [Message::CommandResult { id: 8, result }] if result.ok));
        let out = r.rx.on_message(
            Message::Command {
                id: 9,
                cmd: RemoteCommand::TypeText { text: "oi".into() },
            },
            r.t0,
        );
        assert!(matches!(&out[..], [Message::CommandResult { id: 9, result }] if result.ok));
    }
}
