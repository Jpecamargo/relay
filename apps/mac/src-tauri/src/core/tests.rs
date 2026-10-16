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
use relay_core::net::{ListenerConfig, SessionEvent, SessionListener};
use relay_core::platform::{
    CaptureError, ChangeCallback, CursorControl, DisplaySource, InjectError, InputCapture,
    InputInjector, KeyStore, PlatformError,
};
use relay_core::types::{
    CapturedEvent, CommandResult, Config, ConnectionStatus, DisplayInfo, DisplaysPayload, DockInfo,
    DockOrientation, HotCorners, InputEvent, LatencyStats, Machine, Message, PairingEvent,
    PairingState, PermissionKind, PermissionState, PermissionsStatus, Ports, StatusSnapshot,
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

struct MockCapture {
    sink: Sink,
    swallow: Arc<AtomicBool>,
    deny: Arc<AtomicBool>,
}
impl InputCapture for MockCapture {
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

#[derive(Default)]
struct MockCursor {
    log: Arc<Mutex<Vec<String>>>,
}
impl CursorControl for MockCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        self.log.lock().unwrap().push("hide".into());
        Ok(())
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        self.log.lock().unwrap().push("show".into());
        Ok(())
    }
    fn associate(&mut self, a: bool) -> Result<(), PlatformError> {
        self.log.lock().unwrap().push(format!("associate({a})"));
        Ok(())
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        self.log.lock().unwrap().push(format!("warp({x},{y})"));
        Ok(())
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        Ok((0, 0))
    }
}

#[derive(Default, Clone)]
struct MockInjector {
    events: Arc<Mutex<Vec<InputEvent>>>,
}
impl InputInjector for MockInjector {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.events.lock().unwrap().push(*event);
        Ok(())
    }
    fn type_text(&mut self, _text: &str) -> Result<(), InjectError> {
        Ok(())
    }
}

struct MockDisplays {
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

struct MockPlatform {
    _dir: tempfile::TempDir,
    config_path: PathBuf,
    keys: Arc<MemoryKeyStore>,
    sink: Sink,
    swallow: Arc<AtomicBool>,
    deny: Arc<AtomicBool>,
    cursor_log: Arc<Mutex<Vec<String>>>,
    /// Eventos que o injetor do Mac recebeu (sentido inverso, R1).
    injected: Arc<Mutex<Vec<InputEvent>>>,
    changed: Arc<Mutex<Option<ChangeCallback>>>,
    displays: Arc<Mutex<Vec<DisplayInfo>>>,
    sys_cb: SysCb,
    perms: Mutex<PermissionsStatus>,
    opened: Mutex<Vec<PermissionKind>>,
}

impl MockPlatform {
    fn new(cfg: &Config) -> Arc<Self> {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        config::save(&config_path, cfg).unwrap();
        Arc::new(Self {
            _dir: dir,
            config_path,
            keys: Arc::default(),
            sink: Arc::default(),
            swallow: Arc::default(),
            deny: Arc::default(),
            cursor_log: Arc::default(),
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
            swallow: self.swallow.clone(),
            deny: self.deny.clone(),
        }
    }
    fn new_cursor(&self) -> MockCursor {
        MockCursor {
            log: self.cursor_log.clone(),
        }
    }
    fn new_injector(&self) -> MockInjector {
        MockInjector {
            events: self.injected.clone(),
        }
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
    fn permissions(&self, p: &PermissionsStatus) {
        self.permissions.lock().unwrap().push(*p);
    }
    fn secure_input(&self, _active: bool) {}
    fn pairing(&self, e: &PairingEvent) {
        self.pairing.lock().unwrap().push(e.clone());
    }
    fn command_result(&self, _r: &CommandResult) {}
}

// ---- agente falso: SessionListener + Receiver, com runtime próprio ----

struct Agent {
    control_port: u16,
    input_port: u16,
    received: Arc<Mutex<Vec<Message>>>,
    injected: Arc<Mutex<Vec<InputEvent>>>,
    active: Arc<AtomicBool>,
    /// Novo código a abrir (o anterior é de uso único).
    next_code: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Agent {
    fn start(code: &str) -> Self {
        let received: Arc<Mutex<Vec<Message>>> = Arc::default();
        let injected: Arc<Mutex<Vec<InputEvent>>> = Arc::default();
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ports_tx, ports_rx) = std::sync::mpsc::channel();
        let next_code: Arc<Mutex<Option<String>>> = Arc::default();
        let (r, i, a, s, n, code) = (
            received.clone(),
            injected.clone(),
            active.clone(),
            stop.clone(),
            next_code.clone(),
            code.to_owned(),
        );
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(agent_main(code, ports_tx, r, i, a, n, s));
        });
        let (control_port, input_port) = ports_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            control_port,
            input_port,
            received,
            injected,
            active,
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

type AgentReceiver = Receiver<MockInjector, MockCursor>;

async fn agent_main(
    code: String,
    ports_tx: std::sync::mpsc::Sender<(u16, u16)>,
    received: Arc<Mutex<Vec<Message>>>,
    injected: Arc<Mutex<Vec<InputEvent>>>,
    active: Arc<AtomicBool>,
    next_code: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
) {
    let identity = load_or_create_identity(&MemoryKeyStore::default()).unwrap();
    let device = LocalDevice {
        name: "PC de teste".into(),
        public_key: identity.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    };
    let injector = MockInjector {
        events: injected.clone(),
    };
    let receiver: Arc<Mutex<AgentReceiver>> = Arc::new(Mutex::new(Receiver::new(
        &Config::default(),
        vec![dell_display()],
        injector,
        MockCursor::default(),
    )));
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
        tokio::select! {
            ev = listener.recv_event() => match ev {
                None => break,
                Some(SessionEvent::Paired(peer)) => listener.add_peer(peer),
                Some(SessionEvent::Connected(_)) => {
                    let _ = listener.send(Message::Displays(vec![dell_display()])).await;
                }
                Some(SessionEvent::Message(m)) => {
                    received.lock().unwrap().push(m.clone());
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
    let agent = Agent::start("123456");
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
    assert_eq!((shown.mac.len(), shown.windows.len()), (1, 1));
    // O perfil padrão foi persistido para o editor listar.
    assert_eq!(core.config().layout.profiles.len(), 1);
    let on_disk = config::load_or_default(&platform.config_path).unwrap();
    assert_eq!(on_disk.layout.profiles, vec![layout.clone()]);
    assert!(
        rec.displays
            .lock()
            .unwrap()
            .iter()
            .any(|d| d.windows.len() == 1)
    );

    // save_layout: grava, sem avisos (Dock sem autohide).
    let result = core.save_layout(layout).unwrap();
    assert!(result.warnings.is_empty());

    // Cursor até a borda direita: FocusEnter no agente, foco passa ao Windows.
    move_to_edge(&platform, 400);
    wait_until("FocusEnter no agente", 3000, || {
        agent.saw(|m| matches!(m, Message::FocusEnter { .. }))
    })
    .await;
    assert!(agent.active.load(Ordering::SeqCst));
    wait_until("foco no Windows", 2000, || {
        core.status().focus == Machine::Windows
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
        core.status().focus == Machine::Mac
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
