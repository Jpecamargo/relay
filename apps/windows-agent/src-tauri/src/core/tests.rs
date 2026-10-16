//! dono: WI1
//! Loopback no Mac: `AgentCore` com mocks de plataforma contra um "Mac falso" (o `Controller` do
//! core sobre uma `SessionDialer` real em 127.0.0.1).

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_core::crypto::{
    LocalDevice, MemoryKeyStore, StaticKeypair, device_id_of, load_or_create_identity, load_peer,
};
use relay_core::engine::Controller;
use relay_core::net::{DialerConfig, SessionDialer, SessionEvent};
use relay_core::platform::{
    CaptureError, ChangeCallback, CursorControl, DisplaySource, InjectError, InputCapture,
    InputInjector, KeyStore, PlatformError,
};
use relay_core::types::{
    CapturedEvent, Config, ConnectionStatus, DisplayInfo, InputEvent, KeyCode, LatencyStats,
    LayoutProfile, Machine, Message, PairedPeer, PairingEvent, PairingState, PlacedDisplay, Ports,
    ProfileKey, RectMm, StatusSnapshot,
};
use rtrb::{Consumer, RingBuffer};
use tokio::time::{sleep, timeout};

use super::*;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const KEY_A: u16 = 0x1E;

// ---- mock do lado Windows ----

#[derive(Default)]
struct Rec {
    status: Mutex<Vec<StatusSnapshot>>,
    pairing: Mutex<Vec<PairingEvent>>,
    latency: AtomicUsize,
}

impl UiEvents for Rec {
    fn status(&self, s: &StatusSnapshot) {
        self.status.lock().unwrap().push(s.clone());
    }
    fn latency(&self, _: &LatencyStats) {
        self.latency.fetch_add(1, Ordering::SeqCst);
    }
    fn pairing(&self, e: &PairingEvent) {
        self.pairing.lock().unwrap().push(e.clone());
    }
}

type Events = Arc<Mutex<Vec<InputEvent>>>;
type Pos = Arc<Mutex<(i32, i32)>>;

/// Injeta guardando os eventos; `MouseMove` desloca o cursor mock (como o Windows), limitado à tela.
struct MockInj {
    events: Events,
    cursor: Option<Pos>,
}

impl InputInjector for MockInj {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        if let (InputEvent::MouseMove { dx, dy }, Some(pos)) = (event, &self.cursor) {
            let mut p = pos.lock().unwrap();
            p.0 = (p.0 + i32::from(*dx)).clamp(0, 2559);
            p.1 = (p.1 + i32::from(*dy)).clamp(0, 1439);
        }
        self.events.lock().unwrap().push(*event);
        Ok(())
    }
    fn type_text(&mut self, _: &str) -> Result<(), InjectError> {
        Ok(())
    }
}

struct MockPipe {
    inj: MockInj,
    available: Arc<AtomicBool>,
}

impl InputInjector for MockPipe {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.inj.inject(event)
    }
    fn type_text(&mut self, t: &str) -> Result<(), InjectError> {
        self.inj.type_text(t)
    }
}

impl PipeInjector for MockPipe {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }
}

struct MockCursor(Pos);

impl CursorControl for MockCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn associate(&mut self, _: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        *self.0.lock().unwrap() = (x, y);
        Ok(())
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        Ok(*self.0.lock().unwrap())
    }
}

type Cb = Arc<Mutex<Option<ChangeCallback>>>;

struct MockDisplays {
    list: Arc<Mutex<Vec<DisplayInfo>>>,
    cb: Cb,
}

impl DisplaySource for MockDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(self.list.lock().unwrap().clone())
    }
    fn set_change_callback(&mut self, callback: ChangeCallback) {
        *self.cb.lock().unwrap() = Some(callback);
    }
}

type FgCb = Arc<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>>;

struct MockForeground(FgCb);

impl ForegroundSource for MockForeground {
    fn start(&mut self, on_change: Box<dyn Fn(String) + Send + Sync>) {
        *self.0.lock().unwrap() = Some(on_change);
    }
}

struct Guard(Arc<AtomicUsize>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn display(
    id: &str,
    machine: Machine,
    size: (u32, u32),
    mm: (f32, f32),
    builtin: bool,
) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: id.into(),
        machine,
        origin_px: (0, 0),
        size_px: size,
        scale: 1.0,
        size_mm: Some(mm),
        is_primary: builtin,
        is_builtin: builtin,
    }
}

fn mac_display() -> DisplayInfo {
    display("mac", Machine::Mac, (1440, 900), (300.0, 190.0), true)
}

fn dell_display() -> DisplayInfo {
    display(
        "dell",
        Machine::Windows,
        (2560, 1440),
        (597.0, 336.0),
        false,
    )
}

/// Mac à esquerda, Dell à direita (mais alto: o segmento compartilhado é a altura do Mac).
fn profile() -> LayoutProfile {
    let placed = |id: &str, machine, x, y, w, h| PlacedDisplay {
        display_id: id.into(),
        machine,
        rect_mm: RectMm { x, y, w, h },
        sensitivity: 1.0,
    };
    LayoutProfile {
        key: ProfileKey(vec!["dell".into(), "mac".into()]),
        name: "t".into(),
        displays: vec![
            placed("mac", Machine::Mac, 0.0, 0.0, 300.0, 190.0),
            placed("dell", Machine::Windows, 300.0, -100.0, 597.0, 336.0),
        ],
        windows_detached: false,
    }
}

struct Agent {
    core: AgentCore,
    rec: Arc<Rec>,
    store: Arc<MemoryKeyStore>,
    injected: Events,
    pipe_injected: Events,
    pipe_available: Arc<AtomicBool>,
    cursor: Pos,
    displays: Arc<Mutex<Vec<DisplayInfo>>>,
    display_cb: Cb,
    fg_cb: FgCb,
    engaged: Arc<AtomicUsize>,
    live_guards: Arc<AtomicUsize>,
    config_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn start_agent() -> Agent {
    start_agent_with(Arc::new(MemoryKeyStore::default()))
}

fn start_agent_with(store: Arc<MemoryKeyStore>) -> Agent {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let injected = Events::default();
    let pipe_injected = Events::default();
    let pipe_available = Arc::new(AtomicBool::new(false));
    let cursor: Pos = Arc::default();
    let displays = Arc::new(Mutex::new(vec![dell_display()]));
    let display_cb = Cb::default();
    let fg_cb = FgCb::default();
    let engaged = Arc::new(AtomicUsize::new(0));
    let live_guards = Arc::new(AtomicUsize::new(0));
    let rec = Arc::new(Rec::default());

    let mut config = Config::default();
    config.network.ports = Ports {
        control: 0,
        input: 0,
        ..Ports::default()
    };
    config.transition.dwell_ms = 0;

    let (eng, live) = (engaged.clone(), live_guards.clone());
    let pipe: SharedPipe = Arc::new(Mutex::new(Box::new(MockPipe {
        inj: MockInj {
            events: pipe_injected.clone(),
            cursor: Some(cursor.clone()),
        },
        available: pipe_available.clone(),
    })));
    let deps = AgentDeps {
        keystore: store.clone(),
        injector: Box::new(MockInj {
            events: injected.clone(),
            cursor: Some(cursor.clone()),
        }),
        pipe: Some(pipe),
        cursor: Box::new(MockCursor(cursor.clone())),
        displays: Box::new(MockDisplays {
            list: displays.clone(),
            cb: display_cb.clone(),
        }),
        foreground: Some(Box::new(MockForeground(fg_cb.clone()))),
        executor: None,
        pointer_guard: Box::new(move || {
            eng.fetch_add(1, Ordering::SeqCst);
            live.fetch_add(1, Ordering::SeqCst);
            Box::new(Guard(live.clone()))
        }),
        service_probe: Box::new(|helper_active| ServiceStatus {
            installed: true,
            running: true,
            helper_active,
        }),
        device_name: "dell-pc".into(),
        mac_address: [9, 8, 7, 6, 5, 4],
        config_path: Some(config_path.clone()),
        bind_ip: LO,
        advertise: false,
        message_hook: None,
        engine_events: None,
    };
    let core = AgentCore::start(deps, config, rec.clone()).unwrap();
    Agent {
        core,
        rec,
        store,
        injected,
        pipe_injected,
        pipe_available,
        cursor,
        displays,
        display_cb,
        fg_cb,
        engaged,
        live_guards,
        config_path,
        _dir: dir,
    }
}

impl Agent {
    fn foreground(&self, exe: &str) {
        (self.fg_cb.lock().unwrap().as_ref().unwrap())(exe.into());
    }

    fn injected(&self) -> Vec<InputEvent> {
        self.injected.lock().unwrap().clone()
    }

    fn paired_events(&self) -> Vec<PairingState> {
        self.rec
            .pairing
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.state)
            .collect()
    }
}

// ---- o "Mac" ----

struct NopCap;
impl InputCapture for NopCap {
    fn start(&mut self, _: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn set_swallow(&self, _: bool) {}
    fn is_swallowing(&self) -> bool {
        false
    }
}

struct NopCursor;
impl CursorControl for NopCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }
    fn associate(&mut self, _: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    fn warp(&mut self, _: i32, _: i32) -> Result<(), PlatformError> {
        Ok(())
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        Ok((0, 0))
    }
}

struct NopInj;
impl InputInjector for NopInj {
    fn inject(&mut self, _: &InputEvent) -> Result<(), InjectError> {
        Ok(())
    }
    fn type_text(&mut self, _: &str) -> Result<(), InjectError> {
        Ok(())
    }
}

struct Mac {
    dialer: SessionDialer,
    ctl: Controller<NopCap, NopCursor, NopInj>,
    control: Consumer<Message>,
    received: Vec<Message>,
}

fn mac_device(k: &StaticKeypair) -> LocalDevice {
    LocalDevice {
        name: "macbook".into(),
        public_key: k.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    }
}

fn ports_of(agent: &Agent) -> Ports {
    let (c, i) = agent.core.local_addrs();
    Ports {
        control: c.port(),
        input: i.port(),
        ..Ports::default()
    }
}

/// Pareia pelo `AgentCore` (código aberto por `new_pairing_code`) e devolve o peer do agente.
async fn pair(agent: &Agent, mac: &StaticKeypair) -> PairedPeer {
    let code = agent.core.new_pairing_code().unwrap().code;
    eventually("código aberto", || {
        agent.paired_events().contains(&PairingState::Code)
    })
    .await;
    SessionDialer::pair(agent.core.local_addrs().0, &code, &mac_device(mac))
        .await
        .expect("pareamento")
}

async fn connect(agent: &Agent, mac: &StaticKeypair, peer: PairedPeer) -> Mac {
    let mut cfg = DialerConfig::new(
        mac.clone(),
        peer,
        LO,
        ports_of(agent),
        Ports {
            input: 0,
            ..Ports::default()
        },
    );
    cfg.bind_ip = LO;
    let mut dialer = SessionDialer::start(cfg, Box::new(|_| {})).await.unwrap();
    let fast = dialer.take_input_producer().unwrap();
    let (ctrl_p, control) = RingBuffer::new(4096);
    let mut cfg = Config::default();
    cfg.transition.dwell_ms = 0;
    let ctl = Controller::new(
        &cfg,
        profile(),
        vec![mac_display()],
        NopCap,
        NopCursor,
        NopInj,
        fast,
        ctrl_p,
    );
    Mac {
        dialer,
        ctl,
        control,
        received: Vec::new(),
    }
}

impl Mac {
    async fn flush(&mut self) {
        while let Ok(m) = self.control.pop() {
            let _ = self.dialer.send(m).await;
        }
    }

    /// Esvazia o que o Mac quer enviar e recebe do agente até `cond` ser verdadeira.
    async fn pump_until(&mut self, what: &str, mut cond: impl FnMut(&mut Mac) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.flush().await;
            if cond(self) {
                return;
            }
            assert!(Instant::now() < deadline, "tempo esgotado: {what}");
            if let Ok(Some(SessionEvent::Message(m))) =
                timeout(Duration::from_millis(10), self.dialer.recv_event()).await
            {
                self.ctl.on_message(m.clone(), Instant::now());
                self.received.push(m);
            }
        }
    }

    fn got(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.received.iter().any(f)
    }

    fn key(&mut self, scan: u16, down: bool) {
        self.ctl.handle(CapturedEvent::Key {
            code: KeyCode::Scan(scan),
            down,
            flags: 0,
            autorepeat: false,
        });
    }

    fn mv(&mut self, dx: i32, dy: i32) {
        self.ctl.handle(CapturedEvent::MouseMove {
            dx,
            dy,
            pos: (1439.0, 450.0),
        });
    }
}

async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !cond() {
        assert!(Instant::now() < deadline, "tempo esgotado: {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

fn identity() -> StaticKeypair {
    load_or_create_identity(&MemoryKeyStore::default()).unwrap()
}

// ---- testes ----

#[tokio::test(flavor = "multi_thread")]
async fn loopback_pareia_conecta_injeta_e_devolve_pela_borda() {
    let agent = start_agent();
    let mac_id = identity();
    let mac_dev_id = device_id_of(&mac_id.public);

    // Pareamento: código de 6 dígitos válido por 120 s, peer guardado no KeyStore.
    let before = now_ms();
    let code = agent.core.new_pairing_code().unwrap();
    assert_eq!(code.code.len(), 6);
    assert!((119_000..=121_000).contains(&(code.expires_at - before)));
    eventually("evento de código", || {
        agent
            .rec
            .pairing
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Code && e.code.as_deref() == Some(&code.code))
    })
    .await;
    eventually("status Pairing", || {
        agent.core.status().status == ConnectionStatus::Pairing
    })
    .await;
    let peer = SessionDialer::pair(agent.core.local_addrs().0, &code.code, &mac_device(&mac_id))
        .await
        .expect("pareamento");
    assert_eq!(peer.device_id, agent.core.device_id());
    eventually("peer salvo", || {
        load_peer(&*agent.store, &mac_dev_id).unwrap().is_some()
    })
    .await;
    eventually("evento paired", || {
        agent.paired_events().contains(&PairingState::Paired)
    })
    .await;
    assert_eq!(agent.core.status().peer_name.as_deref(), Some("macbook"));

    // App em primeiro plano já conhecido antes da conexão: segue com as telas ao conectar.
    agent.foreground("explorer.exe");
    sleep(Duration::from_millis(50)).await;
    let mut mac = connect(&agent, &mac_id, peer).await;
    mac.pump_until("Displays + ForegroundApp ao conectar", |m| {
        m.got(|x| matches!(x, Message::Displays(d) if d.len() == 1 && d[0].id == "dell"))
            && m.got(|x| matches!(x, Message::ForegroundApp { exe } if exe == "explorer.exe"))
    })
    .await;
    assert_eq!(agent.core.status().status, ConnectionStatus::Connected);
    assert_eq!(agent.engaged.load(Ordering::SeqCst), 1);
    assert_eq!(agent.live_guards.load(Ordering::SeqCst), 1);

    // Mudança de app e de telas durante a sessão.
    agent.foreground("code.exe");
    agent.displays.lock().unwrap().push(display(
        "dell2",
        Machine::Windows,
        (1920, 1080),
        (527.0, 296.0),
        false,
    ));
    (agent.display_cb.lock().unwrap().as_ref().unwrap())();
    mac.pump_until("troca de app e de telas", |m| {
        m.got(|x| matches!(x, Message::ForegroundApp { exe } if exe == "code.exe"))
            && m.got(|x| matches!(x, Message::Displays(d) if d.len() == 2))
    })
    .await;

    // O Mac manda as próprias telas e o layout (como o MI1 fará ao conectar); o layout recebido
    // fica no Config do agente e em disco.
    mac.dialer
        .send(Message::Displays(vec![mac_display()]))
        .await
        .unwrap();
    mac.dialer
        .send(Message::LayoutSync(profile()))
        .await
        .unwrap();
    eventually("layout no config", || {
        agent.core.config().layout.profiles == vec![profile()]
    })
    .await;
    let on_disk = relay_core::config::load_or_default(&agent.config_path).unwrap();
    assert_eq!(on_disk.layout.profiles, vec![profile()]);

    // Empurrar a borda direita do Mac entra no Dell; Key e MouseMove chegam ao injetor.
    mac.mv(5, 0);
    assert!(mac.ctl.is_remote());
    mac.pump_until("FocusEnter aplicado", |_| {
        agent.core.status().focus == Machine::Windows && *agent.cursor.lock().unwrap() != (0, 0)
    })
    .await;
    let entry = *agent.cursor.lock().unwrap();
    assert_eq!(entry.0, 0);
    mac.key(KEY_A, true);
    mac.pump_until("tecla injetada", |_| {
        agent.injected().contains(&InputEvent::Key {
            scan: KEY_A,
            down: true,
        })
    })
    .await;
    // O movimento relativo vai por UDP (o par aprende o endereço pelo probe do heartbeat).
    mac.pump_until("MouseMove (UDP) injetado", |m| {
        m.ctl.handle(CapturedEvent::MouseMove {
            dx: 10,
            dy: 0,
            pos: (0.0, 0.0),
        });
        agent
            .injected()
            .contains(&InputEvent::MouseMove { dx: 10, dy: 0 })
    })
    .await;

    // Retorno: empurrar a borda esquerda do Dell. O agente detecta pela posição real do cursor e
    // manda FocusReturn; o Mac volta a ter o foco e o agente solta a tecla.
    mac.pump_until("retorno pela borda", |m| {
        m.ctl.handle(CapturedEvent::MouseMove {
            dx: -30,
            dy: 0,
            pos: (0.0, 0.0),
        });
        !m.ctl.is_remote()
    })
    .await;
    assert!(
        mac.got(|x| matches!(x, Message::FocusReturn { display_id, .. } if display_id == "mac"))
    );
    mac.pump_until("tecla solta no agente", |_| {
        agent.injected().contains(&InputEvent::Key {
            scan: KEY_A,
            down: false,
        }) && agent.core.status().focus == Machine::Mac
    })
    .await;

    // Queda da sessão: o ponteiro é restaurado e o status sai de Connected.
    mac.dialer.shutdown().await;
    eventually("ponteiro restaurado", || {
        agent.live_guards.load(Ordering::SeqCst) == 0
    })
    .await;
    eventually("status fora de Connected", || {
        agent.core.status().status != ConnectionStatus::Connected
    })
    .await;
    assert!(
        agent
            .rec
            .status
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.status == ConnectionStatus::Connected),
        "relay:status chegou à UI"
    );
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn queda_com_o_mac_no_controle_chama_focus_lost() {
    let agent = start_agent();
    let mac_id = identity();
    let peer = pair(&agent, &mac_id).await;
    let mut mac = connect(&agent, &mac_id, peer).await;
    mac.pump_until("conectado", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    mac.dialer
        .send(Message::LayoutSync(profile()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;
    mac.mv(5, 0);
    mac.key(KEY_A, true);
    mac.pump_until("tecla injetada", |_| {
        agent.injected().contains(&InputEvent::Key {
            scan: KEY_A,
            down: true,
        })
    })
    .await;
    mac.dialer.shutdown().await;
    eventually("tecla solta pela queda", || {
        agent.injected().contains(&InputEvent::Key {
            scan: KEY_A,
            down: false,
        }) && agent.core.status().focus == Machine::Mac
    })
    .await;
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_nao_pareado_e_recusado() {
    let agent = start_agent();
    let mac_id = identity();
    let peer = pair(&agent, &mac_id).await;
    // Outra identidade, que conhece a chave pública do agente mas nunca pareou.
    let intruder = identity();
    let mut mac = connect(&agent, &intruder, peer).await;
    for _ in 0..40 {
        mac.flush().await;
        sleep(Duration::from_millis(50)).await;
        assert_ne!(agent.core.status().status, ConnectionStatus::Connected);
        assert_ne!(*mac.dialer.status().borrow(), ConnectionStatus::Connected);
    }
    assert_eq!(agent.engaged.load(Ordering::SeqCst), 0);
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn codigo_e_de_uso_unico() {
    let agent = start_agent();
    let mac_id = identity();
    let code = agent.core.new_pairing_code().unwrap().code;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let addr = agent.core.local_addrs().0;
    assert!(
        SessionDialer::pair(addr, wrong, &mac_device(&mac_id))
            .await
            .is_err()
    );
    eventually("falha reportada", || {
        agent.paired_events().contains(&PairingState::Failed)
    })
    .await;
    // A primeira tentativa consumiu o código, mesmo errada.
    assert!(
        SessionDialer::pair(addr, &code, &mac_device(&mac_id))
            .await
            .is_err()
    );
    assert!(
        load_peer(&*agent.store, &device_id_of(&mac_id.public))
            .unwrap()
            .is_none()
    );
    assert_ne!(agent.core.status().status, ConnectionStatus::Pairing);
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn revogar_remove_o_peer_e_manda_unpair() {
    let agent = start_agent();
    let mac_id = identity();
    let mac_dev_id = device_id_of(&mac_id.public);
    let peer = pair(&agent, &mac_id).await;
    eventually("peer salvo", || {
        load_peer(&*agent.store, &mac_dev_id).unwrap().is_some()
    })
    .await;
    let mut mac = connect(&agent, &mac_id, peer).await;
    mac.pump_until("conectado", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    agent.core.revoke_pair().unwrap();
    mac.pump_until("Unpair recebido", |m| {
        m.got(|x| matches!(x, Message::Unpair))
    })
    .await;
    eventually("peer removido", || {
        load_peer(&*agent.store, &mac_dev_id).unwrap().is_none()
            && agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_none()
    })
    .await;
    eventually("sem peer na UI", || agent.core.status().peer_name.is_none()).await;
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_guardado_sobrevive_ao_reinicio() {
    let agent = start_agent();
    let mac_id = identity();
    let _ = pair(&agent, &mac_id).await;
    eventually("peer salvo", || {
        agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_some()
    })
    .await;
    agent.core.shutdown();
    let store = agent.store.clone();
    drop(agent);
    // Mesmo KeyStore: o novo agente já nasce pareado.
    let again = start_agent_with(store);
    eventually("pareado ao subir", || {
        again.core.status().peer_name.as_deref() == Some("macbook")
    })
    .await;
    again.core.shutdown();
}

#[test]
fn injecao_usa_o_pipe_quando_disponivel_e_cai_para_o_local() {
    let local = Events::default();
    let piped = Events::default();
    let available = Arc::new(AtomicBool::new(true));
    let pipe: SharedPipe = Arc::new(Mutex::new(Box::new(MockPipe {
        inj: MockInj {
            events: piped.clone(),
            cursor: None,
        },
        available: available.clone(),
    })));
    let mut inj = RoutedInjector::new(
        Box::new(MockInj {
            events: local.clone(),
            cursor: None,
        }),
        Some(pipe),
    );
    let ev = InputEvent::Key {
        scan: KEY_A,
        down: true,
    };
    inj.inject(&ev).unwrap();
    assert_eq!(
        (piped.lock().unwrap().len(), local.lock().unwrap().len()),
        (1, 0)
    );
    available.store(false, Ordering::SeqCst);
    inj.inject(&ev).unwrap();
    assert_eq!(
        (piped.lock().unwrap().len(), local.lock().unwrap().len()),
        (1, 1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn estado_do_servico_reflete_o_pipe() {
    let agent = start_agent();
    assert!(!agent.core.service_status().helper_active);
    agent.pipe_available.store(true, Ordering::SeqCst);
    assert!(agent.core.service_status().helper_active);
    assert!(agent.pipe_injected.lock().unwrap().is_empty());
    agent.core.shutdown();
}
