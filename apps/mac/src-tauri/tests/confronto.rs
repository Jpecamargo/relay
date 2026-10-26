//! dono: onda 3 (testes externos)
//! Confronto dos dois apps reais: o `AppCore` do Mac (plataforma fake) e o `AgentCore` do Windows
//! (deps fake) conversam por sockets TCP/UDP de verdade em 127.0.0.1. O mDNS é substituído por
//! um alimentador que "anuncia" o outro lado, com as chaves TXT que cada versão anuncia.
//!
//! Fonte: critérios de I9 a I12 e decisões D41 a D45 do backlog. Nada aqui lê a implementação.

use std::any::Any;
use std::net::{IpAddr, Ipv4Addr, TcpListener, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_agent_lib::backend::AgentBackend;
use relay_agent_lib::core::media::{MediaOut, MediaPort};
use relay_agent_lib::core::{
    AgentCore, AgentDeps, ForegroundSource, PeerBrowse, ReverseDeps, UiEvents,
};
use relay_core::config;
use relay_core::crypto::{
    MemoryKeyStore, ReplayMode, StaticKeypair, UdpAead, device_id_of, load_or_create_identity,
    save_active_peer,
};
use relay_core::engine::testutil::{MockCursor, MockInjector, dell_display, mac_display};
use relay_core::net::AudioLink;
use relay_core::node::session::DIAL_FALLBACK;
use relay_core::platform::{
    CaptureError, ChangeCallback, DisplaySource, InputCapture, KeyStore, LoopWaker, PlatformError,
};
use relay_core::types::{
    AudioState, Caps, CapturedEvent, CommandResult, Config, ConnectionStatus, DiscoveredPeer,
    DisplayInfo, DisplaysPayload, DockInfo, DockOrientation, Host, HotCorners, InputEvent,
    LatencyStats, LayoutBook, LayoutProfile, Machine, PairedPeer, PairingEvent, PairingState,
    PermissionKind, PermissionState, PermissionsStatus, Ports, ServiceStatus, StatusSnapshot,
};
use relay_mac_lib::backend::AppBackend;
use relay_mac_lib::core::{
    AppCore, AudioKeys, Browse, CoreEvents, CoreOptions, MediaHooks, Platform, SessionCtx, SysEvent,
};
use tokio::runtime::Handle;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const V03: &str = "0.3.0";
const V02: &str = "0.2.9";

// ---------------------------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------------------------

async fn wait_until(what: &str, ms: u64, mut cond: impl FnMut() -> bool) {
    let t = Instant::now();
    while !cond() {
        assert!(
            t.elapsed() < Duration::from_millis(ms),
            "não aconteceu em {ms} ms: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Porta de controle `p` livre (TCP) com `p + 1` livre (UDP): o par de portas vizinhas que os dois
/// apps assumem (o UDP de input fica logo depois do controle).
fn free_ports() -> Ports {
    for _ in 0..500 {
        let tcp = TcpListener::bind((LO, 0)).unwrap();
        let p = tcp.local_addr().unwrap().port();
        if p == u16::MAX {
            continue;
        }
        if UdpSocket::bind((LO, p + 1)).is_ok() {
            return Ports {
                control: p,
                input: p + 1,
                ..Ports::default()
            };
        }
    }
    panic!("sem par de portas vizinhas livres");
}

fn identity_in(store: &dyn KeyStore) -> StaticKeypair {
    load_or_create_identity(store).unwrap()
}

fn peer_of(identity: &StaticKeypair, name: &str, mac: [u8; 6]) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&identity.public),
        name: name.into(),
        public_key: identity.public,
        mac_address: mac,
    }
}

/// Keystore novo cujo `device_id` fica abaixo (ou acima) de `other_id`.
fn store_vs(other_id: &str, lower: bool) -> Arc<MemoryKeyStore> {
    loop {
        let s = Arc::new(MemoryKeyStore::default());
        let id = device_id_of(&identity_in(&*s).public);
        if (id.as_str() < other_id) == lower {
            return s;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Lado Mac: plataforma fake + eventos
// ---------------------------------------------------------------------------------------------

type Sink = Arc<Mutex<Option<rtrb::Producer<CapturedEvent>>>>;
type WakerSlot = Arc<Mutex<Option<LoopWaker>>>;
type Pos = Arc<Mutex<(i32, i32)>>;

struct MacCapture {
    sink: Sink,
    waker: WakerSlot,
    swallow: Arc<AtomicBool>,
}
impl InputCapture for MacCapture {
    fn set_waker(&mut self, waker: LoopWaker) {
        *self.waker.lock().unwrap() = Some(waker);
    }
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
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

struct FakeDisplays(Vec<DisplayInfo>);
impl DisplaySource for FakeDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(self.0.clone())
    }
    fn set_change_callback(&mut self, _callback: ChangeCallback) {}
}

struct MacBrowse(Arc<Mutex<Receiver<DiscoveredPeer>>>);
impl Browse for MacBrowse {
    fn next_peer(&self, timeout: Duration) -> Option<DiscoveredPeer> {
        self.0.lock().unwrap().recv_timeout(timeout).ok()
    }
}

struct MacFake {
    _dir: tempfile::TempDir,
    config_path: PathBuf,
    keys: Arc<MemoryKeyStore>,
    sink: Sink,
    waker: WakerSlot,
    swallow: Arc<AtomicBool>,
    cursor_pos: Pos,
    injected: Arc<Mutex<Vec<InputEvent>>>,
    browse: Arc<Mutex<Receiver<DiscoveredPeer>>>,
    advertised: Mutex<Vec<(String, u16, u16)>>,
}

impl MacFake {
    fn push(&self, ev: CapturedEvent) {
        if let Some(p) = self.sink.lock().unwrap().as_mut() {
            let _ = p.push(ev);
        }
        if let Some(w) = self.waker.lock().unwrap().as_ref() {
            w.wake();
        }
    }
    fn injected(&self) -> Vec<InputEvent> {
        self.injected.lock().unwrap().clone()
    }
}

impl Platform for MacFake {
    type Capture = MacCapture;
    type Cursor = MockCursor;
    type Injector = MockInjector;
    type Displays = FakeDisplays;

    fn new_capture(&self) -> MacCapture {
        MacCapture {
            sink: self.sink.clone(),
            waker: self.waker.clone(),
            swallow: self.swallow.clone(),
        }
    }
    fn new_cursor(&self) -> MockCursor {
        MockCursor::at(self.cursor_pos.clone())
    }
    fn new_injector(&self) -> MockInjector {
        MockInjector::recording(self.injected.clone())
            .moving(self.cursor_pos.clone(), (0, 0, 1440, 900))
    }
    fn new_displays(&self) -> FakeDisplays {
        FakeDisplays(vec![mac_display()])
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
        PermissionsStatus {
            accessibility: PermissionState::Granted,
            input_monitoring: PermissionState::Granted,
        }
    }
    fn open_permission_settings(&self, _kind: PermissionKind) {}
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
    fn watch_system(&self, _cb: Box<dyn Fn(SysEvent) + Send + Sync>) -> Box<dyn Any + Send> {
        Box::new(())
    }
    fn watch_secure_input(&self, _cb: Box<dyn Fn(bool) + Send + Sync>) -> Box<dyn Any + Send> {
        Box::new(())
    }
    fn browse(&self) -> Result<Box<dyn Browse>, String> {
        Ok(Box::new(MacBrowse(self.browse.clone())))
    }
    fn advertise(
        &self,
        device_id: &str,
        control_port: u16,
        input_port: u16,
    ) -> Result<Box<dyn Any + Send>, String> {
        self.advertised
            .lock()
            .unwrap()
            .push((device_id.to_owned(), control_port, input_port));
        Ok(Box::new(()))
    }
}

#[derive(Default)]
struct MacRec {
    statuses: Mutex<Vec<StatusSnapshot>>,
    layouts: Mutex<Vec<LayoutBook>>,
    pairing: Mutex<Vec<PairingEvent>>,
    updates: Mutex<Vec<String>>,
}
impl CoreEvents for MacRec {
    fn status(&self, s: &StatusSnapshot) {
        self.statuses.lock().unwrap().push(s.clone());
    }
    fn latency(&self, _l: &LatencyStats) {}
    fn displays(&self, _d: &DisplaysPayload) {}
    fn layout(&self, b: &LayoutBook) {
        self.layouts.lock().unwrap().push(b.clone());
    }
    fn permissions(&self, _p: &PermissionsStatus) {}
    fn secure_input(&self, _a: bool) {}
    fn pairing(&self, e: &PairingEvent) {
        self.pairing.lock().unwrap().push(e.clone());
    }
    fn command_result(&self, _r: &CommandResult) {}
    fn update_available(&self, v: &str) {
        self.updates.lock().unwrap().push(v.to_owned());
    }
}

/// Grava o `SessionCtx` de cada sessão (chaves de áudio do microfone).
#[derive(Default)]
struct MacMedia {
    sessions: Mutex<Vec<SessionCtx>>,
}
impl MediaHooks for MacMedia {
    fn session_up(&self, ctx: SessionCtx) {
        self.sessions.lock().unwrap().push(ctx);
    }
}

struct MacNode {
    platform: Arc<MacFake>,
    rec: Arc<MacRec>,
    media: Arc<MacMedia>,
    core: Option<Arc<AppCore<MacFake>>>,
    version: Option<String>,
    ports: Ports,
}

impl MacNode {
    /// `remote` são as portas do PC (o Mac disca nelas); `ports` são as de escuta do Mac.
    fn new(keys: Arc<MemoryKeyStore>, remote: Ports, tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut cfg = Config::default();
        cfg.network.ports = remote;
        cfg.transition.dwell_ms = 0;
        cfg.transition.edge_resistance_pt = 0.0;
        tweak(&mut cfg);
        config::save(&config_path, &cfg).unwrap();
        let (_tx, rx) = channel();
        // O remetente fica solto: o alimentador cria o seu e troca o receptor por este.
        let platform = Arc::new(MacFake {
            _dir: dir,
            config_path,
            keys,
            sink: Arc::default(),
            waker: Arc::default(),
            swallow: Arc::default(),
            cursor_pos: Arc::default(),
            injected: Arc::default(),
            browse: Arc::new(Mutex::new(rx)),
            advertised: Mutex::default(),
        });
        Self {
            platform,
            rec: Arc::default(),
            media: Arc::default(),
            core: None,
            version: Some(V03.into()),
            ports: free_ports(),
        }
    }

    fn id(&self) -> String {
        device_id_of(&identity_in(&*self.platform.keys).public)
    }

    fn me(&self) -> PairedPeer {
        peer_of(&identity_in(&*self.platform.keys), "Mac de teste", [0; 6])
    }

    fn start(&mut self) {
        let core = Arc::new(AppCore::start(
            self.platform.clone(),
            self.rec.clone(),
            self.media.clone() as Arc<dyn MediaHooks>,
            CoreOptions {
                bind_ip: LO,
                local_ports: Some(self.ports.clone()),
                manual_grace: Duration::ZERO,
                app_version: self.version.clone(),
            },
            Handle::current(),
        ));
        self.core = Some(core);
    }

    fn stop(&mut self) {
        if let Some(c) = self.core.take() {
            c.shutdown();
        }
    }

    /// Portas do PC que o Mac usa ao discar (a 0.2 as lê do config).
    fn set_remote_ports(&self, remote: Ports) {
        let mut cfg = config::load_or_default(&self.platform.config_path).unwrap();
        cfg.network.ports = remote;
        config::save(&self.platform.config_path, &cfg).unwrap();
    }

    fn core(&self) -> &Arc<AppCore<MacFake>> {
        self.core.as_ref().expect("Mac iniciado")
    }

    fn status(&self) -> StatusSnapshot {
        self.core().status()
    }

    fn connected(&self) -> bool {
        self.core
            .as_ref()
            .is_some_and(|c| c.status().status == ConnectionStatus::Connected)
    }

    /// Como o comando IPC `pair_with_code`: guarda o host e disca.
    async fn type_code(
        &self,
        code: &str,
        pc_ports: Ports,
        layout_host: Host,
    ) -> Result<(), String> {
        let c = self.core();
        c.set_pending_layout_host(Some(layout_host));
        AppBackend::pair_with_code(
            &**c,
            code.to_owned(),
            Some(format!("127.0.0.1:{}", pc_ports.control)),
        )
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Lado Windows: AgentDeps fake + eventos
// ---------------------------------------------------------------------------------------------

type Events = Arc<Mutex<Vec<InputEvent>>>;
type RevSink = Arc<Mutex<Option<rtrb::Producer<CapturedEvent>>>>;

struct RevCap {
    sink: RevSink,
    swallow: Arc<AtomicBool>,
}
impl InputCapture for RevCap {
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
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

struct PcBrowse(Arc<Mutex<Receiver<DiscoveredPeer>>>);
impl PeerBrowse for PcBrowse {
    fn next_peer(&mut self, timeout: Duration) -> Option<DiscoveredPeer> {
        self.0.lock().unwrap().recv_timeout(timeout).ok()
    }
}

struct NoForeground;
impl ForegroundSource for NoForeground {
    fn start(&mut self, _on_change: Box<dyn Fn(String) + Send + Sync>) {}
}

#[derive(Default)]
struct PcRec {
    statuses: Mutex<Vec<StatusSnapshot>>,
    layouts: Mutex<Vec<LayoutBook>>,
    pairing: Mutex<Vec<PairingEvent>>,
    updates: Mutex<Vec<String>>,
}
impl UiEvents for PcRec {
    fn status(&self, s: &StatusSnapshot) {
        self.statuses.lock().unwrap().push(s.clone());
    }
    fn latency(&self, _s: &LatencyStats) {}
    fn pairing(&self, e: &PairingEvent) {
        self.pairing.lock().unwrap().push(e.clone());
    }
    fn layout(&self, b: &LayoutBook) {
        self.layouts.lock().unwrap().push(b.clone());
    }
    fn displays(&self, _d: &DisplaysPayload) {}
    fn update_requested(&self, v: &str) {
        self.updates.lock().unwrap().push(v.to_owned());
    }
}

/// Grava o `AudioLink` (chaves do microfone) de cada sessão.
#[derive(Clone, Default)]
struct PcMedia(Arc<Mutex<Vec<Option<AudioLink>>>>);
impl MediaPort for PcMedia {
    fn link_changed(&mut self, link: Option<AudioLink>) {
        self.0.lock().unwrap().push(link);
    }
    fn config_changed(&mut self, _config: &Config) {}
    fn audio_state(&self) -> AudioState {
        AudioState::default()
    }
    fn clipboard_send_now(&mut self) {}
    fn take_outbox(&mut self) -> Option<tokio::sync::mpsc::UnboundedReceiver<MediaOut>> {
        None
    }
}

struct PcNode {
    _dir: tempfile::TempDir,
    config_path: PathBuf,
    store: Arc<MemoryKeyStore>,
    rec: Arc<PcRec>,
    media: PcMedia,
    injected: Events,
    engine_log: Arc<Mutex<Vec<String>>>,
    cursor: Pos,
    rev_sink: RevSink,
    rev_swallow: Arc<AtomicBool>,
    browse: Arc<Mutex<Receiver<DiscoveredPeer>>>,
    core: Option<AgentCore>,
    version: Option<String>,
    ports: Ports,
}

impl PcNode {
    fn new(store: Arc<MemoryKeyStore>, tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        // UDP de input do PC efêmero (0), como nos testes do agente: o PC disca no UDP vizinho do
        // controle do Mac. O Mac disca nas portas reais do PC, que `World::start_both` grava.
        let mut ports = free_ports();
        ports.input = 0;
        let mut cfg = Config::default();
        cfg.network.ports = ports.clone();
        cfg.transition.dwell_ms = 0;
        cfg.transition.edge_resistance_pt = 0.0;
        tweak(&mut cfg);
        config::save(&config_path, &cfg).unwrap();
        let (_tx, rx) = channel();
        Self {
            _dir: dir,
            config_path,
            store,
            rec: Arc::default(),
            media: PcMedia::default(),
            injected: Arc::default(),
            engine_log: Arc::default(),
            cursor: Arc::default(),
            rev_sink: Arc::default(),
            rev_swallow: Arc::default(),
            browse: Arc::new(Mutex::new(rx)),
            core: None,
            version: Some(V03.into()),
            ports,
        }
    }

    fn id(&self) -> String {
        device_id_of(&identity_in(&*self.store).public)
    }

    fn me(&self) -> PairedPeer {
        peer_of(&identity_in(&*self.store), "dell-pc", [9, 8, 7, 6, 5, 4])
    }

    fn start(&mut self) {
        let cfg = config::load_or_default(&self.config_path).unwrap();
        let deps = AgentDeps {
            keystore: self.store.clone(),
            injector: Box::new(
                MockInjector::recording(self.injected.clone())
                    .moving(self.cursor.clone(), (0, 0, 2560, 1440)),
            ),
            pipe: None,
            cursor: Box::new(MockCursor::at(self.cursor.clone())),
            displays: Box::new(FakeDisplays(vec![dell_display()])),
            foreground: Some(Box::new(NoForeground)),
            executor: None,
            pointer_guard: Box::new(|| Box::new(())),
            service_probe: Box::new(|helper_active| ServiceStatus {
                installed: true,
                running: true,
                helper_active,
                vbcable_detected: false,
            }),
            device_name: "dell-pc".into(),
            mac_address: [9, 8, 7, 6, 5, 4],
            config_path: Some(self.config_path.clone()),
            bind_ip: LO,
            advertise: false,
            browse: Some({
                let rx = self.browse.clone();
                Box::new(move || Ok(Box::new(PcBrowse(rx.clone())) as Box<dyn PeerBrowse>))
            }),
            message_hook: None,
            engine_events: Some({
                let log = self.engine_log.clone();
                Box::new(move |e| log.lock().unwrap().push(format!("{e:?}")))
            }),
            media: Some(Box::new(self.media.clone())),
            app_version: self.version.clone(),
        };
        let reverse = ReverseDeps {
            capture: Box::new(RevCap {
                sink: self.rev_sink.clone(),
                swallow: self.rev_swallow.clone(),
            }),
            cursor: Box::new(MockCursor::at(Arc::default())),
            injector: Box::new(MockInjector::recording(Events::default())),
        };
        let core =
            AgentCore::start_with_reverse(deps, cfg, self.rec.clone(), Some(reverse)).unwrap();
        self.ports.input = core.local_addrs().1.port();
        self.core = Some(core);
    }

    fn stop(&mut self) {
        if let Some(c) = self.core.take() {
            c.shutdown();
        }
    }

    fn core(&self) -> &AgentCore {
        self.core.as_ref().expect("PC iniciado")
    }

    fn status(&self) -> StatusSnapshot {
        self.core().status()
    }

    fn connected(&self) -> bool {
        self.core
            .as_ref()
            .is_some_and(|c| c.status().status == ConnectionStatus::Connected)
    }

    fn injected(&self) -> Vec<InputEvent> {
        self.injected.lock().unwrap().clone()
    }

    fn push_rev(&self, ev: CapturedEvent) {
        if let Some(p) = self.rev_sink.lock().unwrap().as_mut() {
            let _ = p.push(ev);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Alimentador de mDNS (reanuncia o outro lado a cada 1,5 s, como o mDNS real repete)
// ---------------------------------------------------------------------------------------------

struct Feeder {
    stop: Arc<AtomicBool>,
}
impl Feeder {
    fn start(tx: Sender<DiscoveredPeer>, peer: DiscoveredPeer) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                if tx.send(peer.clone()).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
        });
        Self { stop }
    }
}
impl Drop for Feeder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn announce(
    name: &str,
    id: &str,
    ports: Ports,
    os: Option<relay_core::types::Os>,
    version: Option<&str>,
) -> DiscoveredPeer {
    DiscoveredPeer {
        name: name.into(),
        addrs: vec!["127.0.0.1".into()],
        port: ports.control,
        device_id: id.into(),
        protocol_version: relay_core::types::consts::PROTOCOL_VERSION,
        os,
        app_version: version.map(Into::into),
    }
}

// ---------------------------------------------------------------------------------------------
// O par: Mac e PC reais, com o par 0.2 já salvo (ou não) nos dois keystores
// ---------------------------------------------------------------------------------------------

struct World {
    mac: MacNode,
    pc: PcNode,
    feeders: Vec<Feeder>,
}

/// Quem tem o menor `device_id` e quais lados "enxergam" o outro pelo mDNS.
struct Setup {
    mac_lower: bool,
    saved_pair: bool,
    /// Mac enxerga o PC / PC enxerga o Mac no mDNS.
    mac_sees: bool,
    pc_sees: bool,
    /// `layout.host` gravado nos dois configs (a migração da 0.2 deixa Mac).
    host: Machine,
}

impl Setup {
    fn new(mac_lower: bool) -> Self {
        Self {
            mac_lower,
            saved_pair: true,
            mac_sees: true,
            pc_sees: true,
            host: Machine::Mac,
        }
    }
}

fn build(s: &Setup) -> World {
    let mac_keys = Arc::new(MemoryKeyStore::default());
    let mac_id = device_id_of(&identity_in(&*mac_keys).public);
    let pc_store = store_vs(&mac_id, !s.mac_lower);
    let host = s.host;
    let mut pc = PcNode::new(pc_store, |c| c.layout.host = host);
    let mac = MacNode::new(mac_keys, pc.ports.clone(), |c| c.layout.host = host);
    if s.saved_pair {
        // Formato da 0.2: identidade + `active_peer`/`active-peer` + `peer:<id>`.
        save_active_peer(&*mac.platform.keys, "active_peer", &pc.me()).unwrap();
        save_active_peer(&*pc.store, "active-peer", &mac.me()).unwrap();
    }
    pc.version = Some(V03.into());
    let mut w = World {
        mac,
        pc,
        feeders: vec![],
    };
    use relay_core::types::Os;
    if s.mac_sees {
        let (tx, rx) = channel();
        *w.mac.platform.browse.lock().unwrap() = rx;
        w.feeders.push(Feeder::start(
            tx,
            announce(
                "dell-pc",
                &w.pc.id(),
                w.pc.ports.clone(),
                Some(Os::Windows),
                Some(V03),
            ),
        ));
    }
    if s.pc_sees {
        let (tx, rx) = channel();
        *w.pc.browse.lock().unwrap() = rx;
        w.feeders.push(Feeder::start(
            tx,
            announce(
                "Mac de teste",
                &w.mac.id(),
                w.mac.ports.clone(),
                Some(Os::Mac),
                Some(V03),
            ),
        ));
    }
    w
}

impl World {
    fn start_both(&mut self) {
        self.pc.start();
        self.mac.set_remote_ports(self.pc.ports.clone());
        self.mac.start();
    }

    async fn connected(&self, ms: u64) {
        wait_until("Mac e PC Connected", ms, || {
            self.mac.connected() && self.pc.connected()
        })
        .await;
    }

    /// Os dois livros iguais e com perfil (o host cria o perfil do conjunto novo de telas).
    async fn converged(&self) -> LayoutBook {
        let t = Instant::now();
        loop {
            let (a, b) = (
                self.mac.core().layout_book(),
                self.pc.core().get_layout_book(),
            );
            if !a.profiles.is_empty() && a == b {
                return a;
            }
            assert!(
                t.elapsed() < Duration::from_secs(8),
                "livros não convergiram em 8 s (layout_host mac={:?} pc={:?}):\n mac = {a:#?}\n pc  = {b:#?}",
                self.mac.status().layout_host,
                self.pc.status().layout_host
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.mac.stop();
        self.pc.stop();
    }
}

fn caps_baseline(os: relay_core::types::Os) -> Caps {
    Caps::baseline(os)
}

// ---------------------------------------------------------------------------------------------
// Cenário 1: par 0.2 salvo nos dois keystores, os dois em 0.3
// ---------------------------------------------------------------------------------------------

async fn par_salvo_conecta(mac_lower: bool) {
    let mut w = build(&Setup::new(mac_lower));
    w.start_both();
    // O de menor id disca na hora (D41): bem antes do fallback de 10 s.
    w.connected(DIAL_FALLBACK.as_millis() as u64 - 4000).await;

    let (ms, ps) = (w.mac.status(), w.pc.status());
    assert_eq!(ms.peer_os, Some(relay_core::types::Os::Windows));
    assert_eq!(ps.peer_os, Some(relay_core::types::Os::Mac));
    // Caps nos dois sentidos (I9/I11): cada lado conhece o SO e os bits do outro.
    wait_until("Caps do outro lado nos dois apps", 5000, || {
        let (m, p) = (w.mac.status().peer_caps, w.pc.status().peer_caps);
        m.is_some_and(|c| c.os == relay_core::types::Os::Windows && c.has(Caps::WOL_TARGET))
            && p.is_some_and(|c| c.os == relay_core::types::Os::Mac && c.has(Caps::MIC_SOURCE))
    })
    .await;
    // Sem re-parear: nenhum evento de pareamento nos dois lados.
    assert!(w.mac.rec.pairing.lock().unwrap().is_empty());
    assert!(w.pc.rec.pairing.lock().unwrap().is_empty());

    // Uma sessão só: passado o fallback do outro lado, ninguém cai nem religa.
    let marca = (
        w.mac.rec.statuses.lock().unwrap().len(),
        w.pc.rec.statuses.lock().unwrap().len(),
    );
    tokio::time::sleep(DIAL_FALLBACK + Duration::from_secs(3)).await;
    assert!(w.mac.connected() && w.pc.connected());
    let flap = |v: &Mutex<Vec<StatusSnapshot>>, from: usize| {
        v.lock().unwrap()[from..]
            .iter()
            .any(|s| s.status != ConnectionStatus::Connected)
    };
    assert!(
        !flap(&w.mac.rec.statuses, marca.0),
        "o Mac saiu de Connected depois de conectar (sessão duplicada?)"
    );
    assert!(
        !flap(&w.pc.rec.statuses, marca.1),
        "o PC saiu de Connected depois de conectar (sessão duplicada?)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn par_02_salvo_conecta_sem_repareamento_mac_de_id_menor() {
    par_salvo_conecta(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn par_02_salvo_conecta_sem_repareamento_pc_de_id_menor() {
    par_salvo_conecta(false).await;
}

/// D41: só o lado de maior id enxerga o outro no mDNS, então só disca depois do fallback.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn so_o_de_maior_id_enxerga_e_disca_depois_do_fallback() {
    for mac_lower in [true, false] {
        let mut s = Setup::new(mac_lower);
        // Só o de maior id vê o outro.
        s.mac_sees = !mac_lower;
        s.pc_sees = mac_lower;
        let mut w = build(&s);
        let t = Instant::now();
        w.start_both();
        w.connected(20_000).await;
        assert!(
            t.elapsed() >= DIAL_FALLBACK - Duration::from_secs(1),
            "o de maior id discou em {:?}, antes do fallback de {DIAL_FALLBACK:?} (mac_lower={mac_lower})",
            t.elapsed()
        );
    }
}

/// D41: só o lado de menor id enxerga o outro: disca na hora.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn so_o_de_menor_id_enxerga_e_disca_na_hora() {
    for mac_lower in [true, false] {
        let mut s = Setup::new(mac_lower);
        s.mac_sees = mac_lower;
        s.pc_sees = !mac_lower;
        let mut w = build(&s);
        w.start_both();
        w.connected(DIAL_FALLBACK.as_millis() as u64 - 3000).await;
    }
}

// ---- o cursor cruza nos dois sentidos, com qualquer um discando (risco 5b) ----

/// Retângulos do livro: devolve (pc_à_direita, y em px do Mac, y em px do PC) numa altura comum.
fn shared_heights(book: &LayoutBook) -> (bool, f64, f64) {
    let p = book
        .profiles
        .iter()
        .find(|p| Some(&p.name) == book.active.as_ref())
        .unwrap_or(&book.profiles[0]);
    let mac = p
        .displays
        .iter()
        .find(|d| d.machine == Machine::Mac)
        .expect("tela do Mac");
    let pc = p
        .displays
        .iter()
        .find(|d| d.machine == Machine::Windows)
        .expect("tela do PC");
    let top = mac.rect_mm.y.max(pc.rect_mm.y);
    let bottom = (mac.rect_mm.y + mac.rect_mm.h).min(pc.rect_mm.y + pc.rect_mm.h);
    assert!(bottom > top, "perfil padrão sem trecho de borda comum");
    let mid = (top + bottom) / 2.0;
    let mac_y = (mid - mac.rect_mm.y) / mac.rect_mm.h * 900.0;
    let pc_y = (mid - pc.rect_mm.y) / pc.rect_mm.h * 1440.0;
    (
        pc.rect_mm.x > mac.rect_mm.x,
        f64::from(mac_y),
        f64::from(pc_y),
    )
}

/// Sobe o par, espera o livro convergir e devolve (mundo, pc_à_direita, y do Mac, y do PC).
async fn par_pronto(mac_lower: bool) -> (World, bool, f64, f64) {
    let mut w = build(&Setup::new(mac_lower));
    w.start_both();
    w.connected(8000).await;
    let book = w.converged().await;
    let (pc_right, mac_y, pc_y) = shared_heights(&book);
    (w, pc_right, mac_y, pc_y)
}

async fn cursor_mac_para_pc(mac_lower: bool) {
    let (w, pc_right, mac_y, _) = par_pronto(mac_lower).await;
    let (edge_x, dx) = if pc_right { (1439.0, 6) } else { (0.0, -6) };
    let t = Instant::now();
    while w.mac.status().focus != Host::Peer && t.elapsed() < Duration::from_secs(5) {
        w.mac.platform.push(CapturedEvent::MouseMove {
            dx,
            dy: 0,
            pos: (edge_x, mac_y),
        });
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
    assert_eq!(w.mac.status().focus, Host::Peer, "o foco não passou ao PC");
    let t = Instant::now();
    while !w.pc.injected().iter().any(|e| {
        matches!(
            e,
            InputEvent::MouseMove { .. } | InputEvent::MouseAbs { .. }
        )
    }) {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "o PC não injetou o movimento do Mac pelo UDP de input (mac_lower={mac_lower}); injetado = {:?}; engine do PC = {:?}",
            w.pc.injected(),
            w.pc.engine_log.lock().unwrap(),
        );
        w.mac.platform.push(CapturedEvent::MouseMove {
            dx: 5 * dx.signum(),
            dy: 0,
            pos: (edge_x, mac_y),
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Volta ao Mac empurrando para o lado oposto.
    let t = Instant::now();
    while w.mac.status().focus != Host::Local && t.elapsed() < Duration::from_secs(8) {
        w.mac.platform.push(CapturedEvent::MouseMove {
            dx: -8 * dx.signum(),
            dy: 0,
            pos: (700.0, mac_y),
        });
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
    assert_eq!(
        w.mac.status().focus,
        Host::Local,
        "o foco não voltou ao Mac"
    );
}

async fn cursor_pc_para_mac(mac_lower: bool) {
    let (w, pc_right, _, pc_y) = par_pronto(mac_lower).await;
    let (pc_edge_x, pdx) = if pc_right { (0.0, -5) } else { (2559.0, 5) };
    let t = Instant::now();
    while w.mac.platform.injected().is_empty() {
        assert!(
            t.elapsed() < Duration::from_secs(6),
            "o Mac não recebeu o movimento do PC (sentido inverso, mac_lower={mac_lower})"
        );
        w.pc.push_rev(CapturedEvent::MouseMove {
            dx: pdx,
            dy: 0,
            pos: (pc_edge_x, pc_y),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_do_mac_ao_pc_com_o_mac_discando() {
    cursor_mac_para_pc(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_do_mac_ao_pc_com_o_pc_discando() {
    cursor_mac_para_pc(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_do_pc_ao_mac_com_o_mac_discando() {
    cursor_pc_para_mac(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cursor_do_pc_ao_mac_com_o_pc_discando() {
    cursor_pc_para_mac(false).await;
}

// ---- risco 5a: chaves de áudio com qualquer um discando ----

async fn chaves_de_audio(mac_lower: bool) {
    let mut w = build(&Setup::new(mac_lower));
    w.start_both();
    w.connected(8000).await;
    wait_until("chaves de áudio nos dois lados", 5000, || {
        w.mac
            .media
            .sessions
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.audio_keys.is_some())
            && w.pc.media.0.lock().unwrap().iter().any(Option::is_some)
    })
    .await;
    let mac_keys: AudioKeys = w
        .mac
        .media
        .sessions
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|c| c.audio_keys.clone())
        .unwrap();
    let pc_link =
        w.pc.media
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(Clone::clone)
            .unwrap();

    // Mac -> PC (microfone): o que o Mac cifra o PC decifra.
    let header = [7u8; 17];
    let mac_tx = UdpAead::new(mac_keys.mac_to_pc.clone(), ReplayMode::Window64);
    let mut pc_rx = UdpAead::new(pc_link.mac_to_pc.clone(), ReplayMode::Window64);
    let (mut sealed, mut opened) = (Vec::new(), Vec::new());
    mac_tx
        .seal(1, &header, b"pcm do microfone", &mut sealed)
        .unwrap();
    pc_rx
        .open(1, &header, &sealed, &mut opened)
        .expect("o PC não decifra o áudio que o Mac cifrou (chaves cruzadas)");
    assert_eq!(opened, b"pcm do microfone");
    // As duas pontas não podem usar a mesma direção de nonce.
    assert_ne!(mac_keys.mac_to_pc.send_dir, pc_link.mac_to_pc.send_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaves_de_audio_batem_com_o_mac_discando() {
    chaves_de_audio(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaves_de_audio_batem_com_o_pc_discando() {
    chaves_de_audio(false).await;
}

// ---- risco 5c: "Acordar" visível offline ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_caps_vem_do_par_salvo_mesmo_desconectado() {
    use relay_core::types::Os;
    let mut s = Setup::new(true);
    s.mac_sees = false;
    s.pc_sees = false;
    let mut w = build(&s);
    w.start_both();
    wait_until("peer_caps preenchido offline nos dois apps", 5000, || {
        w.mac.status().peer_caps.is_some() && w.pc.status().peer_caps.is_some()
    })
    .await;
    assert!(!w.mac.connected() && !w.pc.connected());
    let (m, p) = (w.mac.status(), w.pc.status());
    assert_eq!(m.peer_caps, Some(caps_baseline(Os::Windows)));
    assert_eq!(m.peer_os, Some(Os::Windows));
    assert_eq!(p.peer_caps, Some(caps_baseline(Os::Mac)));
    assert_eq!(p.peer_os, Some(Os::Mac));
    // "Acordar" só no Mac (o PC é alvo de WoL; o Mac não é).
    assert!(m.peer_caps.unwrap().has(Caps::WOL_TARGET));
    assert!(!p.peer_caps.unwrap().has(Caps::WOL_TARGET));
    // O botão "Acordar" do Mac funciona offline (WOL_TARGET veio do par salvo).
    assert_eq!(AppBackend::wake_peer(&**w.mac.core()), Ok(()));
}

// ---------------------------------------------------------------------------------------------
// Cenário 2: pareamento novo pelos dois modos
// ---------------------------------------------------------------------------------------------

/// `pc_shows`: o PC mostra o código e o Mac digita; senão, o contrário.
/// `typist_choice`: o valor de "Guardar os layouts em" (relativo a quem digita).
async fn pareia(pc_shows: bool, typist_choice: Host) {
    let mut s = Setup::new(true);
    s.saved_pair = false;
    s.mac_sees = false;
    s.pc_sees = false;
    let mut w = build(&s);
    w.start_both();

    if pc_shows {
        let code = AgentBackend::new_pairing_code(w.pc.core()).unwrap();
        assert_eq!(code.code.len(), 6);
        w.mac
            .type_code(&code.code, w.pc.ports.clone(), typist_choice)
            .await
            .expect("o Mac não pareou com o código do PC");
    } else {
        // O Mac sobe a escuta de forma assíncrona: tenta até ela abrir.
        let t = Instant::now();
        let code = loop {
            match w.mac.core().new_pairing_code() {
                Ok(c) => break c,
                Err(e) => {
                    assert!(t.elapsed() < Duration::from_secs(5), "Mac não escuta: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        assert_eq!(code.code.len(), 6);
        AgentBackend::pair_with_code(
            w.pc.core(),
            code.code.clone(),
            Some(format!("127.0.0.1:{}", w.mac.ports.control)),
            typist_choice,
        )
        .expect("o PC não pareou com o código do Mac");
    }
    w.connected(10_000).await;

    // Quem digita escolheu o host em relação a si.
    let typist_is_mac = pc_shows;
    let host_is_mac = (typist_choice == Host::Local) == typist_is_mac;
    let expect_host = if host_is_mac {
        Machine::Mac
    } else {
        Machine::Windows
    };
    let book = w.converged().await;
    assert_eq!(
        book.host, expect_host,
        "o livro não guardou a escolha do host"
    );
    let (ms, ps) = (w.mac.status(), w.pc.status());
    assert_eq!(
        ms.layout_host,
        if host_is_mac { Host::Local } else { Host::Peer }
    );
    assert_eq!(
        ps.layout_host,
        if host_is_mac { Host::Peer } else { Host::Local }
    );
    assert!(
        ms.layout_editable && ps.layout_editable,
        "os dois editam conectados"
    );
    // O pareamento acabou de guardar a chave em cada lado (keystore 0.2 compatível).
    assert!(w.mac.platform.keys.get("active_peer").unwrap().is_some());
    assert!(w.pc.store.get("active-peer").unwrap().is_some());
    // A UI do lado que mostra o código recebe `paired`.
    let eventos = |v: &Mutex<Vec<PairingEvent>>| {
        v.lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Paired)
    };
    wait_until("evento paired nos dois lados", 3000, || {
        eventos(&w.mac.rec.pairing) && eventos(&w.pc.rec.pairing)
    })
    .await;
    // Caps trocadas no par novo.
    wait_until("Caps nos dois lados", 5000, || {
        w.mac
            .status()
            .peer_caps
            .is_some_and(|c| c.os == relay_core::types::Os::Windows)
            && w.pc
                .status()
                .peer_caps
                .is_some_and(|c| c.os == relay_core::types::Os::Mac)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pc_mostra_mac_digita_host_neste_computador() {
    pareia(true, Host::Local).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pc_mostra_mac_digita_host_no_outro() {
    pareia(true, Host::Peer).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_mostra_pc_digita_host_neste_computador() {
    pareia(false, Host::Local).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_mostra_pc_digita_host_no_outro() {
    pareia(false, Host::Peer).await;
}

// ---------------------------------------------------------------------------------------------
// Cenário 3: edição no não-host, Sync de volta, desconexão e reconexão
// ---------------------------------------------------------------------------------------------

fn sensitivity_of(book: &LayoutBook, machine: Machine) -> f32 {
    let p = book
        .profiles
        .iter()
        .find(|p| Some(&p.name) == book.active.as_ref())
        .unwrap_or(&book.profiles[0]);
    p.displays
        .iter()
        .find(|d| d.machine == machine)
        .unwrap()
        .sensitivity
}

fn with_sensitivity(book: &LayoutBook, machine: Machine, value: f32) -> LayoutProfile {
    let mut p = book
        .profiles
        .iter()
        .find(|p| Some(&p.name) == book.active.as_ref())
        .unwrap_or(&book.profiles[0])
        .clone();
    for d in p.displays.iter_mut().filter(|d| d.machine == machine) {
        d.sensitivity = value;
    }
    p
}

impl World {
    fn book(&self, m: Machine) -> LayoutBook {
        match m {
            Machine::Mac => self.mac.core().layout_book(),
            Machine::Windows => self.pc.core().get_layout_book(),
        }
    }

    fn editable(&self, m: Machine) -> bool {
        match m {
            Machine::Mac => self.mac.status().layout_editable,
            Machine::Windows => self.pc.status().layout_editable,
        }
    }

    fn edit(&self, m: Machine, p: LayoutProfile) -> Result<(), String> {
        match m {
            Machine::Mac => self.mac.core().save_layout(p).map(|_| ()),
            Machine::Windows => AgentBackend::save_layout(self.pc.core(), p)
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    fn stop(&mut self, m: Machine) {
        match m {
            Machine::Mac => self.mac.stop(),
            Machine::Windows => self.pc.stop(),
        }
    }

    fn start(&mut self, m: Machine) {
        match m {
            Machine::Mac => self.mac.start(),
            Machine::Windows => self.pc.start(),
        }
    }

    fn up(&self, m: Machine) -> bool {
        match m {
            Machine::Mac => self.mac.connected(),
            Machine::Windows => self.pc.connected(),
        }
    }
}

fn other(m: Machine) -> Machine {
    match m {
        Machine::Mac => Machine::Windows,
        Machine::Windows => Machine::Mac,
    }
}

/// `host`: quem guarda os layouts (gravado nos dois configs, como depois de um pareamento).
/// O PC disca (menor id do PC): o Mac pode cair e voltar sem mudar as portas de ninguém.
async fn edicao_sync_desconexao_reconexao(host: Machine) {
    let mut s = Setup::new(false);
    s.host = host;
    let mut w = build(&s);
    w.start_both();
    w.connected(8000).await;
    let book0 = w.converged().await;
    assert_eq!(book0.host, host);
    let non_host = other(host);

    // Edição no NÃO-host: vira proposta, o host aceita e devolve o livro (Sync de volta).
    let v = 1.75_f32;
    w.edit(non_host, with_sensitivity(&book0, Machine::Windows, v))
        .expect("o não-host conectado edita");
    wait_until("host e não-host com o livro editado e igual", 8000, || {
        let (a, b) = (w.book(host), w.book(non_host));
        a == b && (sensitivity_of(&a, Machine::Windows) - v).abs() < 1e-3
    })
    .await;
    let book1 = w.book(host);
    assert!(book1.revision > book0.revision, "a revisão não subiu");
    assert_eq!(book1.host, host, "editar não troca o host");
    assert!(
        !w.mac.rec.layouts.lock().unwrap().is_empty()
            && !w.pc.rec.layouts.lock().unwrap().is_empty(),
        "relay:layout não foi emitido nos dois apps"
    );

    // Edição no HOST chega ao outro.
    let v2 = 0.6_f32;
    w.edit(host, with_sensitivity(&book1, Machine::Windows, v2))
        .unwrap();
    wait_until("edição do host espelhada", 8000, || {
        let (a, b) = (w.book(host), w.book(non_host));
        a == b && (sensitivity_of(&b, Machine::Windows) - v2).abs() < 1e-3
    })
    .await;
    let book2 = w.book(host);

    // Cai o NÃO-host: o espelho some da edição, mas o host segue dono do livro (I4: editable =
    // host é local || conectado).
    w.stop(non_host);
    wait_until("host sem sessão", 8000, || !w.up(host)).await;
    assert!(w.editable(host), "o host perdeu a edição offline");
    let v3 = 2.2_f32;
    w.edit(host, with_sensitivity(&book2, Machine::Windows, v3))
        .expect("o host edita offline");
    // Reconexão do não-host: recebe o que o host editou enquanto ele estava fora.
    w.start(non_host);
    w.connected(25_000).await;
    wait_until(
        "não-host alcança a edição offline do host",
        8000,
        || {
            let (a, b) = (w.book(host), w.book(non_host));
            a == b && (sensitivity_of(&b, Machine::Windows) - v3).abs() < 1e-3
        },
    )
    .await;
    wait_until("editável de novo nos dois", 5000, || {
        w.editable(Machine::Mac) && w.editable(Machine::Windows)
    })
    .await;

    // Cai o HOST: o não-host fica só-leitura, com o espelho intacto.
    let espelho = w.book(non_host);
    w.stop(host);
    wait_until("não-host sem sessão", 8000, || !w.up(non_host)).await;
    assert!(!w.editable(non_host), "o não-host editável sem o host");
    assert_eq!(
        w.edit(non_host, with_sensitivity(&espelho, Machine::Windows, 1.0))
            .err()
            .as_deref(),
        Some("layout_read_only")
    );
    let set_host = match non_host {
        Machine::Mac => w.mac.core().set_layout_host(Host::Local),
        Machine::Windows => {
            AgentBackend::set_layout_host(w.pc.core(), Host::Local).map_err(|e| e.to_string())
        }
    };
    assert_eq!(set_host.err().as_deref(), Some("layout_read_only"));
    assert_eq!(w.book(non_host), espelho, "o espelho some ao desconectar");

    // Reconexão do host: tudo volta a convergir e a editar.
    w.start(host);
    w.connected(25_000).await;
    assert_eq!(w.converged().await.host, host);
    wait_until("editável de novo nos dois", 5000, || {
        w.editable(Machine::Mac) && w.editable(Machine::Windows)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edicao_no_nao_host_pc_com_mac_host() {
    edicao_sync_desconexao_reconexao(Machine::Mac).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edicao_no_nao_host_mac_com_pc_host() {
    edicao_sync_desconexao_reconexao(Machine::Windows).await;
}

// ---------------------------------------------------------------------------------------------
// Cenário 4: peer 0.2 de um lado (fake no fio, o app real do outro)
// ---------------------------------------------------------------------------------------------

use relay_core::crypto::LocalDevice;
use relay_core::engine::testutil::profile as testutil_profile;
use relay_core::net::{
    DialerConfig, FastEvent, ListenerConfig, SessionDialer, SessionEvent, SessionListener,
};
use relay_core::types::Message;

fn is_new_in_03(m: &Message) -> bool {
    matches!(
        m,
        Message::Caps(_) | Message::LayoutBookSync(_) | Message::LayoutBookEdit(_)
    )
}

/// Mac 0.2: só disca (não anuncia, não escuta). Manda Displays e LayoutSync ao conectar, como a 0.2.
struct Mac02 {
    seen: Arc<Mutex<Vec<Message>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Mac02 {
    fn start(identity: StaticKeypair, pc: PairedPeer, remote: Ports) -> Self {
        let seen: Arc<Mutex<Vec<Message>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let mut cfg = DialerConfig::new(
                    identity,
                    pc,
                    LO,
                    remote,
                    Ports {
                        input: 0,
                        ..Ports::default()
                    },
                );
                cfg.bind_ip = LO;
                cfg.app_version = V02.into();
                let mut d = SessionDialer::start(cfg, Box::new(|_: FastEvent| {}))
                    .await
                    .unwrap();
                let mut status = d.status();
                let mut greeted = false;
                while !st.load(Ordering::SeqCst) {
                    tokio::select! {
                        ev = d.recv_event() => match ev {
                            None => break,
                            Some(SessionEvent::Message(m)) => s.lock().unwrap().push(m),
                            Some(_) => {}
                        },
                        _ = status.changed() => {
                            let up = *status.borrow_and_update() == ConnectionStatus::Connected;
                            if up && !greeted {
                                greeted = true;
                                let _ = d.send(Message::Displays(vec![mac_display()])).await;
                                let _ = d.send(Message::LayoutSync(testutil_profile(1.0))).await;
                            }
                            if !up { greeted = false; }
                        }
                        () = tokio::time::sleep(Duration::from_millis(20)) => {}
                    }
                }
                d.shutdown().await;
            });
        });
        Self {
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.seen.lock().unwrap().iter().any(f)
    }
}

impl Drop for Mac02 {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// PC 0.2: só escuta. Manda Displays ao conectar e guarda tudo o que recebe.
struct Pc02 {
    ports: Ports,
    seen: Arc<Mutex<Vec<Message>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Pc02 {
    fn start(identity: StaticKeypair, mac: PairedPeer) -> Self {
        let seen: Arc<Mutex<Vec<Message>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (seen.clone(), stop.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let device = LocalDevice {
                    name: "PC 0.2".into(),
                    public_key: identity.public,
                    mac_address: [1, 2, 3, 4, 5, 6],
                };
                let ports = free_ports();
                let mut cfg = ListenerConfig::new(identity, device, vec![mac], ports.clone());
                cfg.bind_ip = LO;
                cfg.app_version = V02.into();
                let mut l = SessionListener::start(cfg, Box::new(|_| {})).await.unwrap();
                tx.send(ports).unwrap();
                while !st.load(Ordering::SeqCst) {
                    tokio::select! {
                        ev = l.recv_event() => match ev {
                            None => break,
                            Some(SessionEvent::Connected(_)) => {
                                let _ = l.send(Message::Displays(vec![dell_display()])).await;
                            }
                            Some(SessionEvent::Message(m)) => s.lock().unwrap().push(m),
                            Some(_) => {}
                        },
                        () = tokio::time::sleep(Duration::from_millis(20)) => {}
                    }
                }
                l.shutdown().await;
            });
        });
        let ports = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            ports,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.seen.lock().unwrap().iter().any(f)
    }
}

impl Drop for Pc02 {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Mac 0.2 + PC 0.3 (app real): o PC só escuta; o Mac fake disca.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_02_com_pc_03_funciona_sem_mensagens_novas_e_o_pc_fica_so_leitura() {
    use relay_core::types::Os;
    let mac_store = Arc::new(MemoryKeyStore::default());
    let mac_identity = identity_in(&*mac_store);
    let mac_peer = peer_of(&mac_identity, "Mac 0.2", [0; 6]);
    let mut pc = PcNode::new(Arc::new(MemoryKeyStore::default()), |_| {});
    save_active_peer(&*pc.store, "active-peer", &mac_peer).unwrap();
    pc.start();
    let _mac = Mac02::start(mac_identity, pc.me(), pc.ports.clone());

    wait_until("PC conectado ao Mac 0.2", 10_000, || pc.connected()).await;
    // Mac 0.2 não anuncia Caps: o PC assume o baseline do Mac (D45) e o SO pelo papel.
    let st = pc.status();
    assert_eq!(st.peer_os, Some(Os::Mac));
    assert_eq!(st.peer_caps, Some(Caps::baseline(Os::Mac)));
    // O host efetivo é o Mac (D43): o PC mostra só-leitura.
    assert_eq!(st.layout_host, Host::Peer);
    assert!(!st.layout_editable);
    let book = pc.core().get_layout_book();
    assert_eq!(book.host, Machine::Mac);
    // A sessão funciona: Displays e LayoutSync do Mac 0.2 chegam ao PC.
    wait_until("PC recebeu as telas e o layout do Mac 0.2", 5000, || {
        !AgentBackend::get_displays(pc.core()).peer.is_empty()
            && !pc.core().get_layout_book().profiles.is_empty()
    })
    .await;
    assert_eq!(
        pc.core().get_layout_book().profiles[0].name,
        testutil_profile(1.0).name
    );
    assert_eq!(
        AgentBackend::save_layout(pc.core(), testutil_profile(1.0))
            .err()
            .map(|e| e.to_string())
            .as_deref(),
        Some("layout_read_only")
    );
    // Nada novo chega ao 0.2; e há prova de que o PC mandou algo (não é um teste vazio).
    wait_until("o Mac 0.2 recebeu o Displays do PC", 5000, || {
        _mac.saw(|m| matches!(m, Message::Displays(_)))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !_mac.saw(is_new_in_03),
        "mensagem da 0.3 chegou ao Mac 0.2: {:?}",
        _mac.seen.lock().unwrap()
    );
    assert!(
        !_mac.saw(|m| matches!(m, Message::RequestUpdate { .. })),
        "o PC 0.3 não pede atualização a um Mac 0.2 (D49)"
    );
}

/// Mac 0.3 (app real) + PC 0.2: o Mac disca como na 0.2 e continua host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_03_com_pc_02_funciona_sem_mensagens_novas_e_o_mac_segue_host() {
    use relay_core::types::Os;
    let mac_keys = Arc::new(MemoryKeyStore::default());
    let mac_identity = identity_in(&*mac_keys);
    let pc_store = MemoryKeyStore::default();
    let pc_identity = identity_in(&pc_store);
    let pc_peer = peer_of(&pc_identity, "PC 0.2", [1, 2, 3, 4, 5, 6]);
    let pc02 = Pc02::start(pc_identity, peer_of(&mac_identity, "Mac de teste", [0; 6]));
    let mut mac = MacNode::new(mac_keys, pc02.ports.clone(), |_| {});
    save_active_peer(&*mac.platform.keys, "active_peer", &pc_peer).unwrap();
    // PC 0.2 anuncia sem os/v.
    let (tx, rx) = channel();
    *mac.platform.browse.lock().unwrap() = rx;
    let _feeder = Feeder::start(
        tx,
        announce("PC 0.2", &pc_peer.device_id, pc02.ports.clone(), None, None),
    );
    mac.start();

    wait_until("Mac conectado ao PC 0.2", 10_000, || mac.connected()).await;
    let st = mac.status();
    assert_eq!(st.peer_os, Some(Os::Windows));
    assert_eq!(st.peer_caps, Some(Caps::baseline(Os::Windows)));
    assert_eq!(st.layout_host, Host::Local);
    assert!(st.layout_editable, "o Mac segue host e edita com um PC 0.2");
    // O Mac, host, cria o perfil quando as telas do PC 0.2 chegam e o manda como LayoutSync.
    wait_until("PC 0.2 recebeu LayoutSync", 8000, || {
        pc02.saw(|m| matches!(m, Message::LayoutSync(p) if p.displays.len() == 2))
    })
    .await;
    // Edição do Mac chega ao 0.2 como LayoutSync, nunca como livro.
    let book = mac.core().layout_book();
    let editado = with_sensitivity(&book, Machine::Windows, 1.9);
    mac.core().save_layout(editado).unwrap();
    wait_until("LayoutSync editado no PC 0.2", 5000, || {
        pc02.saw(|m| {
            matches!(m, Message::LayoutSync(p)
                if p.displays.iter().any(|d| d.machine == Machine::Windows
                    && (d.sensitivity - 1.9).abs() < 1e-3))
        })
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !pc02.saw(is_new_in_03),
        "mensagem da 0.3 chegou ao PC 0.2: {:?}",
        pc02.seen.lock().unwrap()
    );
    mac.stop();
}
