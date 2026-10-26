//! dono: MI1
//! AppCore com plataforma mock contra um agente falso (SessionListener + Receiver do core) em
//! loopback: pareia com código, recebe Displays, manda LayoutSync, e eventos sintéticos levam o
//! cursor à borda direita até virarem FocusEnter e MouseMove no agente.

use std::any::Any;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_core::config;
use relay_core::crypto::{LocalDevice, MemoryKeyStore, load_or_create_identity};
use relay_core::engine::Receiver;
use relay_core::engine::testutil::{MockCursor, MockInjector};
use relay_core::net::{ListenerConfig, SessionEvent, SessionListener};
use relay_core::platform::{
    CaptureError, ChangeCallback, DisplaySource, InjectError, InputCapture, InputInjector,
    KeyStore, LoopWaker, PlatformError,
};
use relay_core::types::{
    CapturedEvent, CommandResult, Config, ConnectionStatus, DisplayInfo, DisplaysPayload, DockInfo,
    DockOrientation, HotCorners, HotkeyAction, HotkeyBinding, InputEvent, KeyCode, KeyCombo,
    LatencyStats, LayoutBook, Machine, MediaAction, Message, Modifier, PairingEvent, PairingState,
    PermissionKind, PermissionState, PermissionsStatus, Ports, RemoteCommand, StatusSnapshot,
};
use tokio::runtime::Handle;

use super::*;
use crate::backend::AppBackend;

mod reverse;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn mac_display() -> DisplayInfo {
    DisplayInfo {
        id: "mac".into(),
        name: "MacBook".into(),
        machine: Machine::Mac,
        origin_px: (0, 0),
        size_px: (1440, 900),
        scale: 2.0,
        size_mm: Some((300.0, 190.0)),
        is_primary: true,
        is_builtin: true,
    }
}

fn dell_display() -> DisplayInfo {
    DisplayInfo {
        id: "dell".into(),
        name: "Dell".into(),
        machine: Machine::Windows,
        origin_px: (0, 0),
        size_px: (2560, 1440),
        scale: 1.0,
        size_mm: Some((597.0, 336.0)),
        is_primary: true,
        is_builtin: false,
    }
}

// ---- plataforma mock do Mac ----

type SysCb = Arc<Mutex<Option<Box<dyn Fn(SysEvent) + Send + Sync>>>>;
type Sink = Arc<Mutex<Option<rtrb::Producer<CapturedEvent>>>>;
type WakerSlot = Arc<Mutex<Option<LoopWaker>>>;

pub(super) struct MockCapture {
    sink: Sink,
    waker: WakerSlot,
    swallow: Arc<AtomicBool>,
    deny: Arc<AtomicBool>,
}
impl InputCapture for MockCapture {
    fn set_waker(&mut self, waker: LoopWaker) {
        *self.waker.lock().unwrap() = Some(waker);
    }
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        if self.deny.load(Ordering::SeqCst) {
            return Err(CaptureError::PermissionDenied);
        }
        *self.sink.lock().unwrap() = Some(sink);
        Ok(())
    }
    fn stop(&mut self) {}
    fn set_swallow(&self, swallow: bool) {
        self.swallow.store(swallow, Ordering::SeqCst);
    }
    fn is_swallowing(&self) -> bool {
        self.swallow.load(Ordering::SeqCst)
    }
}

pub(super) struct MockDisplays {
    changed: Arc<Mutex<Option<ChangeCallback>>>,
    list: Arc<Mutex<Vec<DisplayInfo>>>,
}
impl DisplaySource for MockDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(self.list.lock().unwrap().clone())
    }
    fn set_change_callback(&mut self, callback: ChangeCallback) {
        *self.changed.lock().unwrap() = Some(callback);
    }
}

pub(super) struct MockPlatform {
    _dir: tempfile::TempDir,
    config_path: PathBuf,
    keys: Arc<MemoryKeyStore>,
    sink: Sink,
    /// Acordador que o core entregou à captura; o `push` o aciona como o tap real faria.
    waker: WakerSlot,
    /// Simula uma captura sem waker (laço só vê o evento na próxima volta do relógio).
    no_wake: AtomicBool,
    swallow: Arc<AtomicBool>,
    deny: Arc<AtomicBool>,
    cursor_log: Arc<Mutex<Vec<String>>>,
    /// Posição do cursor do Mac, compartilhada com o injetor (que a desloca nos `MouseMove`).
    cursor_pos: Arc<Mutex<(i32, i32)>>,
    /// Eventos que o injetor do Mac recebeu (sentido inverso, R1).
    injected: Arc<Mutex<Vec<InputEvent>>>,
    changed: Arc<Mutex<Option<ChangeCallback>>>,
    displays: Arc<Mutex<Vec<DisplayInfo>>>,
    sys_cb: SysCb,
    perms: Mutex<PermissionsStatus>,
    opened: Mutex<Vec<PermissionKind>>,
}

impl MockPlatform {
    pub(super) fn new(cfg: &Config) -> Arc<Self> {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        config::save(&config_path, cfg).unwrap();
        Arc::new(Self {
            _dir: dir,
            config_path,
            keys: Arc::default(),
            sink: Arc::default(),
            waker: Arc::default(),
            no_wake: AtomicBool::new(false),
            swallow: Arc::default(),
            deny: Arc::default(),
            cursor_log: Arc::default(),
            cursor_pos: Arc::default(),
            injected: Arc::default(),
            changed: Arc::default(),
            displays: Arc::new(Mutex::new(vec![mac_display()])),
            sys_cb: Arc::default(),
            perms: Mutex::new(PermissionsStatus {
                accessibility: PermissionState::Granted,
                input_monitoring: PermissionState::Granted,
            }),
            opened: Mutex::default(),
        })
    }

    fn push(&self, ev: CapturedEvent) {
        if let Some(p) = self.sink.lock().unwrap().as_mut() {
            let _ = p.push(ev);
        }
        if !self.no_wake.load(Ordering::Relaxed) {
            if let Some(w) = self.waker.lock().unwrap().as_ref() {
                w.wake();
            }
        }
    }
}

struct NoBrowse;
impl Browse for NoBrowse {
    fn next_peer(&self, timeout: Duration) -> Option<relay_core::types::DiscoveredPeer> {
        std::thread::sleep(timeout);
        None
    }
}

impl Platform for MockPlatform {
    type Capture = MockCapture;
    type Cursor = MockCursor;
    type Injector = MockInjector;
    type Displays = MockDisplays;

    fn new_capture(&self) -> MockCapture {
        MockCapture {
            sink: self.sink.clone(),
            waker: self.waker.clone(),
            swallow: self.swallow.clone(),
            deny: self.deny.clone(),
        }
    }
    fn new_cursor(&self) -> MockCursor {
        MockCursor {
            log: self.cursor_log.clone(),
            pos: self.cursor_pos.clone(),
        }
    }
    fn new_injector(&self) -> MockInjector {
        MockInjector::recording(self.injected.clone())
            .moving(self.cursor_pos.clone(), (0, 0, 1440, 900))
    }
    fn new_displays(&self) -> MockDisplays {
        MockDisplays {
            changed: self.changed.clone(),
            list: self.displays.clone(),
        }
    }
    fn key_store(&self) -> Arc<dyn KeyStore> {
        self.keys.clone()
    }
    fn config_path(&self) -> PathBuf {
        self.config_path.clone()
    }
    fn device_name(&self) -> String {
        "Mac de teste".into()
    }
    fn check_permissions(&self) -> PermissionsStatus {
        *self.perms.lock().unwrap()
    }
    fn open_permission_settings(&self, kind: PermissionKind) {
        self.opened.lock().unwrap().push(kind);
    }
    fn dock(&self) -> (DockInfo, HotCorners) {
        (
            DockInfo {
                autohide: false,
                orientation: DockOrientation::Bottom,
            },
            HotCorners::default(),
        )
    }
    fn set_launch_at_login(&self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }
    fn watch_system(&self, callback: Box<dyn Fn(SysEvent) + Send + Sync>) -> Box<dyn Any + Send> {
        *self.sys_cb.lock().unwrap() = Some(callback);
        Box::new(())
    }
    fn watch_secure_input(
        &self,
        _callback: Box<dyn Fn(bool) + Send + Sync>,
    ) -> Box<dyn Any + Send> {
        Box::new(())
    }
    fn browse(&self) -> Result<Box<dyn Browse>, String> {
        Ok(Box::new(NoBrowse))
    }
}

// ---- registro de eventos para a UI ----

#[derive(Default)]
struct Rec {
    statuses: Mutex<Vec<StatusSnapshot>>,
    displays: Mutex<Vec<DisplaysPayload>>,
    permissions: Mutex<Vec<PermissionsStatus>>,
    pairing: Mutex<Vec<PairingEvent>>,
    latency: Mutex<Vec<Instant>>,
    results: Mutex<Vec<CommandResult>>,
    layouts: Mutex<Vec<LayoutBook>>,
}
impl CoreEvents for Rec {
    fn status(&self, s: &StatusSnapshot) {
        self.statuses.lock().unwrap().push(s.clone());
    }
    fn latency(&self, _l: &LatencyStats) {
        self.latency.lock().unwrap().push(Instant::now());
    }
    fn displays(&self, d: &DisplaysPayload) {
        self.displays.lock().unwrap().push(d.clone());
    }
    fn layout(&self, b: &LayoutBook) {
        self.layouts.lock().unwrap().push(b.clone());
    }
    fn permissions(&self, p: &PermissionsStatus) {
        self.permissions.lock().unwrap().push(*p);
    }
    fn secure_input(&self, _active: bool) {}
    fn pairing(&self, e: &PairingEvent) {
        self.pairing.lock().unwrap().push(e.clone());
    }
    fn command_result(&self, r: &CommandResult) {
        self.results.lock().unwrap().push(r.clone());
    }
}

// ---- agente falso: SessionListener + Receiver, com runtime próprio ----

struct Agent {
    control_port: u16,
    input_port: u16,
    received: Arc<Mutex<Vec<Message>>>,
    injected: Arc<Mutex<Vec<InputEvent>>>,
    active: Arc<AtomicBool>,
    /// Liga: o agente recebe `Command` mas não responde (provoca o timeout).
    mute: Arc<AtomicBool>,
    /// Enviado ao Mac assim que o laço do agente o vê.
    outbox: Arc<Mutex<Vec<Message>>>,
    /// Novo código a abrir (o anterior é de uso único).
    next_code: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Agent {
    fn start(code: &str) -> Self {
        Self::start_versioned(code, relay_core::version::APP_VERSION)
    }

    fn start_versioned(code: &str, app_version: &str) -> Self {
        let received: Arc<Mutex<Vec<Message>>> = Arc::default();
        let injected: Arc<Mutex<Vec<InputEvent>>> = Arc::default();
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ports_tx, ports_rx) = std::sync::mpsc::channel();
        let next_code: Arc<Mutex<Option<String>>> = Arc::default();
        let mute = Arc::new(AtomicBool::new(false));
        let outbox: Arc<Mutex<Vec<Message>>> = Arc::default();
        let (r, i, a, s, n, m, code, v, ob) = (
            received.clone(),
            injected.clone(),
            active.clone(),
            stop.clone(),
            next_code.clone(),
            mute.clone(),
            code.to_owned(),
            app_version.to_owned(),
            outbox.clone(),
        );
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(agent_main(
                code,
                v,
                ports_tx,
                r,
                i,
                a,
                AgentCtl {
                    next_code: n,
                    mute: m,
                    outbox: ob,
                },
                s,
            ));
        });
        let (control_port, input_port) = ports_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            control_port,
            input_port,
            received,
            injected,
            active,
            mute,
            outbox,
            next_code,
            stop,
            thread: Some(thread),
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.received.lock().unwrap().iter().any(f)
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Executor do agente falso: todo comando dá certo.
struct OkExecutor;
impl relay_core::platform::CommandExecutor for OkExecutor {
    fn execute(&mut self, id: u32, _c: &RemoteCommand) -> CommandResult {
        CommandResult::ok(
            id,
            Some(relay_core::types::CommandOutput::Text("ok".into())),
        )
    }
}

/// Alças que o teste usa para dirigir o agente falso.
struct AgentCtl {
    next_code: Arc<Mutex<Option<String>>>,
    mute: Arc<AtomicBool>,
    outbox: Arc<Mutex<Vec<Message>>>,
}

type AgentReceiver = Receiver<MockInjector, MockCursor>;

#[allow(clippy::too_many_arguments)]
async fn agent_main(
    code: String,
    app_version: String,
    ports_tx: std::sync::mpsc::Sender<(u16, u16)>,
    received: Arc<Mutex<Vec<Message>>>,
    injected: Arc<Mutex<Vec<InputEvent>>>,
    active: Arc<AtomicBool>,
    ctl: AgentCtl,
    stop: Arc<AtomicBool>,
) {
    let AgentCtl {
        next_code,
        mute,
        outbox,
    } = ctl;
    let identity = load_or_create_identity(&MemoryKeyStore::default()).unwrap();
    let device = LocalDevice {
        name: "PC de teste".into(),
        public_key: identity.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    };
    let injector = MockInjector::recording(injected.clone());
    let receiver: Arc<Mutex<AgentReceiver>> = Arc::new(Mutex::new(Receiver::new(
        &Config::default(),
        vec![dell_display()],
        injector,
        MockCursor::new(&Default::default()),
    )));
    receiver.lock().unwrap().set_executor(Box::new(OkExecutor));
    let fast_rx = receiver.clone();
    let mut cfg = ListenerConfig::new(
        identity,
        device,
        Vec::new(),
        Ports {
            control: 0,
            input: 0,
            ..Ports::default()
        },
    );
    cfg.bind_ip = LO;
    cfg.app_version = app_version;
    let mut listener = SessionListener::start(
        cfg,
        Box::new(move |e| {
            fast_rx.lock().unwrap().on_input(e.event, Instant::now());
        }),
    )
    .await
    .unwrap();
    listener.open_pairing(code, Duration::from_secs(120));
    ports_tx
        .send((
            listener.local_control_addr().port(),
            listener.local_input_addr().port(),
        ))
        .unwrap();
    while !stop.load(Ordering::SeqCst) {
        if let Some(code) = next_code.lock().unwrap().take() {
            listener.open_pairing(code, Duration::from_secs(120));
        }
        let pending: Vec<Message> = outbox.lock().unwrap().drain(..).collect();
        for m in pending {
            let _ = listener.send(m).await;
        }
        tokio::select! {
            ev = listener.recv_event() => match ev {
                None => break,
                Some(SessionEvent::Paired(peer)) => listener.add_peer(peer),
                Some(SessionEvent::Connected(_)) => {
                    let _ = listener.send(Message::Displays(vec![dell_display()])).await;
                }
                Some(SessionEvent::Message(m)) => {
                    received.lock().unwrap().push(m.clone());
                    if mute.load(Ordering::SeqCst) && matches!(m, Message::Command { .. }) {
                        continue;
                    }
                    let replies = receiver.lock().unwrap().on_message(m, Instant::now());
                    for r in replies {
                        let _ = listener.send(r).await;
                    }
                }
                Some(_) => {}
            },
            () = tokio::time::sleep(Duration::from_millis(5)) => {}
        }
        let mut rx = receiver.lock().unwrap();
        rx.tick(Instant::now());
        active.store(rx.is_active(), Ordering::SeqCst);
    }
    listener.shutdown().await;
}

// ---- utilidades ----

async fn wait_until(what: &str, ms: u64, mut cond: impl FnMut() -> bool) {
    let t = Instant::now();
    while !cond() {
        assert!(
            t.elapsed() < Duration::from_millis(ms),
            "não aconteceu em {ms} ms: {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn base_config(agent: &Agent) -> Config {
    let mut cfg = Config::default();
    cfg.network.ports = Ports {
        control: agent.control_port,
        input: agent.input_port,
        ..Ports::default()
    };
    cfg
}

fn options() -> CoreOptions {
    CoreOptions {
        bind_ip: LO,
        local_ports: Some(Ports {
            input: 0,
            ..Ports::default()
        }),
        manual_grace: Duration::ZERO,
    }
}

fn move_to_edge(platform: &MockPlatform, ms: u64) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(ms) {
        platform.push(CapturedEvent::MouseMove {
            dx: 6,
            dy: 0,
            pos: (1439.0, 450.0),
        });
        std::thread::sleep(Duration::from_millis(4));
    }
}

#[test]
fn status_derivado() {
    use ConnectionStatus::*;
    assert_eq!(derive_status(false, false, None), Disconnected);
    assert_eq!(derive_status(false, true, None), Pairing);
    assert_eq!(derive_status(true, false, None), Discovering);
    assert_eq!(derive_status(true, false, Some(Disconnected)), Connecting);
    assert_eq!(derive_status(true, false, Some(Connected)), Connected);
    assert_eq!(
        derive_status(
            true,
            false,
            Some(IncompatibleVersion {
                local: 1,
                remote: 2
            })
        ),
        IncompatibleVersion {
            local: 1,
            remote: 2
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loopback_pareia_conecta_e_leva_o_cursor_ao_agente() {
    // Peer 0.2: recebe LayoutSync (o 0.3 recebe o livro, testado à parte).
    let agent = Agent::start_versioned("123456", "0.2.9");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = AppCore::start(
        platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    );
    assert_eq!(core.status().status, ConnectionStatus::Disconnected);
    wait_until("relay:permissions no início", 2000, || {
        !rec.permissions.lock().unwrap().is_empty()
    })
    .await;

    // Pareamento com código (endereço manual addr:port, como a UI manda).
    let addr = format!("127.0.0.1:{}", agent.control_port);
    assert!(
        core.pair_with_code("000000".into(), Some(addr.clone()))
            .await
            .is_err()
    );
    assert_eq!(core.status().status, ConnectionStatus::Disconnected);
    *agent.next_code.lock().unwrap() = Some("123456".into());
    tokio::time::sleep(Duration::from_millis(50)).await;
    core.pair_with_code("123456".into(), Some(addr))
        .await
        .unwrap();
    let peer_id = String::from_utf8(platform.keys.get("active_peer").unwrap().unwrap()).unwrap();
    assert!(
        platform
            .keys
            .get(&format!("peer:{peer_id}"))
            .unwrap()
            .is_some()
    );
    assert!(
        rec.pairing
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Paired)
    );
    assert!(
        rec.pairing
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Failed)
    );

    // Conecta, recebe Displays do agente e devolve Displays + LayoutSync.
    wait_until("status Connected", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    assert_eq!(core.status().peer_name.as_deref(), Some("PC de teste"));
    wait_until("LayoutSync no agente", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutSync(p) if p.displays.len() == 2))
    })
    .await;
    assert!(agent.saw(|m| matches!(m, Message::Displays(d) if d.len() == 1 && d[0].id == "mac")));
    let layout = core.layout();
    assert_eq!(layout.displays.len(), 2);
    let shown = core.displays();
    assert_eq!((shown.local.len(), shown.peer.len()), (1, 1));
    wait_until("relay:displays com o peer", 3000, || {
        rec.displays
            .lock()
            .unwrap()
            .iter()
            .any(|d| d.peer.len() == 1)
    })
    .await;
    // Conjunto de telas novo: o Mac, host, cria o perfil e grava (regra do I4).
    assert_eq!(core.config().layout.profiles, vec![layout.clone()]);
    assert_eq!(core.layout_book().profiles, vec![layout.clone()]);

    // save_layout: grava como perfil em uso, sem avisos (Dock sem autohide).
    let result = core.save_layout(layout.clone()).unwrap();
    assert!(result.warnings.is_empty());
    let on_disk = config::load_or_default(&platform.config_path).unwrap();
    assert_eq!(on_disk.layout.profiles, vec![layout]);
    assert_eq!(on_disk.layout.active.as_deref(), Some("Principal"));

    // Cursor até a borda direita: FocusEnter no agente, foco passa ao Windows.
    move_to_edge(&platform, 400);
    wait_until("FocusEnter no agente", 3000, || {
        agent.saw(|m| matches!(m, Message::FocusEnter { .. }))
    })
    .await;
    assert!(agent.active.load(Ordering::SeqCst));
    wait_until("foco no Windows", 2000, || {
        core.status().focus == Host::Peer
    })
    .await;
    assert!(platform.swallow.load(Ordering::SeqCst));
    assert!(
        platform
            .cursor_log
            .lock()
            .unwrap()
            .contains(&"hide".to_string())
    );
    assert!(
        platform
            .cursor_log
            .lock()
            .unwrap()
            .contains(&"associate(false)".to_string())
    );

    // Movimento remoto chega ao injetor do agente pelo canal rápido.
    let t = Instant::now();
    loop {
        platform.push(CapturedEvent::MouseMove {
            dx: 7,
            dy: 3,
            pos: (1439.0, 450.0),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let got = agent
            .injected
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, InputEvent::MouseMove { .. }));
        if got {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(4), "MouseMove não chegou");
    }

    // Acordar o Mac derruba o foco remoto (nunca engolir input sem destino) e reconecta.
    {
        let wake = platform.sys_cb.lock().unwrap();
        (wake.as_ref().unwrap())(SysEvent::DidWake);
    }
    wait_until("volta ao Mac na queda", 5000, || {
        core.status().focus == Host::Local
    })
    .await;
    wait_until("reconecta", 10000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    assert!(
        rec.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.status == ConnectionStatus::Reconnecting)
    );

    // relay:latency a no máximo 2 Hz.
    wait_until("relay:latency", 3000, || {
        rec.latency.lock().unwrap().len() >= 2
    })
    .await;
    let lat = rec.latency.lock().unwrap().clone();
    assert!(
        lat.windows(2)
            .all(|w| w[1] - w[0] >= Duration::from_millis(450))
    );

    // Revogar: o agente recebe Unpair e o Keychain fica sem o peer.
    core.unpair().unwrap();
    assert_eq!(core.status().status, ConnectionStatus::Disconnected);
    assert!(platform.keys.get("active_peer").unwrap().is_none());
    wait_until("Unpair no agente", 3000, || {
        agent.saw(|m| matches!(m, Message::Unpair))
    })
    .await;
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sem_permissao_emite_evento_abre_o_painel_certo_e_retoma() {
    let agent = Agent::start("111111");
    let platform = MockPlatform::new(&base_config(&agent));
    *platform.perms.lock().unwrap() = PermissionsStatus {
        accessibility: PermissionState::Granted,
        input_monitoring: PermissionState::Denied,
    };
    platform.deny.store(true, Ordering::SeqCst);
    let rec = Arc::new(Rec::default());
    let core = AppCore::start(
        platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    );
    wait_until("evento de permissões negadas", 2000, || {
        rec.permissions
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.input_monitoring == PermissionState::Denied)
    })
    .await;
    assert_eq!(
        *platform.opened.lock().unwrap(),
        vec![PermissionKind::InputMonitoring]
    );
    assert!(platform.sink.lock().unwrap().is_none());

    // Concedida a permissão, a captura sobe sozinha e o evento é reemitido.
    *platform.perms.lock().unwrap() = PermissionsStatus {
        accessibility: PermissionState::Granted,
        input_monitoring: PermissionState::Granted,
    };
    platform.deny.store(false, Ordering::SeqCst);
    wait_until("captura iniciada", 6000, || {
        platform.sink.lock().unwrap().is_some()
    })
    .await;
    wait_until("evento de permissões concedidas", 1000, || {
        rec.permissions
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.input_monitoring == PermissionState::Granted)
    })
    .await;
    assert_eq!(platform.opened.lock().unwrap().len(), 1);
    core.shutdown();
}

/// Pareia e espera `Connected`.
async fn connected(
    agent: &Agent,
    platform: &Arc<MockPlatform>,
    rec: &Arc<Rec>,
    code: &str,
) -> Arc<AppCore<MockPlatform>> {
    let core = Arc::new(AppCore::start(
        platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    ));
    let addr = format!("127.0.0.1:{}", agent.control_port);
    core.pair_with_code(code.into(), Some(addr)).await.unwrap();
    wait_until("status Connected", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    core
}

fn press(platform: &MockPlatform, vk: u16, down: bool) {
    platform.push(CapturedEvent::Key {
        code: KeyCode::Mac(vk),
        down,
        flags: 0,
        autorepeat: false,
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controles_remotos_hotkeys_wol_timeout_e_reconexao() {
    const CTRL: u16 = 0x3B;
    const OPTION: u16 = 0x3A;
    const KEY_M: u16 = 0x2E;
    const KEY_W: u16 = 0x0D;
    let agent = Agent::start("123456");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "123456").await;

    // send_command: o resultado do PC volta como retorno (fora da thread do runtime).
    let c = core.clone();
    let r = tokio::task::spawn_blocking(move || {
        AppBackend::send_command(
            &*c,
            RemoteCommand::Media {
                action: MediaAction::PlayPause,
            },
        )
    })
    .await
    .unwrap();
    assert!(r.ok, "{r:?}");
    assert!(agent.saw(|m| matches!(
        m,
        Message::Command {
            cmd: RemoteCommand::Media {
                action: MediaAction::PlayPause
            },
            ..
        }
    )));

    // Hotkeys configuráveis: Ctrl+Option+M -> Media PlayPause, sem o painel aberto.
    let combo = |key: &str| KeyCombo {
        modifiers: vec![Modifier::Ctrl, Modifier::Option],
        key: key.into(),
    };
    let mut cfg = core.config();
    cfg.hotkeys = vec![
        HotkeyBinding {
            combo: combo("KeyM"),
            action: HotkeyAction::Command(RemoteCommand::Media {
                action: MediaAction::Next,
            }),
        },
        HotkeyBinding {
            combo: combo("KeyW"),
            action: HotkeyAction::WakePc,
        },
    ];
    core.save_config(cfg).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Wake-on-LAN: a porta 9 local recebe o pacote mágico com o MAC do peer (se o SO deixar
    // abrir a porta e entregar o broadcast; senão só confere que o envio não falha).
    let wol = std::net::UdpSocket::bind("0.0.0.0:9").ok();
    eprintln!(
        "WoL: porta 9 {}",
        if wol.is_some() {
            "aberta"
        } else {
            "indisponível"
        }
    );
    if let Some(s) = &wol {
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    }
    for vk in [CTRL, OPTION] {
        press(&platform, vk, true);
    }
    press(&platform, KEY_W, true);
    press(&platform, KEY_W, false);
    if let Some(s) = &wol {
        let mut buf = [0u8; 128];
        let n = s.recv(&mut buf).expect("pacote de Wake-on-LAN na porta 9");
        assert_eq!(
            &buf[..n],
            &relay_core::wol::magic_packet([1, 2, 3, 4, 5, 6])[..]
        );
    }
    core.inner.wake_pc().unwrap();

    press(&platform, KEY_M, true);
    press(&platform, KEY_M, false);
    for vk in [OPTION, CTRL] {
        press(&platform, vk, false);
    }
    wait_until("Command Next no agente", 3000, || {
        agent.saw(|m| {
            matches!(
                m,
                Message::Command {
                    cmd: RemoteCommand::Media {
                        action: MediaAction::Next
                    },
                    ..
                }
            )
        })
    })
    .await;
    // O resultado da hotkey (sem quem espere) vira relay:command-result.
    wait_until("relay:command-result da hotkey", 3000, || {
        rec.results.lock().unwrap().iter().any(|r| r.ok)
    })
    .await;

    // Timeout: o agente não responde; o dispatcher devolve `timeout` em ~5 s.
    agent.mute.store(true, Ordering::SeqCst);
    let c = core.clone();
    let t = Instant::now();
    let r = tokio::task::spawn_blocking(move || {
        AppBackend::send_command(&*c, RemoteCommand::GetMetrics)
    })
    .await
    .unwrap();
    assert!(!r.ok);
    assert_eq!(r.error.as_deref(), Some("timeout"));
    assert!(t.elapsed() >= Duration::from_secs(4), "{:?}", t.elapsed());
    agent.mute.store(false, Ordering::SeqCst);

    // Reconectar: derruba a sessão e volta a Connected.
    let before = rec.statuses.lock().unwrap().len();
    AppBackend::reconnect(&*core).unwrap();
    wait_until("passa por Reconnecting", 5000, || {
        rec.statuses.lock().unwrap()[before..]
            .iter()
            .any(|s| s.status == ConnectionStatus::Reconnecting)
    })
    .await;
    wait_until("volta a Connected", 10000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    core.shutdown();
}

/// Do evento que cruza a borda até o `FocusEnter` chegar ao agente (loopback, laço do core em
/// foco local). `RELAY_NO_WAKE=1` desliga o acordar por evento da captura (comportamento antigo).
/// Rodar: `cargo test -p relay-mac latencia_da_borda -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "medição"]
async fn latencia_da_borda() {
    let agent = Agent::start("123456");
    let platform = MockPlatform::new(&base_config(&agent));
    if std::env::var("RELAY_NO_WAKE").is_ok() {
        platform.no_wake.store(true, Ordering::Relaxed);
    }
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "123456").await;
    wait_until("LayoutSync no agente", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutSync(p) if p.displays.len() == 2))
    })
    .await;
    let mut us = Vec::new();
    for _ in 0..200 {
        agent.received.lock().unwrap().clear();
        // Deixa o laço ocioso (dormindo) antes de cada cruzamento.
        std::thread::sleep(Duration::from_millis(30));
        let t = Instant::now();
        platform.push(CapturedEvent::MouseMove {
            dx: 20,
            dy: 0,
            pos: (1439.0, 450.0),
        });
        while !agent.saw(|m| matches!(m, Message::FocusEnter { .. })) {
            assert!(
                t.elapsed() < Duration::from_secs(2),
                "FocusEnter não chegou"
            );
            std::hint::spin_loop();
        }
        us.push(t.elapsed().as_micros() as u64);
        core.set_sharing_paused(true);
        wait_until("volta ao Mac", 2000, || core.status().focus == Host::Local).await;
        core.set_sharing_paused(false);
        std::thread::sleep(Duration::from_millis(20));
    }
    us.sort_unstable();
    println!(
        "borda->FocusEnter (n={}): p50 {} us, p95 {} us, max {} us",
        us.len(),
        us[us.len() / 2],
        us[us.len() * 95 / 100],
        us[us.len() - 1]
    );
    core.shutdown();
}

#[test]
fn pc_visto_mas_mudo_so_depois_do_limite_na_primeira_conexao() {
    use ConnectionStatus::*;
    let (curto, longo) = (Some(UNREACHABLE_AFTER / 2), Some(UNREACHABLE_AFTER));
    assert!(!pc_unreachable(Connecting, curto));
    assert!(pc_unreachable(Connecting, longo));
    assert!(!pc_unreachable(Reconnecting, longo));
    assert!(!pc_unreachable(Discovering, None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_traz_a_versao_do_pc_e_quem_esta_atras() {
    use relay_core::types::VersionSync;
    let agent = Agent::start_versioned("111111", "99.0.0");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "111111").await;
    let status = core.status();
    assert_eq!(status.peer_version.as_deref(), Some("99.0.0"));
    assert_eq!(status.version_sync, VersionSync::LocalBehind);
    assert!(!agent.saw(|m| matches!(m, Message::RequestUpdate { .. })));
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pc_antigo_demais_para_o_pedido_fica_para_atualizar_a_mao_sem_pedido() {
    use relay_core::types::VersionSync;
    let agent = Agent::start_versioned("111111", "0.0.1");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "111111").await;
    let status = core.status();
    assert_eq!(status.peer_version.as_deref(), Some("0.0.1"));
    assert_eq!(status.version_sync, VersionSync::PeerBehindManual);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!agent.saw(|m| matches!(m, Message::RequestUpdate { .. })));
    core.shutdown();
}

// ---- layouts pelo LayoutHostState (I10) ----

fn book(host: Machine, revision: u64) -> LayoutBook {
    LayoutBook {
        revision,
        host,
        profiles: Vec::new(),
        active: None,
    }
}

fn some_profile() -> LayoutProfile {
    LayoutProfile {
        key: relay_core::layout::profile_key(&[mac_display(), dell_display()]),
        name: "Mesa".into(),
        displays: Vec::new(),
        windows_detached: false,
    }
}

/// Entrega ao core uma mensagem como se viesse do peer.
fn peer_says(core: &AppCore<MockPlatform>, msg: Message) {
    core.inner.core(CoreCmd::Peer(msg));
}

fn config_with_layout_host(host: Machine) -> Config {
    let mut cfg = Config::default();
    cfg.layout.host = host;
    cfg
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layout_so_leitura_devolve_layout_read_only_e_o_status_reflete() {
    // Host do livro é o PC e ele não está conectado: nada edita.
    let platform = MockPlatform::new(&config_with_layout_host(Machine::Windows));
    let rec = Arc::new(Rec::default());
    let core = AppCore::start(
        platform,
        rec,
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    );
    let st = core.status();
    assert_eq!(st.layout_host, Host::Peer);
    assert!(!st.layout_editable);
    assert_eq!(core.layout_book().host, Machine::Windows);
    let err = |r: Result<(), String>| r.unwrap_err();
    assert_eq!(
        core.save_layout(some_profile()).unwrap_err(),
        "layout_read_only"
    );
    assert_eq!(
        err(core.set_layout_profiles(vec![some_profile()], "Mesa".into())),
        "layout_read_only"
    );
    assert_eq!(err(core.set_layout_host(Host::Local)), "layout_read_only");
    assert!(core.layout_book().profiles.is_empty());
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layout_do_host_local_edita_e_emite_relay_layout() {
    let platform = MockPlatform::new(&Config::default());
    let rec = Arc::new(Rec::default());
    let core = AppCore::start(
        platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    );
    let st = core.status();
    assert_eq!(st.layout_host, Host::Local);
    assert!(st.layout_editable);
    core.save_layout(some_profile()).unwrap();
    let b = core.layout_book();
    assert_eq!(
        (b.revision, b.active.as_deref(), b.profiles.len()),
        (1, Some("Mesa"), 1)
    );
    assert_eq!(rec.layouts.lock().unwrap().last(), Some(&b));
    let on_disk = config::load_or_default(&platform.config_path).unwrap();
    assert_eq!(on_disk.layout.profiles, b.profiles);
    assert_eq!(on_disk.layout.revision, 1);
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_03_recebe_o_livro_cria_perfil_para_telas_novas_e_consome_o_host_do_pareamento() {
    let agent = Agent::start_versioned("123456", "0.3.0");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = AppCore::start(
        platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        options(),
        Handle::current(),
    );
    // Escolha do pareamento novo: o outro computador será o host (D44).
    core.inner
        .mutate(|s| s.pending_layout_host = Some(Host::Peer));
    let addr = format!("127.0.0.1:{}", agent.control_port);
    core.pair_with_code("123456".into(), Some(addr))
        .await
        .unwrap();
    wait_until("status Connected", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    // Peer 0.3: livro, não LayoutSync; e Displays do Mac.
    wait_until("LayoutBookSync no agente", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutBookSync(_)))
    })
    .await;
    assert!(agent.saw(|m| matches!(m, Message::Displays(d) if d.len() == 1 && d[0].id == "mac")));
    // Troca inicial: o livro do peer (o gate 0.2 do rótulo de versão do Mac nos testes impede o
    // agente de mandá-lo pelo fio, então entra direto na fila do core); empate com o mesmo host.
    peer_says(&core, Message::LayoutBookSync(book(Machine::Mac, 0)));
    // Host escolhido no pareamento: o Mac propõe ao peer (LayoutBookEdit com host Windows).
    wait_until("LayoutBookEdit com o host do pareamento", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutBookEdit(b) if b.host == Machine::Windows))
    })
    .await;
    assert_eq!(core.inner.st().pending_layout_host, None);
    assert!(!agent.saw(|m| matches!(m, Message::LayoutSync(_))));

    // O peer vira host: o Mac passa a só-leitura espelhando, e edita por proposta.
    peer_says(
        &core,
        Message::LayoutBookSync(LayoutBook {
            profiles: vec![some_profile()],
            active: Some("Mesa".into()),
            ..book(Machine::Windows, 9)
        }),
    );
    wait_until("livro do peer adotado", 5000, || {
        core.layout_book().revision == 9
    })
    .await;
    assert_eq!(core.status().layout_host, Host::Peer);
    assert!(core.status().layout_editable); // host conectado e simétrico
    assert_eq!(core.config().layout.revision, 9);
    assert!(rec.layouts.lock().unwrap().iter().any(|b| b.revision == 9));
    core.save_layout(some_profile()).unwrap();
    wait_until("proposta de edição ao host", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutBookEdit(b) if b.revision == 9 && b.active.as_deref() == Some("Mesa")))
    })
    .await;
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_local_com_peer_03_cria_perfil_para_conjunto_novo_de_telas() {
    let agent = Agent::start_versioned("123456", "0.3.0");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "123456").await;
    peer_says(&core, Message::LayoutBookSync(book(Machine::Mac, 0)));
    wait_until("perfil criado e publicado ao peer", 5000, || {
        agent.saw(|m| matches!(m, Message::LayoutBookSync(b) if b.profiles.len() == 1))
    })
    .await;
    assert_eq!(core.layout_book().profiles.len(), 1);
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn comando_remoto_recebido_e_respondido_com_unsupported() {
    let agent = Agent::start_versioned("123456", "0.3.0");
    let platform = MockPlatform::new(&base_config(&agent));
    let rec = Arc::new(Rec::default());
    let core = connected(&agent, &platform, &rec, "123456").await;
    agent.outbox.lock().unwrap().push(Message::Command {
        id: 77,
        cmd: RemoteCommand::GetMetrics,
    });
    wait_until("UNSUPPORTED no agente", 5000, || {
        agent.saw(|m| {
            matches!(m, Message::CommandResult { id: 77, result }
                if !result.ok && result.error.as_deref() == Some("unsupported"))
        })
    })
    .await;
    core.shutdown();
}

#[test]
fn acordar_o_par_exige_wol_target() {
    use relay_core::types::Os;
    assert!(wake_allowed(None).is_ok());
    assert!(wake_allowed(Some(Caps::baseline(Os::Windows))).is_ok());
    let sem = Caps {
        os: Os::Windows,
        features: Caps::COMMANDS,
    };
    assert_eq!(wake_allowed(Some(sem)).unwrap_err(), "unsupported");
    assert_eq!(
        wake_allowed(Some(Caps::baseline(Os::Mac))).unwrap_err(),
        "unsupported"
    );
}
