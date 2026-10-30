//! dono: R1
//! Sentido inverso no agente (D24, D25): o mouse e o teclado físicos do PC controlam o Mac.
//!
//! Uma thread própria (`relay-reverse`) é dona do `Controller` do core em modo reverso, da captura
//! (hooks LL) e do cursor, como a thread `relay-core` do app Mac: o Controller exige `handle`,
//! `on_message` e `tick` na mesma thread. O pump do agente só conversa com ela por filas:
//! mensagens do Mac entram por `ReverseHandle::send`, e as do Controller saem por um canal tokio.
//!
//! Arbitragem: só um lado controla por vez, e quem atravessa a borda por último manda. Enquanto o
//! `Receiver` do agente está ativo (o Mac controla o PC) o pump chama `set_blocked(true)` e o
//! Controller fica pausado; se o mouse do PC atravessa a borda para o Mac nesse estado, sai
//! `TakeControl` e o Mac devolve o foco. Mexer sem atravessar não pede nada: o mouse do PC funciona
//! no PC junto com o teclado do Mac. Do mesmo jeito, `TakeControl` do Mac faz o Controller daqui
//! devolver. Se as duas pontas
//! entrarem ao mesmo tempo o Mac vence: ele devolve `FocusReturn` ao `FocusEnter` do PC.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver as StdReceiver, Sender as StdSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use relay_core::engine::{Controller, EventHandler};
use relay_core::net::FastSender;
use relay_core::platform::{
    CaptureError, CursorControl, InjectError, InputCapture, InputInjector, KeyRepeat, LoopWaker,
};
use relay_core::types::{CapturedEvent, Config, DisplayInfo, InputEvent, LayoutProfile, Message};
use rtrb::{Consumer, Producer, RingBuffer};
use tokio::sync::mpsc::UnboundedSender;

use super::Side;
use super::routed::BoxCursor;

const CAPTURE_RING: usize = 4096;
const CONTROL_RING: usize = 1024;
/// Tick do Controller (fecha pinça parada, relógio do dwell).
const TICK: Duration = Duration::from_millis(5);
const IDLE_LOCAL: Duration = Duration::from_millis(2);
const IDLE_REMOTE: Duration = Duration::from_micros(500);
const BUSY: Duration = Duration::from_micros(150);

/// Fila entre o Controller (que é dono de um único `FastSender`) e as sessões possíveis.
const FAST_PROXY: usize = 4096;

/// Peças de plataforma do Controller reverso (hooks LL, cursor, injetor local).
pub struct ReverseDeps {
    pub capture: Box<dyn InputCapture>,
    pub cursor: Box<dyn CursorControl>,
    /// Solta no PC as teclas seguradas quando o foco sai dele.
    pub injector: Box<dyn InputInjector>,
}

impl ReverseDeps {
    /// Hooks LL, `ClipCursor` de 1 px e `SendInput` reais (só fazem efeito no Windows).
    pub fn real() -> Self {
        Self {
            capture: Box::new(relay_platform_windows::hooks::WindowsHooks::default()),
            cursor: Box::new(relay_platform_windows::cursor::WindowsCursor::default()),
            injector: Box::new(relay_platform_windows::inject::WindowsInjector::default()),
        }
    }
}

/// A captura é compartilhada com a thread para que ela possa pará-la ao encerrar.
#[derive(Clone)]
struct BoxCapture(Arc<Mutex<Box<dyn InputCapture>>>);

impl BoxCapture {
    fn lock(&self) -> MutexGuard<'_, Box<dyn InputCapture>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl InputCapture for BoxCapture {
    fn start(&mut self, sink: Producer<CapturedEvent>) -> Result<(), CaptureError> {
        self.lock().start(sink)
    }
    fn set_waker(&mut self, waker: LoopWaker) {
        self.lock().set_waker(waker);
    }
    fn stop(&mut self) {
        self.lock().stop();
    }
    fn set_swallow(&self, swallow: bool) {
        self.lock().set_swallow(swallow);
    }
    fn is_swallowing(&self) -> bool {
        self.lock().is_swallowing()
    }
}

struct BoxInjector(Box<dyn InputInjector>);

impl InputInjector for BoxInjector {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.0.inject(event)
    }
    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        self.0.type_text(text)
    }
    fn key_repeat(&self) -> KeyRepeat {
        self.0.key_repeat()
    }
}

enum Cmd {
    /// Mensagem do Mac para o Controller (`Displays`, `FocusReturn`, `TakeControl`).
    Message(Message),
    LocalDisplays(Vec<DisplayInfo>),
    Profile(LayoutProfile),
    Config(Box<Config>),
    Link(bool),
    /// Produtor do canal rápido da sessão deste lado (a do `SessionDialer` nasce depois).
    Fast(Side, FastSender),
    /// Qual sessão leva o movimento do mouse.
    UseSide(Side),
    UserPaused(bool),
    Locked(bool),
    JumpTo(u8),
    Shutdown,
}

/// Ponta do pump para a thread do Controller reverso.
pub struct ReverseHandle {
    tx: StdSender<Cmd>,
    /// Acorda o laço da thread a cada comando e a cada evento da captura (hooks).
    waker: LoopWaker,
    blocked: Arc<AtomicBool>,
    remote: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReverseHandle {
    /// Sobe a thread. `fast` é o produtor do canal rápido da sessão; `out` leva ao pump as
    /// mensagens de controle do Controller (`FocusEnter`, teclas, `FocusReturn`).
    pub fn spawn(
        config: &Config,
        local: Vec<DisplayInfo>,
        profile: Option<LayoutProfile>,
        deps: ReverseDeps,
        fast: FastSender,
        out: UnboundedSender<Message>,
        events: Option<EventHandler>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let blocked = Arc::new(AtomicBool::new(false));
        let remote = Arc::new(AtomicBool::new(false));
        let waker = LoopWaker::default();
        let worker = Worker {
            waker: waker.clone(),
            config: config.clone(),
            local,
            profile,
            deps,
            listener_fast: Some(fast),
            out,
            events,
            rx,
            blocked: blocked.clone(),
            remote: remote.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("relay-reverse".into())
            .spawn(move || worker.run())
            .map_err(|e| tracing::warn!(error = %e, "thread do sentido inverso não iniciou"))
            .ok();
        Self {
            tx,
            waker,
            blocked,
            remote,
            thread,
        }
    }

    fn post(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
        self.waker.wake();
    }

    /// O PC está controlando o Mac (o `FocusReturn` seguinte é do Controller, não do Receiver).
    pub fn is_remote(&self) -> bool {
        self.remote.load(Ordering::Acquire)
    }

    /// `true` enquanto o Mac controla o PC (D24): o Controller não pode assumir.
    pub fn set_blocked(&self, blocked: bool) {
        self.blocked.store(blocked, Ordering::Release);
    }

    pub fn send(&self, msg: Message) {
        self.post(Cmd::Message(msg));
    }

    pub fn set_local_displays(&self, displays: Vec<DisplayInfo>) {
        self.post(Cmd::LocalDisplays(displays));
    }

    pub fn set_profile(&self, profile: LayoutProfile) {
        self.post(Cmd::Profile(profile));
    }

    pub fn apply_config(&self, config: &Config) {
        self.post(Cmd::Config(Box::new(config.clone())));
    }

    /// Sessão `Connected` (ou não): sem Mac conectado o Controller não engole input.
    pub fn set_link(&self, up: bool) {
        self.post(Cmd::Link(up));
    }

    /// Entrega o produtor do canal rápido de `side` (o do listener já vai no `spawn`).
    pub fn set_fast(&self, side: Side, fast: FastSender) {
        self.post(Cmd::Fast(side, fast));
    }

    /// Aponta o movimento do mouse para a sessão em uso (D41: pode ser a que o par discou ou a
    /// que este PC discou).
    pub fn use_side(&self, side: Side) {
        self.post(Cmd::UseSide(side));
    }

    pub fn set_paused(&self, paused: bool) {
        self.post(Cmd::UserPaused(paused));
    }

    /// Trava o cursor no lado atual (`set_cursor_lock`).
    pub fn set_locked(&self, locked: bool) {
        self.post(Cmd::Locked(locked));
    }

    /// Pula o cursor para a tela de número `n` (1 = a primeira do layout).
    pub fn jump_to(&self, n: u8) {
        self.post(Cmd::JumpTo(n));
    }

    /// Para a thread (devolve o foco e solta a captura) e espera.
    pub fn shutdown(&mut self) {
        self.post(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ReverseHandle {
    fn drop(&mut self) {
        self.post(Cmd::Shutdown);
    }
}

type Ctl = Controller<BoxCapture, BoxCursor, BoxInjector>;

struct Worker {
    waker: LoopWaker,
    config: Config,
    local: Vec<DisplayInfo>,
    profile: Option<LayoutProfile>,
    deps: ReverseDeps,
    listener_fast: Option<FastSender>,
    out: UnboundedSender<Message>,
    events: Option<EventHandler>,
    rx: StdReceiver<Cmd>,
    blocked: Arc<AtomicBool>,
    remote: Arc<AtomicBool>,
}

/// Leva o que o Controller pôs na fila rápida para a sessão em uso. Sem sessão, descarta.
struct FastRouter {
    from: Consumer<InputEvent>,
    side: Side,
    listener: Option<FastSender>,
    dialer: Option<FastSender>,
}

impl FastRouter {
    fn route(&mut self) -> bool {
        let mut any = false;
        while let Ok(ev) = self.from.pop() {
            any = true;
            let to = match self.side {
                Side::Listener => self.listener.as_mut(),
                Side::Dialer => self.dialer.as_mut(),
            };
            if let Some(to) = to {
                let _ = to.push(ev);
            }
        }
        any
    }
}

impl Worker {
    fn run(self) {
        let Worker {
            waker,
            config,
            local,
            profile,
            deps,
            listener_fast,
            out,
            events,
            rx,
            blocked,
            remote,
        } = self;
        waker.bind_current();
        let (cap_tx, mut cap_rx) = RingBuffer::new(CAPTURE_RING);
        let mut capture = BoxCapture(Arc::new(Mutex::new(deps.capture)));
        capture.set_waker(waker.clone());
        if let Err(e) = capture.start(cap_tx) {
            // Sem hooks (permissão, ou fora do Windows) o PC simplesmente não controla o Mac.
            tracing::warn!("captura do sentido inverso indisponível: {e}");
            return;
        }
        let (control, mut ctl_out) = RingBuffer::new(CONTROL_RING);
        let profile = profile.unwrap_or_else(|| LayoutProfile {
            key: Default::default(),
            name: String::new(),
            displays: Vec::new(),
            windows_detached: false,
        });
        let (proxy, from) = RingBuffer::new(FAST_PROXY);
        let mut router = FastRouter {
            from,
            side: Side::Listener,
            listener: listener_fast,
            dialer: None,
        };
        let mut ctl: Ctl = Controller::new(
            &config,
            profile,
            local,
            capture.clone(),
            BoxCursor(deps.cursor),
            BoxInjector(deps.injector),
            proxy.into(),
            control,
        );
        ctl.set_reverse(config.keymap.reverse_modifiers.clone());
        ctl.set_external_toggles(true);
        // Ao entrar no Mac o PC perde o foco: ClipboardSwitch e "enviar agora" vão à mídia.
        if let Some(h) = events {
            ctl.set_event_handler(h);
        }
        // Sem Mac conectado não há destino: começa pausado.
        let (mut link, mut user_paused) = (false, false);
        ctl.set_paused(true, Instant::now());
        let mut last_tick = Instant::now();
        loop {
            let mut active = false;
            let mut n = 0;
            let mac_controls = blocked.load(Ordering::Acquire);
            while n < 512 {
                let Ok(ev) = cap_rx.pop() else { break };
                n += 1;
                ctl.handle(ev);
                if ctl.take_crossing_request() && mac_controls && link && !user_paused {
                    tracing::info!("foco: mouse do PC atravessou para o Mac, TakeControl enviado");
                    let _ = out.send(Message::TakeControl);
                }
            }
            active |= n > 0;
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => {
                        ctl.set_paused(true, Instant::now());
                        flush(&mut ctl_out, &out);
                        router.route();
                        remote.store(false, Ordering::Release);
                        capture.stop();
                        return;
                    }
                    Ok(Cmd::Fast(side, f)) => match side {
                        Side::Listener => router.listener = Some(f),
                        Side::Dialer => router.dialer = Some(f),
                    },
                    Ok(Cmd::UseSide(side)) => router.side = side,
                    Ok(cmd) => {
                        active = true;
                        apply(&mut ctl, &mut link, &mut user_paused, cmd);
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            let pause = user_paused || !link || blocked.load(Ordering::Acquire);
            if ctl.is_paused() != pause {
                ctl.set_paused(pause, Instant::now());
            }
            active |= flush(&mut ctl_out, &out);
            let now = Instant::now();
            if now.duration_since(last_tick) >= TICK {
                last_tick = now;
                ctl.tick(now);
            }
            active |= router.route();
            remote.store(ctl.is_remote(), Ordering::Release);
            // Dorme até o relógio do laço vencer ou até a captura/um comando acordar a thread.
            LoopWaker::park(match (active, ctl.is_remote()) {
                (true, _) => BUSY,
                (false, true) => IDLE_REMOTE,
                (false, false) => IDLE_LOCAL,
            });
        }
    }
}

fn apply(ctl: &mut Ctl, link: &mut bool, user_paused: &mut bool, cmd: Cmd) {
    match cmd {
        Cmd::Message(m) => ctl.on_message(m, Instant::now()),
        Cmd::LocalDisplays(d) => ctl.set_local_displays(d),
        Cmd::Profile(p) => ctl.set_profile(p),
        Cmd::Config(c) => ctl.apply_config(&c),
        Cmd::Link(up) => *link = up,
        Cmd::UserPaused(p) => *user_paused = p,
        Cmd::Locked(l) => ctl.set_locked(l),
        Cmd::JumpTo(n) => ctl.jump_to(n, Instant::now()),
        Cmd::Fast(..) | Cmd::UseSide(_) | Cmd::Shutdown => {}
    }
}

fn flush(ctl_out: &mut rtrb::Consumer<Message>, out: &UnboundedSender<Message>) -> bool {
    let mut any = false;
    while let Ok(m) = ctl_out.pop() {
        any = true;
        let _ = out.send(m);
    }
    any
}
