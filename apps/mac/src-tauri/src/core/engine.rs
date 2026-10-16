//! dono: MI1
//! Thread `relay-core`: dona do `Controller` (C10). Serializa captura → `handle`, mensagens do
//! PC → `on_message` e `tick` (≤10 ms) numa thread só, roda o `CommandDispatcher` e esvazia a
//! fila de controle do Controller para a bomba da sessão (a `Session` não é `Sync`; quem a
//! possui é o gerente de rede).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use relay_core::engine::{CommandDispatcher, Controller, EngineEvent};
use relay_core::layout;
use relay_core::platform::{CaptureError, DisplaySource, InputCapture};
use relay_core::types::{
    CapturedEvent, CommandResult, Config, DisplayInfo, DisplaysPayload, HotkeyAction, InputEvent,
    LayoutProfile, Machine, Message, PermissionKind, PermissionState, PermissionsStatus,
    RemoteCommand,
};
use rtrb::{Consumer, Producer, RingBuffer};
use tokio::sync::mpsc::UnboundedSender;

use super::Inner;
use super::net::NetCmd;
use super::platform::{Platform, Shared};

/// Comandos para a thread do core.
pub(crate) enum CoreCmd {
    /// Sessão nova: reconstrói o Controller com o canal rápido dela.
    SessionUp {
        fast: Producer<InputEvent>,
        out: UnboundedSender<Message>,
    },
    SessionDown,
    /// A sessão ficou `Connected` (ou deixou de estar).
    Link(bool),
    Peer(Message),
    SetLocked(bool),
    SetPaused(bool),
    ApplyConfig(Box<Config>),
    ProfilesChanged,
    DisplaysChanged,
    Command {
        cmd: RemoteCommand,
        reply: Option<Sender<CommandResult>>,
    },
    Shutdown,
}

type Ctl<P> = Controller<
    Shared<<P as Platform>::Capture>,
    Shared<<P as Platform>::Cursor>,
    Shared<<P as Platform>::Injector>,
>;

/// Tick do Controller (fecha pinça parada) e poll do dispatcher.
const TICK: Duration = Duration::from_millis(5);
const POLL: Duration = Duration::from_millis(50);
const CAPTURE_RING: usize = 4096;
const CONTROL_RING: usize = 1024;
/// Sono do laço sem atividade: foco local tolera mais; remoto precisa de resposta rápida.
const IDLE_LOCAL: Duration = Duration::from_millis(4);
const IDLE_REMOTE: Duration = Duration::from_micros(500);
const BUSY: Duration = Duration::from_micros(150);

pub(crate) fn spawn<P: Platform>(
    inner: Arc<Inner<P>>,
    rx: Receiver<CoreCmd>,
    displays: P::Displays,
) -> JoinHandle<()> {
    let capture = Shared::new(inner.platform.new_capture());
    let cursor = Shared::new(inner.platform.new_cursor());
    let injector = Shared::new(inner.platform.new_injector());
    let (ev_tx, ev_rx) = mpsc::channel();
    let (_, cap_rx) = RingBuffer::new(1);
    let (_, ctl_out) = RingBuffer::new(1);
    let core = Core::<P> {
        inner,
        rx,
        capture,
        cursor,
        injector,
        displays,
        cap_rx,
        cap_ok: false,
        next_cap_try: Instant::now(),
        opened_settings: false,
        last_perm: None,
        ctl: None,
        ctl_out,
        out: None,
        link: false,
        ev_tx,
        ev_rx,
        dispatcher: CommandDispatcher::default(),
        waiters: HashMap::new(),
        user_locked: false,
        user_paused: false,
    };
    std::thread::Builder::new()
        .name("relay-core".into())
        .spawn(move || core.run())
        .expect("thread relay-core")
}

struct Core<P: Platform> {
    inner: Arc<Inner<P>>,
    rx: Receiver<CoreCmd>,
    capture: Shared<P::Capture>,
    cursor: Shared<P::Cursor>,
    injector: Shared<P::Injector>,
    displays: P::Displays,
    cap_rx: Consumer<CapturedEvent>,
    cap_ok: bool,
    next_cap_try: Instant,
    opened_settings: bool,
    last_perm: Option<PermissionsStatus>,
    ctl: Option<Ctl<P>>,
    ctl_out: Consumer<Message>,
    out: Option<UnboundedSender<Message>>,
    link: bool,
    ev_tx: Sender<EngineEvent>,
    ev_rx: Receiver<EngineEvent>,
    dispatcher: CommandDispatcher,
    waiters: HashMap<u32, Sender<CommandResult>>,
    user_locked: bool,
    user_paused: bool,
}

impl<P: Platform> Core<P> {
    fn run(mut self) {
        self.publish_displays();
        let mut last_tick = Instant::now();
        let mut last_poll = last_tick;
        loop {
            let mut active = false;
            let now = Instant::now();
            self.ensure_capture(now);

            // Captura → Controller (ou descarte, sem sessão).
            let mut n = 0;
            while n < 512 {
                let Ok(ev) = self.cap_rx.pop() else { break };
                n += 1;
                if let Some(c) = self.ctl.as_mut() {
                    c.handle(ev);
                }
            }
            if n > 0 {
                active = true;
                self.drain_events(false);
            }

            // Comandos (UI, rede, sistema).
            loop {
                match self.rx.try_recv() {
                    Ok(CoreCmd::Shutdown) | Err(TryRecvError::Disconnected) => {
                        self.stop();
                        return;
                    }
                    Ok(cmd) => {
                        active = true;
                        self.on_cmd(cmd);
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            // Fila de controle do Controller → bomba da sessão.
            while let Ok(m) = self.ctl_out.pop() {
                active = true;
                if let Some(o) = &self.out {
                    let _ = o.send(m);
                }
            }

            let now = Instant::now();
            if now.duration_since(last_tick) >= TICK {
                last_tick = now;
                if let Some(c) = self.ctl.as_mut() {
                    c.tick(now);
                }
                self.drain_events(false);
            }
            if now.duration_since(last_poll) >= POLL {
                last_poll = now;
                self.flush_results(now);
            }

            let remote = self.ctl.as_ref().is_some_and(|c| c.is_remote());
            std::thread::sleep(match (active, remote) {
                (true, _) => BUSY,
                (false, true) => IDLE_REMOTE,
                (false, false) => IDLE_LOCAL,
            });
        }
    }

    fn stop(&mut self) {
        self.teardown_controller();
        self.capture.stop();
    }

    // ---- captura e permissões ----

    fn ensure_capture(&mut self, now: Instant) {
        if self.cap_ok || now < self.next_cap_try {
            return;
        }
        let (prod, cons) = RingBuffer::new(CAPTURE_RING);
        match self.capture.start(prod) {
            Ok(()) | Err(CaptureError::AlreadyRunning) => {
                self.cap_ok = true;
                self.cap_rx = cons;
                self.publish_permissions();
            }
            Err(CaptureError::PermissionDenied) => {
                let perms = self.publish_permissions();
                if !self.opened_settings {
                    self.opened_settings = true;
                    self.inner
                        .platform
                        .open_permission_settings(first_missing(&perms));
                }
                self.next_cap_try = now + Duration::from_secs(2);
            }
            Err(e) => {
                tracing::warn!("captura não iniciou: {e}");
                self.next_cap_try = now + Duration::from_secs(5);
            }
        }
    }

    /// Lê as permissões e emite `relay:permissions` quando mudam (e na primeira leitura).
    fn publish_permissions(&mut self) -> PermissionsStatus {
        let perms = self.inner.platform.check_permissions();
        if self.last_perm != Some(perms) {
            self.last_perm = Some(perms);
            self.inner.events.permissions(&perms);
        }
        perms
    }

    // ---- telas e perfil ----

    fn local_displays(&self) -> Vec<DisplayInfo> {
        self.inner.st().local.clone()
    }

    fn publish_displays(&self) {
        let payload = {
            let st = self.inner.st();
            DisplaysPayload {
                mac: st.local.clone(),
                windows: st.remote.clone(),
            }
        };
        self.inner.events.displays(&payload);
    }

    /// Perfil para as telas conectadas (persistido em `config.layout.profiles`). Combinação nova
    /// com as duas máquinas presentes vira perfil padrão e é gravada, para o editor listá-la.
    fn refresh_profile(&mut self) {
        let (profile, new_cfg) = {
            let mut st = self.inner.st();
            let combined: Vec<DisplayInfo> = st.local.iter().chain(&st.remote).cloned().collect();
            let Some(profile) = layout::select_profile(&st.config.layout.profiles, &combined)
            else {
                st.profile = None;
                return;
            };
            let known = st
                .config
                .layout
                .profiles
                .iter()
                .any(|p| p.key == profile.key);
            let new_cfg = if !known && !st.remote.is_empty() {
                st.config.layout.profiles.push(profile.clone());
                Some(st.config.clone())
            } else {
                None
            };
            st.profile = Some(profile.clone());
            (profile, new_cfg)
        };
        if let Some(cfg) = new_cfg {
            self.inner.persist_config(&cfg);
        }
        if let Some(c) = self.ctl.as_mut() {
            c.set_profile(profile);
        }
    }

    fn send(&self, msg: Message) {
        if let Some(o) = &self.out {
            let _ = o.send(msg);
        }
    }

    fn send_layout(&self) {
        if !self.link {
            return;
        }
        let profile = self.inner.st().profile.clone();
        if let Some(p) = profile {
            self.send(Message::LayoutSync(p));
        }
    }

    fn on_displays_changed(&mut self) {
        match self.displays.list() {
            Ok(list) => self.inner.mutate(|s| s.local = list),
            Err(e) => {
                tracing::warn!("não consegui listar as telas: {e}");
                return;
            }
        }
        let local = self.local_displays();
        if let Some(c) = self.ctl.as_mut() {
            c.set_local_displays(local.clone());
        }
        self.refresh_profile();
        if self.link {
            self.send(Message::Displays(local));
            self.send_layout();
        }
        self.publish_displays();
    }

    // ---- sessão e Controller ----

    fn build_controller(&mut self, fast: Producer<InputEvent>) {
        self.teardown_controller();
        let (control, ctl_out) = RingBuffer::new(CONTROL_RING);
        self.ctl_out = ctl_out;
        let (config, profile) = {
            let st = self.inner.st();
            (st.config.clone(), st.profile.clone())
        };
        let profile = profile.unwrap_or_else(|| LayoutProfile {
            key: Default::default(),
            name: String::new(),
            displays: Vec::new(),
            windows_detached: false,
        });
        let mut c = Controller::new(
            &config,
            profile,
            self.local_displays(),
            self.capture.clone(),
            self.cursor.clone(),
            self.injector.clone(),
            fast,
            control,
        );
        let tx = self.ev_tx.clone();
        c.set_event_handler(Box::new(move |e| {
            let _ = tx.send(e);
        }));
        c.set_locked(self.user_locked);
        self.ctl = Some(c);
        self.sync_pause();
        self.drain_events(true);
    }

    /// Devolve o foco ao Mac (se estava no PC) e descarta o Controller.
    fn teardown_controller(&mut self) {
        if let Some(c) = self.ctl.as_mut()
            && c.is_remote()
        {
            c.set_paused(true, Instant::now());
        }
        self.drain_events(true);
        self.ctl = None;
        self.inner.mutate(|s| s.status.focus = Machine::Mac);
    }

    /// Pausa efetiva = pausa do usuário ou sem PC conectado (nunca engolir input sem destino).
    fn sync_pause(&mut self) {
        let effective = self.user_paused || !self.link;
        if let Some(c) = self.ctl.as_mut()
            && c.is_paused() != effective
        {
            c.set_paused(effective, Instant::now());
            self.drain_events(true);
        }
    }

    // ---- eventos do motor ----

    fn drain_events(&mut self, ignore_pause: bool) {
        while let Ok(e) = self.ev_rx.try_recv() {
            match e {
                EngineEvent::FocusChanged { remote } => self.inner.mutate(|s| {
                    s.status.focus = if remote {
                        Machine::Windows
                    } else {
                        Machine::Mac
                    };
                }),
                EngineEvent::ClipboardSwitch => self.inner.media.clipboard_switch(),
                EngineEvent::LockChanged(l) => {
                    self.user_locked = l;
                    self.inner.mutate(|s| s.status.locked = l);
                }
                EngineEvent::PauseChanged(p) => {
                    if !ignore_pause {
                        self.user_paused = p;
                        self.inner.mutate(|s| s.status.paused = p);
                        self.sync_pause();
                    }
                }
                EngineEvent::Hotkey(action) => self.on_hotkey(action),
            }
        }
    }

    fn on_hotkey(&mut self, action: HotkeyAction) {
        match action {
            HotkeyAction::Command(cmd) => {
                if !self.send_command(cmd, None) {
                    self.inner
                        .events
                        .command_result(&CommandResult::err(0, "not_connected"));
                }
            }
            HotkeyAction::ClipboardSendNow => self.inner.media.clipboard_send_now(),
            HotkeyAction::WakePc => {
                if let Err(e) = self.inner.wake_pc() {
                    tracing::warn!("Wake-on-LAN: {e}");
                }
            }
            // Pulo, trava e pausa o Controller resolve sozinho.
            HotkeyAction::JumpTo(_) | HotkeyAction::ToggleLock | HotkeyAction::TogglePause => {}
        }
    }

    // ---- comandos remotos ----

    fn send_command(&mut self, cmd: RemoteCommand, reply: Option<Sender<CommandResult>>) -> bool {
        if !self.link {
            return false;
        }
        let msg = self.dispatcher.command_message(cmd, Instant::now());
        if let (Message::Command { id, .. }, Some(r)) = (&msg, reply) {
            self.waiters.insert(*id, r);
        }
        self.send(msg);
        true
    }

    fn flush_results(&mut self, now: Instant) {
        for r in self.dispatcher.poll(now) {
            match self.waiters.remove(&r.id) {
                Some(w) => {
                    let _ = w.send(r);
                }
                None => self.inner.events.command_result(&r),
            }
        }
    }

    // ---- comandos ----

    fn on_cmd(&mut self, cmd: CoreCmd) {
        let now = Instant::now();
        match cmd {
            CoreCmd::SessionUp { fast, out } => {
                self.out = Some(out);
                self.link = false;
                self.build_controller(fast);
            }
            CoreCmd::SessionDown => {
                self.link = false;
                self.teardown_controller();
                self.out = None;
                self.fail_waiters();
                self.inner.media.link_changed(false);
            }
            CoreCmd::Link(up) => {
                if self.link == up {
                    return;
                }
                self.link = up;
                self.sync_pause();
                if up {
                    self.send(Message::Displays(self.local_displays()));
                } else {
                    self.fail_waiters();
                }
                self.inner.media.link_changed(up);
            }
            CoreCmd::Peer(msg) => self.on_peer(msg, now),
            CoreCmd::SetLocked(l) => {
                self.user_locked = l;
                if let Some(c) = self.ctl.as_mut() {
                    c.set_locked(l);
                }
                self.drain_events(false);
            }
            CoreCmd::SetPaused(p) => {
                self.user_paused = p;
                self.sync_pause();
            }
            CoreCmd::ApplyConfig(cfg) => {
                if let Some(c) = self.ctl.as_mut() {
                    c.apply_config(&cfg);
                }
                self.refresh_profile();
                self.send_layout();
            }
            CoreCmd::ProfilesChanged => {
                self.refresh_profile();
                self.send_layout();
            }
            CoreCmd::DisplaysChanged => self.on_displays_changed(),
            CoreCmd::Command { cmd, reply } => {
                let fail = reply.clone();
                if !self.send_command(cmd, reply)
                    && let Some(r) = fail
                {
                    let _ = r.send(CommandResult::err(0, "not_connected"));
                }
            }
            CoreCmd::Shutdown => {}
        }
    }

    fn fail_waiters(&mut self) {
        for (id, w) in self.waiters.drain() {
            let _ = w.send(CommandResult::err(id, "not_connected"));
        }
    }

    fn on_peer(&mut self, msg: Message, now: Instant) {
        match msg {
            Message::Displays(d) => {
                if let Some(c) = self.ctl.as_mut() {
                    c.on_message(Message::Displays(d.clone()), now);
                }
                self.inner.mutate(|s| s.remote = d);
                self.refresh_profile();
                self.send_layout();
                self.publish_displays();
            }
            m @ (Message::FocusReturn { .. } | Message::ForegroundApp { .. }) => {
                if let Some(c) = self.ctl.as_mut() {
                    c.on_message(m, now);
                }
                self.drain_events(false);
            }
            Message::CommandResult { id, result } => {
                let mut result = result;
                result.id = id;
                self.dispatcher.on_result(result);
                self.flush_results(now);
            }
            Message::Unpair => {
                self.inner.forget_peer();
                self.inner.net(NetCmd::StopSession { notify: false });
            }
            other => self.inner.media.on_message(&other),
        }
    }
}

/// Qual ajuste abrir primeiro: o que não está concedido (acessibilidade antes).
fn first_missing(p: &PermissionsStatus) -> PermissionKind {
    if p.accessibility != PermissionState::Granted {
        PermissionKind::Accessibility
    } else {
        PermissionKind::InputMonitoring
    }
}
