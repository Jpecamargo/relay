//! dono: WI1
//! O laço do agente: uma tarefa tokio dona da sessão, do `Receiver` e do guard do ponteiro.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_core::compat::v02::legacy_peer_os;
use relay_core::crypto::{
    LocalDevice, StaticKeypair, forget_active_peer, remove_peer, save_active_peer,
};
use relay_core::discovery::{Advertiser, parse_manual_address};
use relay_core::engine::Receiver;
use relay_core::net::{
    DialerConfig, FastEvent, Session, SessionDialer, SessionEvent, SessionListener,
};
use relay_core::node::session::{DIAL_FALLBACK, caps_message, keep_new_session};
use relay_core::platform::{DisplaySource, KeyStore};
use relay_core::types::{
    Caps, ConnectionStatus, DiscoveredPeer, Host, LatencyStats, Message, Os, PairedPeer,
    PairingEvent, PairingState, Ports, Role, StatusSnapshot, VersionSync,
};
use relay_core::version::{APP_VERSION, update_request, version_sync};
use tokio::sync::{mpsc, watch};

use tokio::time::{Interval, MissedTickBehavior, interval};

use super::dialing::{DialPlanner, local_caps, target_of};
use super::media::{MediaOut, MediaPort};
use super::reverse::{ReverseDeps, ReverseHandle};
use super::routed::{BoxCursor, RoutedInjector};
use super::{
    ACTIVE_PEER_ACCOUNT, AgentDeps, BrowseFactory, Cmd, ForegroundSource, MessageHook, PairReply,
    Shared, Side, UiEvents, now_ms,
};

/// Relógio do `Receiver` enquanto o Mac controla este lado (repetição de tecla ≤ 10 ms).
const TICK_ACTIVE: Duration = Duration::from_millis(5);
/// Relógio ocioso: só expira o código de pareamento.
const TICK_IDLE: Duration = Duration::from_millis(250);
/// `relay:latency` no máximo a 2 Hz.
const LATENCY_EVERY: Duration = Duration::from_millis(500);
/// Folga para o `Unpair` sair antes de derrubar a sessão.
const UNPAIR_FLUSH: Duration = Duration::from_millis(100);
/// Teto de uma tentativa de pareamento discando (como no Mac).
const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
/// Espera entre leituras do mDNS (e entre tentativas de reabrir a busca).
const BROWSE_TICK: Duration = Duration::from_millis(500);
const BROWSE_RETRY: Duration = Duration::from_secs(5);

/// Quem somos na rede (o que o pareamento discado e a discagem precisam) e o par já guardado.
pub(super) struct Me {
    pub id: String,
    pub identity: StaticKeypair,
    pub device: LocalDevice,
    pub peer: Option<PairedPeer>,
}

/// Sessão que este PC iniciou (`SessionDialer`), ao lado da que o `SessionListener` recebe.
struct Dial {
    session: SessionDialer,
    status: watch::Receiver<ConnectionStatus>,
    stats: watch::Receiver<LatencyStats>,
    target: SocketAddr,
}

impl Dial {
    fn up(&self) -> bool {
        *self.status.borrow() == ConnectionStatus::Connected
    }
}

/// Mensagens que abrem uma sessão simétrica: `Caps` (só a par 0.3, D40) e o pedido de atualização
/// (D49). Vazio para um Mac 0.2, que derrubaria a sessão ao ver a variante.
pub(super) fn handshake_messages(
    local: Caps,
    local_version: &str,
    peer_version: Option<&str>,
    peer_os: Os,
) -> Vec<Message> {
    caps_message(local, peer_version)
        .into_iter()
        .chain(update_request(local_version, peer_version, peer_os))
        .collect()
}

/// Thread da busca mDNS: manda cada par visto (menos o próprio) ao pump e reabre a busca se a
/// plataforma falhar. Para quando `stop` liga ou o pump some.
fn spawn_discovery(
    factory: BrowseFactory,
    tx: mpsc::UnboundedSender<Cmd>,
    own_id: String,
    stop: Arc<AtomicBool>,
) {
    let run = move || {
        while !stop.load(Ordering::Relaxed) {
            match factory() {
                Ok(mut browse) => {
                    while !stop.load(Ordering::Relaxed) {
                        if let Some(p) = browse.next_peer(BROWSE_TICK)
                            && p.device_id != own_id
                            && tx.send(Cmd::Discovered(p)).is_err()
                        {
                            return;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mDNS indisponível; só pelo endereço manual");
                    let until = Instant::now() + BROWSE_RETRY;
                    while !stop.load(Ordering::Relaxed) && Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("relay-mdns".into())
        .spawn(run)
    {
        tracing::warn!(error = %e, "busca mDNS não iniciou");
    }
}

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
    me: Me,
    bind_ip: std::net::IpAddr,
    /// Versão anunciada no Hello e usada nas comparações de versão.
    app_version: String,
    /// Sessão que este PC discou (D41); a do listener é `listener`.
    dialer: Option<Dial>,
    /// Qual das duas é a sessão em uso (`None` = nenhuma de pé).
    active: Option<Side>,
    planner: DialPlanner,
    browse_stop: Arc<AtomicBool>,
    /// Último `Caps` que o par mandou (fica entre sessões: o painel mostra o par offline).
    peer_caps: Option<Caps>,
    local_caps: Caps,
    /// Host dos layouts escolhido em `pair_with_code` (D44), até o layout o aplicar.
    pending_layout_host: Option<Host>,
    foreground: Option<String>,
    paused: bool,
    /// Cursor travado pela UI (`set_cursor_lock`).
    locked: bool,
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
        reverse: Option<ReverseDeps>,
        mut me: Me,
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
        let browse_stop = Arc::new(AtomicBool::new(false));
        if let Some(factory) = deps.browse {
            spawn_discovery(
                factory,
                shared.tx.clone(),
                me.id.clone(),
                browse_stop.clone(),
            );
        }
        let vbcable = vbcable_detected(&shared);
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
            peer: me.peer.take(),
            me,
            bind_ip: deps.bind_ip,
            app_version: deps.app_version.unwrap_or_else(|| APP_VERSION.to_owned()),
            dialer: None,
            active: None,
            planner: DialPlanner::new(DIAL_FALLBACK),
            browse_stop,
            peer_caps: None,
            local_caps: local_caps(vbcable),
            pending_layout_host: None,
            foreground: None,
            paused: false,
            locked: false,
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
                    Some(ev) => self.on_session_event(Side::Listener, ev).await,
                    None => break,
                },
                wake = dial_wake(&mut self.dialer) => match wake {
                    DialWake::Event(Some(ev)) => self.on_session_event(Side::Dialer, ev).await,
                    // A sessão discada acabou (shutdown): sem ela, só o listener segue.
                    DialWake::Event(None) | DialWake::Status(false) => {
                        self.dialer = None;
                        self.on_status().await;
                    }
                    DialWake::Status(true) => self.on_status().await,
                    DialWake::Stats => {
                        if self.active == Some(Side::Dialer)
                            && let Some(d) = self.dialer.as_mut()
                        {
                            let stats = *d.stats.borrow_and_update();
                            self.on_stats(stats);
                        }
                    }
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
                    let stats = *stats_rx.borrow_and_update();
                    if self.active != Some(Side::Dialer) {
                        self.on_stats(stats);
                    }
                }
                out = recv_media(&mut self.media_out) => match out {
                    Some(MediaOut::Send(m)) => self.send_all(vec![m]).await,
                    Some(MediaOut::State) => self.publish(),
                    None => self.media_out = None,
                },
                _ = clock.tick() => self.on_tick().await,
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
        self.browse_stop.store(true, Ordering::Relaxed);
        // Desliga o áudio e restaura a saída padrão antes de fechar a sessão.
        self.media = None;
        if let Some(rev) = self.rev.as_mut() {
            rev.shutdown();
        }
        let Pump {
            listener, dialer, ..
        } = self;
        if let Some(d) = dialer {
            d.session.shutdown().await;
        }
        listener.shutdown().await;
    }

    // ---- relógio, estatísticas e snapshot ----

    async fn on_tick(&mut self) {
        let now = Instant::now();
        if self.rx.is_active() {
            self.rx.tick(now);
        }
        if self.pairing_until.is_some_and(|t| now >= t) {
            self.pairing_until = None;
            self.publish();
        }
        // O outro lado devia ter discado e não discou: depois do fallback, este PC disca.
        if let Some(p) = self.planner.due(now, self.active.is_some())
            && let Some(target) = target_of(&p)
        {
            self.start_dial(target).await;
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
        self.active.is_some()
    }

    fn session(&self, side: Side) -> Option<&Session> {
        match side {
            Side::Listener => Some(&self.listener),
            Side::Dialer => self.dialer.as_ref().map(|d| &*d.session),
        }
    }

    fn side_up(&self, side: Side) -> bool {
        match side {
            Side::Listener => *self.status_rx.borrow() == ConnectionStatus::Connected,
            Side::Dialer => self.dialer.as_ref().is_some_and(Dial::up),
        }
    }

    /// Status de conexão que a UI vê: o da sessão em uso; sem ela, o da que está tentando.
    fn link_status(&self) -> ConnectionStatus {
        if self.active.is_some() {
            return ConnectionStatus::Connected;
        }
        let listener = *self.status_rx.borrow();
        let dialer = self.dialer.as_ref().map(|d| *d.status.borrow());
        match (listener, dialer) {
            (s @ ConnectionStatus::IncompatibleVersion { .. }, _)
            | (_, Some(s @ ConnectionStatus::IncompatibleVersion { .. })) => s,
            (_, Some(d)) => d,
            (l, None) => l,
        }
    }

    /// SO e `Caps` do par: o que ele anunciou; senão o baseline do SO legado (par 0.2 ou ainda
    /// sem `Caps`). Com par salvo isso vale também desconectado; sem par, nada.
    fn peer_identity(&self) -> (Option<Os>, Option<Caps>) {
        if self.peer.is_none() && self.active.is_none() {
            return (None, None);
        }
        let caps = self
            .peer_caps
            .unwrap_or_else(|| Caps::baseline(legacy_peer_os(Role::Dialer)));
        (Some(caps.os), Some(caps))
    }

    fn publish(&mut self) {
        let status = self.link_status();
        let peer_version = self
            .active
            .and_then(|s| self.session(s))
            .and_then(|s| s.peer_version());
        let (peer_os, peer_caps) = self.peer_identity();
        let snap = StatusSnapshot {
            status: if status != ConnectionStatus::Connected && self.pairing_until.is_some() {
                ConnectionStatus::Pairing
            } else {
                status
            },
            peer_name: self.peer.as_ref().map(|p| p.name.clone()),
            focus: if self.rx.is_active() {
                Host::Local
            } else {
                Host::Peer
            },
            locked: self.locked,
            paused: self.paused,
            secure_input: false,
            peer_unreachable: false,
            clipboard_sync: self.shared.config().clipboard.enabled,
            peer_version: peer_version.clone(),
            version_sync: version_sync(&self.app_version, peer_version.as_deref()),
            audio: self
                .media
                .as_ref()
                .map(|m| m.audio_state())
                .unwrap_or_default(),
            local_os: Os::Windows,
            peer_os,
            local_caps: self.local_caps,
            peer_caps,
            layout_host: Host::Peer,
            layout_editable: false,
        };
        let mut cur = self.shared.snapshot();
        if *cur != snap {
            *cur = snap.clone();
            drop(cur);
            self.events.status(&snap);
        }
    }

    async fn send_all(&mut self, msgs: Vec<Message>) {
        let Some(session) = self.active.and_then(|s| self.session(s)) else {
            return;
        };
        for m in msgs {
            if let Err(e) = session.send(m).await {
                tracing::debug!(error = %e, "mensagem não enviada");
            }
        }
    }

    // ---- sessão ----

    /// Alguma sessão mudou de estado: escolhe qual está em uso (resolvendo a duplicada) e liga ou
    /// desliga o que depende dela.
    async fn on_status(&mut self) {
        let desired = match (self.side_up(Side::Listener), self.side_up(Side::Dialer)) {
            (false, false) => None,
            (true, false) => Some(Side::Listener),
            (false, true) => Some(Side::Dialer),
            (true, true) => Some(self.resolve_duplicate().await),
        };
        if desired != self.active {
            if self.active.is_some() {
                self.deactivate();
            }
            if let Some(side) = desired {
                self.activate(side).await;
            }
        }
        self.publish();
    }

    /// Duas sessões de pé com o mesmo par (D41): fica a de dialer com menor `device_id` e a outra
    /// recebe `Bye`. Os dois lados avaliam a mesma dupla e chegam à mesma sessão.
    async fn resolve_duplicate(&mut self) -> Side {
        let existing = self.active.unwrap_or(Side::Dialer);
        let new = other(existing);
        let dialer_id = |side: Side| match side {
            Side::Dialer => self.me.id.clone(),
            Side::Listener => self
                .peer
                .as_ref()
                .map_or_else(String::new, |p| p.device_id.clone()),
        };
        let keep = if keep_new_session(&dialer_id(existing), &dialer_id(new)) {
            new
        } else {
            existing
        };
        self.drop_session(other(keep)).await;
        keep
    }

    /// Derruba uma das sessões com `Bye`. A do listener só volta a escutar; a discada é encerrada
    /// (o `SessionDialer` reconectaria sozinho) e só rediscará se o planner mandar de novo.
    async fn drop_session(&mut self, side: Side) {
        match side {
            Side::Listener => self.listener.force_reconnect(),
            Side::Dialer => self.stop_dial().await,
        }
    }

    async fn stop_dial(&mut self) {
        if let Some(d) = self.dialer.take() {
            d.session.shutdown().await;
        }
    }

    /// A sessão em uso caiu: solta teclas, restaura o ponteiro (D27) e desliga mídia e reverso.
    fn deactivate(&mut self) {
        self.active = None;
        self.rx.focus_lost();
        self.guard = None;
        if let Some(rev) = &self.rev {
            rev.set_link(false);
        }
        if let Some(m) = self.media.as_mut() {
            m.link_changed(None);
        }
    }

    /// `side` passou a ser a sessão em uso: ponteiro, mídia, reverso e as primeiras mensagens.
    async fn activate(&mut self, side: Side) {
        self.active = Some(side);
        if self.guard.is_none() {
            self.guard = Some((self.pointer_guard)());
        }
        let local = self.displays.list().unwrap_or_default();
        let audio = self.session(side).and_then(|s| s.audio_link());
        if let Some(m) = self.media.as_mut() {
            m.link_changed(audio);
        }
        self.rx.set_local_displays(local.clone());
        if let Some(rev) = &self.rev {
            rev.use_side(side);
            rev.set_local_displays(local.clone());
            rev.set_link(true);
        }
        let peer_version = self.session(side).and_then(|s| s.peer_version());
        let mut out = vec![Message::Displays(local)];
        if let Some(exe) = &self.foreground {
            out.push(Message::ForegroundApp { exe: exe.clone() });
        }
        self.local_caps = local_caps(vbcable_detected(&self.shared));
        out.extend(handshake_messages(
            self.local_caps,
            &self.app_version,
            peer_version.as_deref(),
            self.peer_identity().0.unwrap_or(Os::Mac),
        ));
        self.send_all(out).await;
    }

    async fn on_session_event(&mut self, side: Side, ev: SessionEvent) {
        // O status pode ainda não ter chegado ao laço: confere antes de decidir de qual sessão ouvir.
        self.on_status().await;
        match ev {
            SessionEvent::Connected(peer) => {
                if self.active == Some(side) {
                    self.peer = Some(peer);
                    self.publish();
                }
            }
            SessionEvent::Message(m) if self.active == Some(side) => self.on_message(m).await,
            SessionEvent::Message(_) => {}
            SessionEvent::Paired(peer) => self.on_paired(peer).await,
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

    /// O pareamento concluiu (aqui como Listener ou discando): guarda o par, esquece o anterior e
    /// já considera discar se o mDNS viu o par.
    async fn on_paired(&mut self, peer: PairedPeer) {
        self.pairing_until = None;
        if let Err(e) = self.adopt_peer(&peer) {
            tracing::error!(error = %e, "não foi possível guardar o peer pareado");
            self.listener.remove_peer(&peer.device_id);
            self.events
                .pairing(&pairing_event(PairingState::Failed, None, None));
            self.publish();
            return;
        }
        self.events
            .pairing(&pairing_event(PairingState::Paired, None, None));
        self.publish();
        let seen = self
            .shared
            .discovered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|d| d.device_id == peer.device_id)
            .cloned();
        if let Some(d) = seen {
            self.on_discovered(d).await;
        }
    }

    /// Persiste `peer` como o par ativo e esquece o anterior (se outro).
    fn adopt_peer(&mut self, peer: &PairedPeer) -> Result<(), relay_core::platform::KeyStoreError> {
        save_active_peer(&*self.keystore, ACTIVE_PEER_ACCOUNT, peer)?;
        if let Some(old) = self.peer.take()
            && old.device_id != peer.device_id
        {
            let _ = remove_peer(&*self.keystore, &old.device_id);
            self.listener.remove_peer(&old.device_id);
            self.peer_caps = None;
        }
        self.peer = Some(peer.clone());
        Ok(())
    }

    fn forget_peer(&mut self) -> Option<PairedPeer> {
        let peer = self.peer.take()?;
        let _ = forget_active_peer(&*self.keystore, ACTIVE_PEER_ACCOUNT, &peer.device_id);
        self.listener.remove_peer(&peer.device_id);
        self.peer_caps = None;
        self.planner.forget();
        Some(peer)
    }

    // ---- discagem (D41) ----

    /// Par visto pelo mDNS: guarda para a UI e disca se a regra mandar.
    async fn on_discovered(&mut self, found: DiscoveredPeer) {
        {
            let mut seen = self
                .shared
                .discovered
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            seen.retain(|d| d.device_id != found.device_id);
            seen.push(found.clone());
        }
        let paired = self.peer.as_ref().map(|p| p.device_id.as_str());
        let dial = self
            .planner
            .on_discovered(&self.me.id, paired, &found, Instant::now());
        if let Some(p) = dial
            && let Some(target) = target_of(&p)
        {
            self.start_dial(target).await;
        }
    }

    /// Disca o par pareado em `target` (porta de controle). Já discando no mesmo endereço, ou com
    /// a sessão discada de pé, nada muda.
    async fn start_dial(&mut self, target: SocketAddr) {
        let Some(peer) = self.peer.clone() else {
            return;
        };
        if let Some(d) = &self.dialer {
            if d.target == target || d.up() {
                return;
            }
            self.stop_dial().await;
        }
        let cfg = self.shared.config().network.ports.clone();
        // Porta 0 = efêmera (só testes): o UDP do par é a seguinte à do controle.
        let remote_input = if cfg.input == 0 {
            target.port().wrapping_add(1)
        } else {
            cfg.input
        };
        let remote = Ports {
            control: target.port(),
            input: remote_input,
            ..cfg.clone()
        };
        // O UDP do canal rápido desta sessão não pode dividir a porta com o do listener.
        let local = Ports { input: 0, ..cfg };
        let mut dial =
            DialerConfig::new(self.me.identity.clone(), peer, target.ip(), remote, local);
        dial.bind_ip = self.bind_ip;
        dial.app_version = self.app_version.clone();
        let fast_tx = self.shared.tx.clone();
        let started = SessionDialer::start(
            dial,
            Box::new(move |ev: FastEvent| {
                let _ = fast_tx.send(Cmd::Fast(ev));
            }),
        )
        .await;
        let mut session = match started {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "sessão discada não iniciou");
                return;
            }
        };
        if let (Some(rev), Some(fast)) = (&self.rev, session.take_input_producer()) {
            rev.set_fast(Side::Dialer, fast);
        }
        self.dialer = Some(Dial {
            status: session.status(),
            stats: session.stats(),
            session,
            target,
        });
    }

    /// Pareamento discando: a rede roda numa tarefa e o resultado volta ao pump (`PairDone`).
    fn pair_with(&mut self, code: String, address: Option<String>, host: Host, reply: PairReply) {
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            let _ = reply.send(Err("O código deve ter 6 dígitos.".into()));
            return;
        }
        let (found, manual) = (
            self.shared
                .discovered
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            self.shared.config().network.manual_address.clone(),
        );
        let (device, tx) = (self.me.device.clone(), self.shared.tx.clone());
        tokio::spawn(async move {
            let result = async {
                let addr = pair_target(address, &found, manual).await?;
                let peer =
                    tokio::time::timeout(PAIR_TIMEOUT, SessionDialer::pair(addr, &code, &device))
                        .await
                        .map_err(|_| "O Mac não respondeu a tempo.".to_string())?
                        .map_err(|e| e.to_string())?;
                Ok((peer, addr))
            }
            .await;
            let _ = tx.send(Cmd::PairDone {
                result,
                layout_host: host,
                reply,
            });
        });
    }

    async fn pair_done(
        &mut self,
        result: Result<(PairedPeer, SocketAddr), String>,
        layout_host: Host,
        reply: PairReply,
    ) {
        let (peer, addr) = match result {
            Ok(ok) => ok,
            Err(e) => {
                self.events
                    .pairing(&pairing_event(PairingState::Failed, None, None));
                let _ = reply.send(Err(e));
                return;
            }
        };
        if let Err(e) = self.adopt_peer(&peer) {
            self.events
                .pairing(&pairing_event(PairingState::Failed, None, None));
            let _ = reply.send(Err(e.to_string()));
            return;
        }
        // O Mac passa a ser aceito também como Dialer; reparear invalida a sessão atual.
        self.listener.add_peer(peer);
        self.planner.forget();
        self.stop_dial().await;
        if self.active.is_some() {
            self.listener.force_reconnect();
        }
        self.pending_layout_host = Some(layout_host);
        self.start_dial(addr).await;
        self.events
            .pairing(&pairing_event(PairingState::Paired, None, None));
        self.publish();
        let _ = reply.send(Ok(()));
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
            Message::Caps(caps) => {
                self.peer_caps = Some(*caps);
                self.publish();
            }
            Message::Unpair => {
                self.forget_peer();
                self.stop_dial().await;
                self.rx.focus_lost();
                self.publish();
            }
            // O pedido só vale se o Mac está de fato numa versão mais nova que esta.
            Message::RequestUpdate { version }
                if version_sync(&self.app_version, Some(version)) == VersionSync::LocalBehind =>
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
                Message::TakeControl => {
                    rev.send(msg.clone());
                    to_controller = true;
                }
                // `is_remote` é um espelho que atrasa uma volta do laço reverso: sem o Mac
                // controlando o PC (`rx` inativo) o `FocusReturn` só pode ser do sentido inverso.
                Message::FocusReturn { .. } if rev.is_remote() || !self.rx.is_active() => {
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

    /// `LayoutSync` recebido vira a cópia única no Config do agente: o perfil em uso no Mac,
    /// usado no retorno antes da primeira sincronização (D11).
    fn persist_layout(&self, profile: &relay_core::types::LayoutProfile) {
        let snapshot = {
            let mut cfg = self.shared.config();
            if cfg.layout.profiles.as_slice() == std::slice::from_ref(profile) {
                return;
            }
            cfg.layout.profiles = vec![profile.clone()];
            cfg.layout.active = None;
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
            Cmd::PairWith {
                code,
                address,
                layout_host,
                reply,
            } => self.pair_with(code, address, layout_host, reply),
            Cmd::PairDone {
                result,
                layout_host,
                reply,
            } => self.pair_done(result, layout_host, reply).await,
            Cmd::Discovered(found) => self.on_discovered(found).await,
            #[cfg(test)]
            Cmd::PendingLayoutHost(reply) => {
                let _ = reply.send(self.pending_layout_host);
            }
            Cmd::Revoke => {
                if self.forget_peer().is_some() {
                    self.send_all(vec![Message::Unpair]).await;
                    tokio::time::sleep(UNPAIR_FLUSH).await;
                    self.stop_dial().await;
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
            Cmd::SetLocked(locked) => {
                self.locked = locked;
                if let Some(rev) = &self.rev {
                    rev.set_locked(locked);
                }
                self.publish();
            }
            Cmd::JumpTo(n) => {
                if let Some(rev) = &self.rev {
                    rev.jump_to(n);
                }
            }
            Cmd::ClipboardSendNow => {
                if let Some(m) = self.media.as_mut() {
                    m.clipboard_send_now();
                }
            }
            Cmd::Reconnect => {
                self.listener.force_reconnect();
                if let Some(d) = &self.dialer {
                    d.session.force_reconnect();
                }
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
                // VB-CABLE instalado (ou removido) com o app aberto: o par fica sabendo.
                let caps = local_caps(vbcable_detected(&self.shared));
                if caps != self.local_caps {
                    self.local_caps = caps;
                    let peer_version = self
                        .active
                        .and_then(|s| self.session(s))
                        .and_then(|s| s.peer_version());
                    let msg = caps_message(caps, peer_version.as_deref());
                    self.send_all(msg.into_iter().collect()).await;
                }
                self.publish();
            }
            Cmd::Shutdown => {}
        }
    }
}

fn other(side: Side) -> Side {
    match side {
        Side::Listener => Side::Dialer,
        Side::Dialer => Side::Listener,
    }
}

/// VB-CABLE detectado agora (consulta o SCM e o Core Audio: só em eventos raros, nunca no `publish`).
fn vbcable_detected(shared: &Shared) -> bool {
    let helper = shared
        .pipe
        .as_ref()
        .is_some_and(|p| p.lock().unwrap_or_else(|e| e.into_inner()).is_available());
    (shared.service_probe)(helper).vbcable_detected
}

/// O que acordou o laço na sessão discada.
enum DialWake {
    Event(Option<SessionEvent>),
    /// `false` = o canal de status fechou.
    Status(bool),
    Stats,
}

/// Espera a sessão discada (evento, status ou latência); sem ela nunca resolve.
async fn dial_wake(d: &mut Option<Dial>) -> DialWake {
    let Some(d) = d else {
        return std::future::pending().await;
    };
    tokio::select! {
        ev = d.session.recv_event() => DialWake::Event(ev),
        r = d.status.changed() => DialWake::Status(r.is_ok()),
        r = d.stats.changed() => match r {
            Ok(()) => DialWake::Stats,
            Err(_) => std::future::pending().await,
        },
    }
}

/// Onde parear: `address`; senão o único par descoberto; senão o endereço manual do config.
async fn pair_target(
    address: Option<String>,
    found: &[DiscoveredPeer],
    manual: Option<String>,
) -> Result<SocketAddr, String> {
    let (host, port) = match address
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
    {
        Some(a) => parse_manual_address(&a).map_err(|e| e.to_string())?,
        None => match (found, manual) {
            ([p], _) => (
                p.addrs
                    .first()
                    .cloned()
                    .ok_or("O Mac não informou endereço.")?,
                p.port,
            ),
            (_, Some(m)) => parse_manual_address(&m).map_err(|e| e.to_string())?,
            ([], None) => return Err("Nenhum Mac encontrado: informe o endereço.".into()),
            _ => return Err("Mais de um Mac encontrado: escolha um.".into()),
        },
    };
    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| format!("Não consegui resolver {host}."))?
        .collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs
        .into_iter()
        .next()
        .ok_or_else(|| format!("Não consegui resolver {host}."))
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
