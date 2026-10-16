//! dono: WI1
//! O laço do agente: uma tarefa tokio dona da sessão, do `Receiver` e do guard do ponteiro.

use std::sync::Arc;
use std::time::{Duration, Instant};

use relay_core::crypto::{remove_peer, save_peer};
use relay_core::discovery::Advertiser;
use relay_core::engine::Receiver;
use relay_core::net::{SessionEvent, SessionListener};
use relay_core::platform::{DisplaySource, KeyStore};
use relay_core::types::{
    ConnectionStatus, Machine, Message, PairedPeer, PairingEvent, PairingState, StatusSnapshot,
};
use tokio::sync::{mpsc, watch};
use tokio::time::{Interval, MissedTickBehavior, interval};

use super::media::{MediaOut, MediaPort};
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
}

impl Pump {
    pub(super) fn new(
        deps: AgentDeps,
        shared: Arc<Shared>,
        listener: SessionListener,
        advertiser: Option<Advertiser>,
        events: Arc<dyn UiEvents>,
        peers: Vec<PairedPeer>,
    ) -> Self {
        let local = deps.displays.list().unwrap_or_default();
        let injector = RoutedInjector::new(deps.injector, deps.pipe);
        let mut rx = Receiver::new(&shared.config(), local, injector, BoxCursor(deps.cursor));
        if let Some(ex) = deps.executor {
            rx.set_executor(ex);
        }
        if let Some(h) = deps.engine_events {
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
            }
            let active = self.rx.is_active();
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
            clipboard_sync: self.shared.config().clipboard.enabled,
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
        let mut out = vec![Message::Displays(local)];
        if let Some(exe) = &self.foreground {
            out.push(Message::ForegroundApp { exe: exe.clone() });
        }
        self.send_all(out).await;
        self.publish();
    }

    fn on_paired(&mut self, peer: PairedPeer) {
        self.pairing_until = None;
        let saved = save_peer(&*self.keystore, &peer).and_then(|()| {
            self.keystore
                .set(ACTIVE_PEER_ACCOUNT, peer.device_id.as_bytes())
        });
        if let Err(e) = saved {
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
        let _ = remove_peer(&*self.keystore, &peer.device_id);
        let _ = self.keystore.delete(ACTIVE_PEER_ACCOUNT);
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
            Message::LayoutSync(profile) => self.persist_layout(profile),
            Message::Unpair => {
                self.forget_peer();
                self.rx.focus_lost();
                self.publish();
            }
            _ => {}
        }
        let ignored =
            self.paused && matches!(msg, Message::Input { .. } | Message::FocusEnter { .. });
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
                if paused {
                    self.rx.focus_lost();
                }
                self.publish();
            }
            Cmd::DisplaysChanged => {
                let local = self.displays.list().unwrap_or_default();
                self.rx.set_local_displays(local.clone());
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
