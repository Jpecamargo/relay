//! dono: WI1
//! O laço do agente: uma tarefa tokio dona da sessão, do `Receiver` e do guard do ponteiro.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_core::crypto::{forget_active_peer, remove_peer, save_active_peer};
use relay_core::discovery::Advertiser;
use relay_core::engine::Receiver;
use relay_core::net::{SessionEvent, SessionListener};
use relay_core::platform::{DisplaySource, KeyStore};
use relay_core::types::{
    ConnectionStatus, Machine, Message, PairedPeer, PairingEvent, PairingState, StatusSnapshot,
    VersionSync,
};
use relay_core::version::{APP_VERSION, version_sync};
use tokio::sync::{mpsc, watch};

use tokio::time::{Interval, MissedTickBehavior, interval};

use super::media::{MediaOut, MediaPort};
use super::reverse::{ReverseDeps, ReverseHandle};
use super::routed::{BoxCursor, RoutedInjector};
use super::{
    ACTIVE_PEER_ACCOUNT, AgentDeps, Cmd, ForegroundSource, MessageHook, Shared, UiEvents, now_ms,
};

/// Relógio do `Receiver` enquanto o Mac controla este lado (repetição de tecla ≤ 10 ms).
const TICK_ACTIVE: Duration = Duration::from_millis(5);
/// Relógio ocioso: só expira o código de pareamento.
const TICK_IDLE: Duration = Duration::from_millis(250);
/// `relay:latency` no máximo a 2 Hz.
const LATENCY_EVERY: Duration = Duration::from_millis(500);
/// Folga para o `Unpair` sair antes de derrubar a sessão.
const UNPAIR_FLUSH: Duration = Duration::from_millis(100);

fn ticker(period: Duration) -> Interval {
    let mut t = interval(period);
    t.set_missed_tick_behavior(MissedTickBehavior::Delay);
    t
}

pub(super) struct Pump {
    shared: Arc<Shared>,
    listener: SessionListener,
    rx: Receiver<RoutedInjector, BoxCursor>,
    keystore: Arc<dyn KeyStore>,
    events: Arc<dyn UiEvents>,
    displays: Box<dyn DisplaySource>,
    // Mantidos vivos até o fim: descartar para o anúncio mDNS e o observador de foreground.
    _advertiser: Option<Advertiser>,
    _foreground: Option<Box<dyn ForegroundSource>>,
    pointer_guard: Box<dyn Fn() -> Box<dyn Send> + Send + Sync>,
    guard: Option<Box<dyn Send>>,
    hook: Option<MessageHook>,
    media: Option<Box<dyn MediaPort>>,
    media_out: Option<mpsc::UnboundedReceiver<MediaOut>>,
    status_rx: watch::Receiver<ConnectionStatus>,
    peer: Option<PairedPeer>,
    foreground: Option<String>,
    paused: bool,
    pairing_until: Option<Instant>,
    last_latency: Option<Instant>,
    /// Sentido inverso (R1): Controller do PC sobre o Mac, em thread própria.
    rev: Option<ReverseHandle>,
    /// Telas do Mac (último `Displays` recebido): destino do `FocusReturn` ao pausar.
    mac_displays: Vec<relay_core::types::DisplayInfo>,
    /// Fila do executor de comandos, que roda fora do pump (CaptureScreen/GetMetrics demoram).
    cmd_worker: Option<std::sync::mpsc::Sender<(u32, relay_core::types::RemoteCommand)>>,
    cmd_out: Option<mpsc::UnboundedReceiver<Message>>,
    rev_out: Option<mpsc::UnboundedReceiver<Message>>,
}

/// Próxima mensagem do Controller reverso; sem sentido inverso nunca resolve.
async fn recv_rev(rx: &mut Option<mpsc::UnboundedReceiver<Message>>) -> Option<Message> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Executa os comandos remotos numa thread própria, em ordem, e devolve cada `CommandResult` ao
/// pump por um canal: um `CaptureScreen` lento não congela a injeção de input.
fn spawn_command_worker(
    mut ex: Box<dyn relay_core::platform::CommandExecutor>,
) -> (
    std::sync::mpsc::Sender<(u32, relay_core::types::RemoteCommand)>,
    mpsc::UnboundedReceiver<Message>,
) {
    let (tx, rx) = std::sync::mpsc::channel::<(u32, relay_core::types::RemoteCommand)>();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let spawned = std::thread::Builder::new()
        .name("relay-commands".into())
        .spawn(move || {
            while let Ok((id, cmd)) = rx.recv() {
                let mut result = ex.execute(id, &cmd);
                result.id = id;
                if out_tx.send(Message::CommandResult { id, result }).is_err() {
                    break;
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "thread de comandos não iniciou");
    }
    (tx, out_rx)
}

impl Pump {
    /// `FocusReturn` para o centro da tela do Mac, para devolver o foco ao pausar.
    fn focus_return(&self) -> Option<Message> {
        let d = self
            .mac_displays
            .iter()
            .find(|d| d.is_primary)
            .or_else(|| self.mac_displays.first())?;
        Some(Message::FocusReturn {
            display_id: d.id.clone(),
            point_px: (
                d.origin_px.0 + (d.size_px.0 / 2) as i32,
                d.origin_px.1 + (d.size_px.1 / 2) as i32,
            ),
        })
    }

    pub(super) fn new(
        mut deps: AgentDeps,
        shared: Arc<Shared>,
        listener: SessionListener,
        advertiser: Option<Advertiser>,
        events: Arc<dyn UiEvents>,
        peers: Vec<PairedPeer>,
        reverse: Option<ReverseDeps>,
    ) -> Self {
        let mut listener = listener;
        // O mesmo gancho de eventos (mídia/clipboard) serve o Receiver e o Controller reverso.
        let events_hook = deps.engine_events.take().map(|h| Arc::new(Mutex::new(h)));
        let forward = |hook: &Option<Arc<Mutex<relay_core::engine::EventHandler>>>| {
            hook.clone().map(|h| -> relay_core::engine::EventHandler {
                Box::new(move |e| (h.lock().unwrap_or_else(|p| p.into_inner()))(e))
            })
        };
        let local = deps.displays.list().unwrap_or_default();
        let (rev, rev_out) = match reverse.zip(listener.take_input_producer()) {
            Some((rdeps, fast)) => {
                let (tx, rx) = mpsc::unbounded_channel();
                let profile = shared.config().layout.profiles.first().cloned();
                let cfg = shared.config().clone();
                (
                    Some(ReverseHandle::spawn(
                        &cfg,
                        local.clone(),
                        profile,
                        rdeps,
                        fast,
                        tx,
                        forward(&events_hook),
                    )),
                    Some(rx),
                )
            }
            None => (None, None),
        };
        let injector = RoutedInjector::new(deps.injector, deps.pipe);
        let mut rx = Receiver::new(&shared.config(), local, injector, BoxCursor(deps.cursor));
        let (cmd_worker, cmd_out) = match deps.executor {
            Some(ex) => {
                let (w, out) = spawn_command_worker(ex);
                (Some(w), Some(out))
            }
            None => (None, None),
        };
        if let Some(h) = forward(&events_hook) {
            rx.set_event_handler(h);
        }
        // Cópia do layout já guardada: o retorno funciona logo na primeira conexão (D11).
        let profile = shared.config().layout.profiles.first().cloned();
        if let Some(p) = profile {
            rx.set_profile(p);
        }
        let status_rx = listener.status();
        let mut media = deps.media;
        let media_out = media.as_mut().and_then(|m| m.take_outbox());
        Self {
            shared,
            listener,
            rx,
            keystore: deps.keystore,
            events,
            displays: deps.displays,
            _advertiser: advertiser,
            _foreground: deps.foreground,
            pointer_guard: deps.pointer_guard,
            guard: None,
            hook: deps.message_hook,
            media,
            media_out,
            status_rx,
            peer: peers.into_iter().next(),
            foreground: None,
            paused: false,
            pairing_until: None,
            last_latency: None,
            rev,
            mac_displays: Vec::new(),
            cmd_worker,
            cmd_out,
            rev_out,
        }
    }

    pub(super) async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<Cmd>) {
        let mut status_rx = self.listener.status();
        let mut stats_rx = self.listener.stats();
        let mut clock = ticker(TICK_IDLE);
        self.publish();
        let mut fast_clock = false;
        loop {
            tokio::select! {
                ev = self.listener.recv_event() => match ev {
                    Some(ev) => self.on_session_event(ev).await,
                    None => break,
                },
                cmd = cmds.recv() => match cmd {
                    Some(Cmd::Shutdown) | None => break,
                    Some(cmd) => self.on_cmd(cmd).await,
                },
                r = status_rx.changed() => {
                    if r.is_err() { break; }
                    self.on_status().await;
                }
                r = stats_rx.changed() => {
                    if r.is_err() { break; }
                    self.on_stats(*stats_rx.borrow_and_update());
                }
                out = recv_media(&mut self.media_out) => match out {
                    Some(MediaOut::Send(m)) => self.send_all(vec![m]).await,
                    Some(MediaOut::State) => self.publish(),
                    None => self.media_out = None,
                },
                _ = clock.tick() => self.on_tick(),
                m = recv_rev(&mut self.cmd_out) => match m {
                    Some(m) => self.send_all(vec![m]).await,
                    None => self.cmd_out = None,
                },
                m = recv_rev(&mut self.rev_out) => match m {
                    Some(m) => self.send_all(vec![m]).await,
                    None => self.rev_out = None,
                },
            }
            let active = self.rx.is_active();
            if let Some(rev) = &self.rev {
                // D24: com o Mac controlando o PC, o PC não assume.
                rev.set_blocked(active);
            }
            if active != fast_clock {
                fast_clock = active;
                clock = ticker(if active { TICK_ACTIVE } else { TICK_IDLE });
                self.publish();
            }
        }
        self.rx.focus_lost();
        self.guard = None;
        // Desliga o áudio e restaura a saída padrão antes de fechar a sessão.
        self.media = None;
        if let Some(rev) = self.rev.as_mut() {
            rev.shutdown();
        }
        let Pump { listener, .. } = self;
        listener.shutdown().await;
    }

    // ---- relógio, estatísticas e snapshot ----

    fn on_tick(&mut self) {
        let now = Instant::now();
        if self.rx.is_active() {
            self.rx.tick(now);
        }
        if self.pairing_until.is_some_and(|t| now >= t) {
            self.pairing_until = None;
            self.publish();
        }
    }

    fn on_stats(&mut self, stats: relay_core::types::LatencyStats) {
        let now = Instant::now();
        if self
            .last_latency
            .is_none_or(|t| now.duration_since(t) >= LATENCY_EVERY)
        {
            self.last_latency = Some(now);
            self.events.latency(&stats);
        }
    }

    fn connected(&self) -> bool {
        *self.status_rx.borrow() == ConnectionStatus::Connected
    }

    fn publish(&mut self) {
        let status = *self.status_rx.borrow();
        let peer_version = self.listener.peer_version();
        let snap = StatusSnapshot {
            status: if status != ConnectionStatus::Connected && self.pairing_until.is_some() {
                ConnectionStatus::Pairing
            } else {
                status
            },
            peer_name: self.peer.as_ref().map(|p| p.name.clone()),
            focus: if self.rx.is_active() {
                Machine::Windows
            } else {
                Machine::Mac
            },
            locked: false,
            paused: self.paused,
            secure_input: false,
            pc_unreachable: false,
            clipboard_sync: self.shared.config().clipboard.enabled,
            peer_version: peer_version.clone(),
            version_sync: version_sync(APP_VERSION, peer_version.as_deref()),
            audio: self
                .media
                .as_ref()
                .map(|m| m.audio_state())
                .unwrap_or_default(),
        };
        let mut cur = self.shared.snapshot();
        if *cur != snap {
            *cur = snap.clone();
            drop(cur);
            self.events.status(&snap);
        }
    }

    async fn send_all(&mut self, msgs: Vec<Message>) {
        for m in msgs {
            if let Err(e) = self.listener.send(m).await {
                tracing::debug!(error = %e, "mensagem não enviada");
            }
        }
    }

    // ---- sessão ----

    /// Saiu de `Connected`: solta teclas, restaura o ponteiro (D27).
    async fn on_status(&mut self) {
        if !self.connected() {
            self.rx.focus_lost();
            self.guard = None;
            if let Some(rev) = &self.rev {
                rev.set_link(false);
            }
        }
        // Sessão de pé: a mídia recebe IP do Mac e chaves de áudio; caiu: desliga o áudio.
        let link = self
            .connected()
            .then(|| self.listener.audio_link())
            .flatten();
        if let Some(m) = self.media.as_mut() {
            m.link_changed(link);
        }
        self.publish();
    }

    async fn on_session_event(&mut self, ev: SessionEvent) {
        match ev {
            SessionEvent::Connected(peer) => self.on_connected(peer).await,
            SessionEvent::Message(m) => self.on_message(m).await,
            SessionEvent::Paired(peer) => self.on_paired(peer),
            SessionEvent::PairFailed(reason) => {
                tracing::warn!(%reason, "pareamento falhou");
                self.pairing_until = None;
                self.events
                    .pairing(&pairing_event(PairingState::Failed, None, None));
                self.publish();
            }
            SessionEvent::Refused => tracing::warn!("conexão de peer não pareado recusada"),
        }
    }

    async fn on_connected(&mut self, peer: PairedPeer) {
        self.peer = Some(peer);
        if self.guard.is_none() {
            self.guard = Some((self.pointer_guard)());
        }
        let local = self.displays.list().unwrap_or_default();
        if let Some(m) = self.media.as_mut() {
            m.link_changed(self.listener.audio_link());
        }
        self.rx.set_local_displays(local.clone());
        if let Some(rev) = &self.rev {
            rev.set_local_displays(local.clone());
            rev.set_link(true);
        }
        let mut out = vec![Message::Displays(local)];
        if let Some(exe) = &self.foreground {
            out.push(Message::ForegroundApp { exe: exe.clone() });
        }
        self.send_all(out).await;
        self.publish();
    }

    fn on_paired(&mut self, peer: PairedPeer) {
        self.pairing_until = None;
        if let Err(e) = save_active_peer(&*self.keystore, ACTIVE_PEER_ACCOUNT, &peer) {
            tracing::error!(error = %e, "não foi possível guardar o peer pareado");
            self.listener.remove_peer(&peer.device_id);
            self.events
                .pairing(&pairing_event(PairingState::Failed, None, None));
            self.publish();
            return;
        }
        if let Some(old) = self.peer.take()
            && old.device_id != peer.device_id
        {
            let _ = remove_peer(&*self.keystore, &old.device_id);
            self.listener.remove_peer(&old.device_id);
        }
        self.peer = Some(peer);
        self.events
            .pairing(&pairing_event(PairingState::Paired, None, None));
        self.publish();
    }

    fn forget_peer(&mut self) -> Option<PairedPeer> {
        let peer = self.peer.take()?;
        let _ = forget_active_peer(&*self.keystore, ACTIVE_PEER_ACCOUNT, &peer.device_id);
        self.listener.remove_peer(&peer.device_id);
        Some(peer)
    }

    async fn on_message(&mut self, msg: Message) {
        let now = Instant::now();
        let mut out = match self.hook.as_mut() {
            Some(h) => h(&msg),
            None => Vec::new(),
        };
        match &msg {
            Message::Displays(d) => self.mac_displays = d.clone(),
            Message::LayoutSync(profile) => self.persist_layout(profile),
            Message::Unpair => {
                self.forget_peer();
                self.rx.focus_lost();
                self.publish();
            }
            // O pedido só vale se o Mac está de fato numa versão mais nova que esta.
            Message::RequestUpdate { version }
                if version_sync(APP_VERSION, Some(version)) == VersionSync::LocalBehind =>
            {
                self.events.update_requested(version);
            }
            _ => {}
        }
        // Com o PC controlando o Mac, o `FocusReturn` é do Controller (pedido de retorno do Mac ou
        // recusa de um `FocusEnter`); fora disso é o Mac largando o PC.
        let mut to_controller = false;
        if let Some(rev) = &self.rev {
            match &msg {
                Message::Displays(_) => rev.send(msg.clone()),
                Message::LayoutSync(p) => rev.set_profile(p.clone()),
                Message::FocusReturn { .. } if rev.is_remote() => {
                    rev.send(msg.clone());
                    to_controller = true;
                }
                _ => {}
            }
        }
        // Pausado, um `FocusEnter` do Mac é devolvido na hora: ele não pode ficar engolindo input.
        if self.paused && matches!(msg, Message::FocusEnter { .. }) {
            out.extend(self.focus_return());
        }
        // Comandos (exceto TypeText, que usa o injetor) rodam na thread de comandos.
        let mut to_worker = false;
        if let (Message::Command { id, cmd }, Some(w)) = (&msg, &self.cmd_worker)
            && !matches!(cmd, relay_core::types::RemoteCommand::TypeText { .. })
        {
            to_worker = w.send((*id, cmd.clone())).is_ok();
        }
        let ignored = to_controller
            || to_worker
            || (self.paused && matches!(msg, Message::Input { .. } | Message::FocusEnter { .. }));
        if !ignored {
            out.extend(self.rx.on_message(msg, now));
        }
        self.send_all(out).await;
    }

    /// `LayoutSync` recebido vira cópia no Config do agente (substitui o perfil de mesma chave).
    fn persist_layout(&self, profile: &relay_core::types::LayoutProfile) {
        let snapshot = {
            let mut cfg = self.shared.config();
            match cfg
                .layout
                .profiles
                .iter_mut()
                .find(|p| p.key == profile.key)
            {
                Some(slot) if slot == profile => return,
                Some(slot) => *slot = profile.clone(),
                None => cfg.layout.profiles.push(profile.clone()),
            }
            cfg.clone()
        };
        self.shared.save_config(&snapshot);
    }

    // ---- comandos da UI e do sistema ----

    async fn on_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::OpenPairing {
                code,
                expires_at_ms,
            } => {
                let ttl = Duration::from_millis(expires_at_ms.saturating_sub(now_ms()));
                self.listener.open_pairing(code.clone(), ttl);
                self.pairing_until = Some(Instant::now() + ttl);
                self.events.pairing(&pairing_event(
                    PairingState::Code,
                    Some(code),
                    Some(expires_at_ms),
                ));
                self.publish();
            }
            Cmd::Revoke => {
                if self.forget_peer().is_some() {
                    self.send_all(vec![Message::Unpair]).await;
                    tokio::time::sleep(UNPAIR_FLUSH).await;
                    self.listener.force_reconnect();
                    self.rx.focus_lost();
                }
                self.publish();
            }
            Cmd::SetPaused(paused) => {
                self.paused = paused;
                if let Some(rev) = &self.rev {
                    rev.set_paused(paused);
                }
                if paused {
                    // Com o Mac controlando o PC, devolve o foco antes de largar tudo.
                    let ret = if self.rx.is_active() {
                        self.focus_return()
                    } else {
                        None
                    };
                    self.rx.focus_lost();
                    self.send_all(ret.into_iter().collect()).await;
                }
                self.publish();
            }
            Cmd::DisplaysChanged => {
                let local = self.displays.list().unwrap_or_default();
                self.rx.set_local_displays(local.clone());
                if let Some(rev) = &self.rev {
                    rev.set_local_displays(local.clone());
                }
                if self.connected() {
                    self.send_all(vec![Message::Displays(local)]).await;
                }
            }
            Cmd::Foreground(exe) => {
                self.foreground = Some(exe.clone());
                if self.connected() {
                    self.send_all(vec![Message::ForegroundApp { exe }]).await;
                }
            }
            Cmd::Fast(ev) => {
                if !self.paused {
                    let out = self.rx.on_input(ev.event, Instant::now());
                    self.send_all(out).await;
                }
            }
            Cmd::ConfigChanged => {
                let clipboard = self.shared.config().clipboard.clone();
                self.rx.set_clipboard_config(clipboard);
                let cfg = self.shared.config().clone();
                if let Some(m) = self.media.as_mut() {
                    m.config_changed(&cfg);
                }
                if let Some(rev) = &self.rev {
                    rev.apply_config(&self.shared.config());
                }
                self.publish();
            }
            Cmd::Shutdown => {}
        }
    }
}

/// Próxima saída da mídia (nunca resolve se não há mídia).
async fn recv_media(rx: &mut Option<mpsc::UnboundedReceiver<MediaOut>>) -> Option<MediaOut> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn pairing_event(
    state: PairingState,
    code: Option<String>,
    expires_at: Option<u64>,
) -> PairingEvent {
    PairingEvent {
        state,
        code,
        expires_at,
    }
}
