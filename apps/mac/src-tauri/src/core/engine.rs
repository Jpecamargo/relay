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
use relay_core::net::FastSender;
use relay_core::platform::{CaptureError, DisplaySource, InputCapture, LoopWaker};
use relay_core::types::{
    AudioState, CapturedEvent, CommandResult, Config, DisplayInfo, DisplaysPayload, Host,
    HotkeyAction, InputEvent, KeyCode, LayoutProfile, Message, MouseButton, PermissionKind,
    PermissionState, PermissionsStatus, RemoteCommand, error_code,
};
use relay_core::version::peer_is_symmetric;
use rtrb::{Consumer, RingBuffer};
use tokio::sync::mpsc::UnboundedSender;

use super::Inner;
use super::net::NetCmd;
use super::platform::{Platform, SessionCtx, Shared};
use super::reverse::MacReverse;

/// Comandos para a thread do core.
pub(crate) enum CoreCmd {
    /// Sessão nova: reconstrói o Controller com o canal rápido dela.
    SessionUp {
        fast: FastSender,
        out: UnboundedSender<Message>,
    },
    SessionDown,
    /// A sessão está de pé: a mídia recebe o contexto dela (na ordem da fila, antes de `Link`).
    MediaUp(Box<SessionCtx>),
    /// A sessão ficou `Connected` (ou deixou de estar).
    Link(bool),
    Peer(Message),
    /// Movimento/rolagem do PC pelo canal rápido (sentido inverso).
    Fast(InputEvent),
    SetLocked(bool),
    SetPaused(bool),
    /// Pulo para a tela de número `n` (D19), pelo menu do painel.
    JumpTo(u8),
    ApplyConfig(Box<Config>),
    ProfilesChanged,
    /// Mensagem para o peer, vinda de fora da thread do core (efeitos do livro de layouts).
    Send(Message),
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
/// Keycode virtual do V e bits de CGEventFlags (Cmd; Control e Option não podem estar juntos).
const KEY_V: u16 = 9;
const FLAG_CMD: u64 = 0x10_0000;
const FLAG_CTRL_OPT: u64 = 0x4_0000 | 0x8_0000;

pub(crate) fn spawn<P: Platform>(
    inner: Arc<Inner<P>>,
    rx: Receiver<CoreCmd>,
    displays: P::Displays,
) -> JoinHandle<()> {
    let capture = Shared::new(inner.platform.new_capture());
    let cursor = Shared::new(inner.platform.new_cursor());
    let injector = Shared::new(inner.platform.new_injector());
    let (ev_tx, ev_rx) = mpsc::channel();
    let mut rev = {
        let st = inner.st();
        MacReverse::<P>::new(
            &st.config,
            st.local.clone(),
            injector.clone(),
            cursor.clone(),
            st.profile.clone(),
        )
    };
    let switch_tx = ev_tx.clone();
    rev.set_switch_handler(move || {
        let _ = switch_tx.send(EngineEvent::ClipboardSwitch);
    });
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
        left_down: false,
        rev,
        rev_active: false,
        layout_ready: false,
        early_book: None,
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
    /// Botão esquerdo do Mac pressionado (a captura entrega o evento mesmo com o foco remoto).
    left_down: bool,
    /// Sentido inverso: o PC controla o Mac (R1).
    rev: MacReverse<P>,
    rev_active: bool,
    /// A troca inicial de livros desta conexão já aconteceu (ou o peer é 0.2): só então o
    /// conjunto de telas pode criar perfil.
    layout_ready: bool,
    /// Livro do peer que chegou antes de a sessão ser dada como de pé (corrida com `Link`).
    early_book: Option<relay_core::types::LayoutBook>,
}

impl<P: Platform> Core<P> {
    fn run(mut self) {
        self.inner.waker.bind_current();
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
                if let CapturedEvent::Button {
                    button: MouseButton::Left,
                    down,
                } = ev
                {
                    self.left_down = down;
                }
                let remote_before = self.ctl.as_ref().is_some_and(|c| c.is_remote());
                let mut crossing = false;
                if let Some(c) = self.ctl.as_mut() {
                    c.handle(ev);
                    crossing = c.take_crossing_request();
                }
                if crossing && self.rev_active && self.link && !self.user_paused {
                    tracing::info!("foco: mouse do Mac atravessou para o PC, TakeControl enviado");
                    self.send(Message::TakeControl);
                }
                self.media_hooks_for(&ev, remote_before);
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

            // A pausa do Controller segue o Receiver reverso por qualquer caminho que o ativou ou
            // desativou, não só pelas mensagens tratadas em `on_peer`.
            if self.rev.is_active() != self.rev_active {
                self.sync_pause();
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
                self.rev.tick(now);
                self.drain_events(false);
            }
            if now.duration_since(last_poll) >= POLL {
                last_poll = now;
                self.flush_results(now);
            }

            let remote = self.ctl.as_ref().is_some_and(|c| c.is_remote());
            // Dorme até o relógio do laço vencer ou até a captura/um comando acordar a thread.
            LoopWaker::park(match (active, remote) {
                (true, _) => BUSY,
                (false, true) => IDLE_REMOTE,
                (false, false) => IDLE_LOCAL,
            });
        }
    }

    /// Ganhos da mídia sobre a captura: soltar o botão (arraste) e Cmd+V (colar arquivos).
    fn media_hooks_for(&mut self, ev: &CapturedEvent, remote_before: bool) {
        match *ev {
            CapturedEvent::Button {
                button: MouseButton::Left,
                down: false,
            } => self.inner.media.left_button_up(),
            CapturedEvent::Key {
                code: KeyCode::Mac(KEY_V),
                down: true,
                flags,
                ..
            } if !remote_before && flags & FLAG_CMD != 0 && flags & FLAG_CTRL_OPT == 0 => {
                self.inner.media.paste_requested();
            }
            _ => {}
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
        self.capture.set_waker(self.inner.waker.clone());
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
                local: st.local.clone(),
                peer: st.remote.clone(),
            }
        };
        self.inner.events.displays(&payload);
    }

    /// Perfil em uso (o ativo do livro, do host local ou do peer, adaptado às telas conectadas).
    /// Combinação nova de telas só cria perfil no host (`layout_displays_changed`).
    fn refresh_profile(&mut self) {
        let profile = {
            let mut st = self.inner.st();
            let combined: Vec<DisplayInfo> = st.local.iter().chain(&st.remote).cloned().collect();
            let book = st.layout.book();
            let Some(profile) =
                layout::select_profile(&book.profiles, book.active.as_deref(), &combined)
            else {
                st.profile = None;
                return;
            };
            st.profile = Some(profile.clone());
            profile
        };
        self.rev.set_profile(profile.clone());
        if let Some(c) = self.ctl.as_mut() {
            c.set_profile(profile);
        }
    }

    fn send(&self, msg: Message) {
        if let Some(o) = &self.out {
            let _ = o.send(msg);
        }
    }

    /// `LayoutSync` do perfil em uso, só para peer 0.2: o 0.3 recebe o livro (`LayoutBookSync`).
    fn send_layout(&self) {
        if !self.link || self.peer_symmetric() {
            return;
        }
        let profile = self.inner.st().profile.clone();
        if let Some(p) = profile {
            self.send(Message::LayoutSync(p));
        }
    }

    fn on_book_sync(&mut self, book: relay_core::types::LayoutBook) {
        let first = !self.layout_ready;
        let effects = self.inner.mutate(|s| s.layout.on_book_sync(book));
        self.inner.apply_layout(effects);
        if first {
            self.layout_exchanged();
        }
    }

    fn peer_symmetric(&self) -> bool {
        peer_is_symmetric(self.inner.st().peer_version.as_deref())
    }

    /// O conjunto de telas (local + peer) mudou: o host cria o perfil se faltar (regra do I4).
    fn layout_displays_changed(&mut self) {
        if !self.link || !self.layout_ready {
            return;
        }
        let effects = self.inner.mutate(|s| {
            if s.remote.is_empty() {
                return Vec::new();
            }
            let combined: Vec<DisplayInfo> = s.local.iter().chain(&s.remote).cloned().collect();
            s.layout.on_displays(&combined)
        });
        self.inner.apply_layout(effects);
    }

    /// A troca de livros desta conexão terminou: aplica a escolha de host do pareamento novo
    /// (D44) e confere as telas.
    fn layout_exchanged(&mut self) {
        self.layout_ready = true;
        let pending = self.inner.mutate(|s| s.pending_layout_host.take());
        if let Some(host) = pending {
            let effects = self.inner.mutate(|s| s.layout.on_new_pairing(host));
            self.inner.apply_layout(effects);
        }
        self.layout_displays_changed();
    }

    /// Sessão `Connected`: o livro vai ao peer 0.3 (e espera o dele); o 0.2 recebe `LayoutSync`.
    fn layout_link_up(&mut self) {
        let symmetric = self.peer_symmetric();
        let effects = self.inner.mutate(|s| s.layout.peer_connected(symmetric));
        self.inner.apply_layout(effects);
        if symmetric {
            if let Some(book) = self.early_book.take() {
                self.on_book_sync(book);
            }
            return;
        }
        // Peer 0.2 sempre usa o Mac como host: a escolha do pareamento não se aplica (D43).
        self.inner.mutate(|s| s.pending_layout_host = None);
        self.layout_ready = true;
        self.send_layout();
        self.layout_displays_changed();
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
        self.rev.set_local_displays(local.clone());
        self.refresh_profile();
        if self.link {
            self.send(Message::Displays(local));
            self.send_layout();
        }
        self.publish_displays();
        self.layout_displays_changed();
    }

    // ---- sessão e Controller ----

    fn build_controller(&mut self, fast: FastSender) {
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
        self.inner.mutate(|s| s.status.focus = Host::Local);
    }

    /// Pausa efetiva = pausa do usuário, sem PC conectado (nunca engolir input sem destino) ou o
    /// PC controlando o Mac (D24: só um lado controla por vez).
    fn sync_pause(&mut self) {
        let was_active = self.rev_active;
        self.rev_active = self.rev.is_active();
        if was_active && !self.rev_active {
            if let Some(c) = self.ctl.as_mut() {
                c.rearm_edge();
            }
        }
        let effective = self.user_paused || !self.link || self.rev_active;
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
                EngineEvent::FocusChanged { remote } => {
                    self.inner.mutate(|s| {
                        s.status.focus = if remote { Host::Peer } else { Host::Local };
                    });
                    self.inner.media.focus_changed(remote, self.left_down);
                }
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
        if matches!(cmd, RemoteCommand::CaptureScreen { .. }) {
            self.inner.media.capture_screen_requested();
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
                self.layout_ready = false;
                self.early_book = None;
                self.inner.mutate(|s| s.layout.peer_disconnected());
                self.rev.focus_lost();
                self.teardown_controller();
                self.out = None;
                self.fail_waiters();
                self.inner.media.link_changed(false);
            }
            CoreCmd::MediaUp(ctx) => self.inner.media.session_up(*ctx),
            CoreCmd::Link(up) => {
                if self.link == up {
                    return;
                }
                self.link = up;
                if !up {
                    self.rev.focus_lost();
                }
                self.sync_pause();
                if up {
                    self.send(Message::Displays(self.local_displays()));
                    self.layout_link_up();
                } else {
                    self.layout_ready = false;
                    self.early_book = None;
                    self.inner.mutate(|s| s.layout.peer_disconnected());
                    self.fail_waiters();
                }
                self.inner.media.link_changed(up);
            }
            CoreCmd::Peer(msg) => self.on_peer(msg, now),
            CoreCmd::Fast(ev) => {
                let back = self.rev.on_input(ev, now);
                for m in back {
                    self.send(m);
                }
            }
            CoreCmd::SetLocked(l) => {
                self.user_locked = l;
                if let Some(c) = self.ctl.as_mut() {
                    c.set_locked(l);
                }
                self.drain_events(false);
            }
            CoreCmd::JumpTo(n) => {
                if let Some(c) = self.ctl.as_mut() {
                    c.jump_to(n, now);
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
                self.rev.set_clipboard_config(cfg.clipboard.clone());
                self.refresh_profile();
                self.send_layout();
            }
            CoreCmd::ProfilesChanged => {
                self.refresh_profile();
                self.send_layout();
            }
            CoreCmd::Send(m) => self.send(m),
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
                self.rev
                    .on_message(Message::Displays(d.clone()), false, now);
                if let Some(c) = self.ctl.as_mut() {
                    c.on_message(Message::Displays(d.clone()), now);
                }
                self.inner.mutate(|s| s.remote = d);
                self.refresh_profile();
                self.send_layout();
                self.publish_displays();
                self.layout_displays_changed();
            }
            Message::LayoutBookSync(book) if !self.link => self.early_book = Some(book),
            Message::LayoutBookSync(book) => self.on_book_sync(book),
            Message::LayoutBookEdit(book) => {
                let effects = self.inner.mutate(|s| s.layout.on_book_edit(book));
                self.inner.apply_layout(effects);
            }
            // O Mac não executa comandos remotos na 0.3 (D46): responde para o PC não esperar.
            Message::Command { id, .. } => self.send(Message::CommandResult {
                id,
                result: CommandResult::err(id, error_code::UNSUPPORTED),
            }),
            // Sentido inverso: o PC pede o foco, manda input e devolve (R1). Com o PC no controle
            // o `FocusReturn` é dele (confirmação), senão é pedido de retorno ao Controller.
            m @ (Message::FocusEnter { .. } | Message::Input { .. }) => {
                let mac_controls = self.ctl.as_ref().is_some_and(|c| c.is_remote());
                for back in self.rev.on_message(m, mac_controls, now) {
                    self.send(back);
                }
                self.sync_pause();
                self.drain_events(true);
            }
            m @ Message::FocusReturn { .. } if self.rev.is_active() => {
                for back in self.rev.on_message(m, false, now) {
                    self.send(back);
                }
                self.sync_pause();
            }
            // Confirmação do PC a um FocusEnter que o Mac recusou: não é pedido de retorno.
            Message::FocusReturn { .. } if self.rev.take_refusal_ack(now) => {}
            m @ (Message::FocusReturn { .. }
            | Message::ForegroundApp { .. }
            | Message::TakeControl) => {
                if let Some(c) = self.ctl.as_mut() {
                    c.on_message(m, now);
                }
                self.drain_events(false);
            }
            // Id 0 nunca é do dispatcher: o agente avisa por ele que o áudio não ligou (WI3).
            Message::CommandResult { id: 0, result } => {
                self.inner.events.command_result(&result);
                self.inner
                    .media
                    .on_message(&Message::CommandResult { id: 0, result });
            }
            Message::CommandResult { id, result } => {
                let mut result = result;
                result.id = id;
                self.dispatcher.on_result(result);
                self.flush_results(now);
            }
            // Estado real do áudio no PC (pode ter recusado um dos sentidos).
            Message::AudioControl { mac_to_pc, .. } => {
                self.inner.mutate(|s| {
                    s.status.audio = AudioState {
                        mic_to_peer: mac_to_pc,
                        mic_from_peer: false,
                    }
                });
                self.inner.media.on_message(&Message::AudioControl {
                    pc_to_mac: false,
                    mac_to_pc,
                });
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
