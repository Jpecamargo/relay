//! dono: R1
//! Sentido inverso no agente (D24, D25): o mouse e o teclado físicos do PC controlam o Mac.
//!
//! Uma thread própria (`relay-reverse`) é dona do `Controller` do core em modo reverso, da captura
//! (hooks LL) e do cursor, como a thread `relay-core` do app Mac: o Controller exige `handle`,
//! `on_message` e `tick` na mesma thread. O pump do agente só conversa com ela por filas:
//! mensagens do Mac entram por `ReverseHandle::send`, e as do Controller saem por um canal tokio.
//!
//! Arbitragem: só um lado controla por vez, e quem mexeu por último manda. Enquanto o `Receiver`
//! do agente está ativo (o Mac controla o PC) o pump chama `set_blocked(true)` e o Controller fica
//! pausado; input físico do PC nesse estado manda `TakeControl` e o Mac devolve o foco. Do mesmo
//! jeito, `TakeControl` do Mac (input físico lá) faz o Controller daqui devolver. Se as duas pontas
//! entrarem ao mesmo tempo o Mac vence: ele devolve `FocusReturn` ao `FocusEnter` do PC.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver as StdReceiver, Sender as StdSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use relay_core::engine::{Controller, EventHandler, LocalActivity};
use relay_core::net::FastSender;
use relay_core::platform::{
    CaptureError, CursorControl, InjectError, InputCapture, InputInjector, KeyRepeat, LoopWaker,
};
use relay_core::types::{CapturedEvent, Config, DisplayInfo, InputEvent, LayoutProfile, Message};
use rtrb::{Producer, RingBuffer};
use tokio::sync::mpsc::UnboundedSender;

use super::routed::BoxCursor;

const CAPTURE_RING: usize = 4096;
const CONTROL_RING: usize = 1024;
/// Tick do Controller (fecha pinça parada, relógio do dwell).
const TICK: Duration = Duration::from_millis(5);
const IDLE_LOCAL: Duration = Duration::from_millis(2);
const IDLE_REMOTE: Duration = Duration::from_micros(500);
const BUSY: Duration = Duration::from_micros(150);

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
    UserPaused(bool),
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
            fast,
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

    pub fn set_paused(&self, paused: bool) {
        self.post(Cmd::UserPaused(paused));
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
    fast: FastSender,
    out: UnboundedSender<Message>,
    events: Option<EventHandler>,
    rx: StdReceiver<Cmd>,
    blocked: Arc<AtomicBool>,
    remote: Arc<AtomicBool>,
}

impl Worker {
    fn run(self) {
        let Worker {
            waker,
            config,
            local,
            profile,
            deps,
            fast,
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
        let mut ctl: Ctl = Controller::new(
            &config,
            profile,
            local,
            capture.clone(),
            BoxCursor(deps.cursor),
            BoxInjector(deps.injector),
            fast,
            control,
        );
        ctl.set_reverse(config.keymap.reverse_modifiers.clone());
        // Ao entrar no Mac o PC perde o foco: ClipboardSwitch e "enviar agora" vão à mídia.
        if let Some(h) = events {
            ctl.set_event_handler(h);
        }
        // Sem Mac conectado não há destino: começa pausado.
        let (mut link, mut user_paused) = (false, false);
        ctl.set_paused(true, Instant::now());
        let mut last_tick = Instant::now();
        let mut activity = LocalActivity::default();
        loop {
            let mut active = false;
            let mut n = 0;
            let mac_controls = blocked.load(Ordering::Acquire);
            if !mac_controls {
                activity.reset();
            }
            while n < 512 {
                let Ok(ev) = cap_rx.pop() else { break };
                n += 1;
                if mac_controls && link && activity.on_event(&ev, Instant::now()) {
                    tracing::info!("foco: input físico no PC, TakeControl enviado ao Mac");
                    let _ = out.send(Message::TakeControl);
                }
                ctl.handle(ev);
            }
            active |= n > 0;
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => {
                        ctl.set_paused(true, Instant::now());
                        flush(&mut ctl_out, &out);
                        remote.store(false, Ordering::Release);
                        capture.stop();
                        return;
                    }
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
        Cmd::Shutdown => {}
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
