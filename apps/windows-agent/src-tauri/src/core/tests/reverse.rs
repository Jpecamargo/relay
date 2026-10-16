//! dono: R1
//! Loopback do sentido inverso: `AgentCore` com hooks mock (captura, cursor, injetor) contra um
//! "Mac falso" (o `Receiver` do core em modo Mac sobre uma `SessionDialer` real em 127.0.0.1).

use std::sync::mpsc::{Receiver as StdRx, channel};

use relay_core::engine::Receiver;
use relay_core::platform::{CaptureError, InputCapture};
use relay_core::types::{CapturedEvent, KeyCode};

use super::*;

const CTRL: u16 = 0x1D;
const ALT: u16 = 0x38;
const WIN: u16 = 0xE05B;
const KEY_C: u16 = 0x2E;
const DIGIT_2: u16 = 0x03;

type Sink = Arc<Mutex<Option<rtrb::Producer<CapturedEvent>>>>;

/// Hooks LL falsos: o teste empurra eventos no `sink` que a thread reversa consome.
struct RevCap {
    sink: Sink,
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

struct RevAgent {
    agent: Agent,
    sink: Sink,
    swallow: Arc<AtomicBool>,
    /// Teclas que o PC soltou localmente ao cruzar a borda.
    local_injected: Events,
    engine_log: Arc<Mutex<Vec<relay_core::engine::EngineEvent>>>,
}

impl RevAgent {
    fn push(&self, ev: CapturedEvent) {
        let mut g = self.sink.lock().unwrap();
        g.as_mut().expect("captura iniciada").push(ev).unwrap();
    }

    fn key(&self, scan: u16, down: bool) {
        self.push(CapturedEvent::Key {
            code: KeyCode::Scan(scan),
            down,
            flags: 0,
            autorepeat: false,
        });
    }

    /// Empurra a borda esquerda do Dell, ligada ao Mac, na altura de 830 px.
    fn push_left_edge(&self) {
        self.push(CapturedEvent::MouseMove {
            dx: -5,
            dy: 0,
            pos: (0.0, 830.0),
        });
    }
}

fn start_reverse_agent() -> RevAgent {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let injected = Events::default();
    let cursor: Pos = Arc::default();
    let displays = Arc::new(Mutex::new(vec![dell_display()]));
    let rec = Arc::new(Rec::default());
    let mut config = Config::default();
    config.network.ports = Ports {
        control: 0,
        input: 0,
        ..Ports::default()
    };
    config.transition.dwell_ms = 0;
    config.clipboard.enabled = true;
    config.clipboard.mode = relay_core::types::ClipboardMode::OnSwitch;
    let engine_log: Arc<Mutex<Vec<relay_core::engine::EngineEvent>>> = Arc::default();
    let sink = engine_log.clone();
    let store = Arc::new(MemoryKeyStore::default());
    let deps = AgentDeps {
        keystore: store.clone(),
        injector: Box::new(MockInj {
            events: injected.clone(),
            cursor: Some(cursor.clone()),
        }),
        pipe: None,
        cursor: Box::new(MockCursor(cursor.clone())),
        displays: Box::new(MockDisplays {
            list: displays.clone(),
            cb: Cb::default(),
        }),
        foreground: None,
        executor: None,
        media: None,
        pointer_guard: Box::new(|| Box::new(())),
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
        engine_events: Some(Box::new(move |e| sink.lock().unwrap().push(e))),
    };
    let sink: Sink = Arc::default();
    let swallow = Arc::new(AtomicBool::new(false));
    let local_injected = Events::default();
    let reverse = ReverseDeps {
        capture: Box::new(RevCap {
            sink: sink.clone(),
            swallow: swallow.clone(),
        }),
        cursor: Box::new(MockCursor(Arc::default())),
        injector: Box::new(MockInj {
            events: local_injected.clone(),
            cursor: None,
        }),
    };
    let core = AgentCore::start_with_reverse(deps, config, rec.clone(), Some(reverse)).unwrap();
    RevAgent {
        agent: Agent {
            core,
            rec,
            store,
            injected,
            pipe_injected: Events::default(),
            pipe_available: Arc::default(),
            cursor,
            displays,
            display_cb: Cb::default(),
            fg_cb: FgCb::default(),
            engaged: Arc::default(),
            live_guards: Arc::default(),
            config_path,
            _dir: dir,
        },
        sink,
        swallow,
        local_injected,
        engine_log,
    }
}

/// O Mac falso: sessão discada + `Receiver` em modo Mac com injetor e cursor mock.
struct FakeMac {
    dialer: SessionDialer,
    rx: Receiver<MockInj, MockCursor>,
    fast: StdRx<InputEvent>,
    injected: Events,
    received: Vec<Message>,
}

async fn connect_mac(ra: &RevAgent, mac_id: &StaticKeypair, peer: PairedPeer) -> FakeMac {
    let mut cfg = DialerConfig::new(
        mac_id.clone(),
        peer,
        LO,
        ports_of(&ra.agent),
        Ports {
            input: 0,
            ..Ports::default()
        },
    );
    cfg.bind_ip = LO;
    let (fast_tx, fast) = channel();
    let dialer = SessionDialer::start(
        cfg,
        Box::new(move |ev| {
            let _ = fast_tx.send(ev.event);
        }),
    )
    .await
    .unwrap();
    let injected = Events::default();
    let cursor: Pos = Arc::default();
    let mut config = Config::default();
    config.transition.dwell_ms = 0;
    let mut rx = Receiver::new(
        &config,
        vec![mac_display()],
        MockInj {
            events: injected.clone(),
            cursor: None,
        },
        MockCursor(cursor),
    );
    rx.set_machine(Machine::Mac);
    rx.set_profile(profile());
    FakeMac {
        dialer,
        rx,
        fast,
        injected,
        received: Vec::new(),
    }
}

impl FakeMac {
    async fn pump_until(&mut self, what: &str, mut cond: impl FnMut(&mut FakeMac) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            while let Ok(ev) = self.fast.try_recv() {
                let back = self.rx.on_input(ev, Instant::now());
                for m in back {
                    let _ = self.dialer.send(m).await;
                }
            }
            if cond(self) {
                return;
            }
            assert!(Instant::now() < deadline, "tempo esgotado: {what}");
            if let Ok(Some(SessionEvent::Message(m))) =
                timeout(Duration::from_millis(10), self.dialer.recv_event()).await
            {
                self.received.push(m.clone());
                for back in self.rx.on_message(m, Instant::now()) {
                    let _ = self.dialer.send(back).await;
                }
            }
        }
    }

    fn got(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.received.iter().any(f)
    }

    fn events(&self) -> Vec<InputEvent> {
        self.injected.lock().unwrap().clone()
    }
}

fn key(scan: u16, down: bool) -> InputEvent {
    InputEvent::Key { scan, down }
}

#[tokio::test(flavor = "multi_thread")]
async fn pc_assume_traduz_teclas_devolve_e_e_recusado_com_o_mac_no_controle() {
    let ra = start_reverse_agent();
    let mac_id = identity();
    let peer = pair(&ra.agent, &mac_id).await;
    let mut mac = connect_mac(&ra, &mac_id, peer).await;
    mac.pump_until("Displays do agente", |m| {
        m.got(|x| matches!(x, Message::Displays(d) if d.len() == 1))
    })
    .await;
    // O Mac manda as telas e o layout (como o MI1 faz ao conectar).
    mac.rx
        .on_message(Message::Displays(vec![dell_display()]), Instant::now());
    mac.dialer
        .send(Message::Displays(vec![mac_display()]))
        .await
        .unwrap();
    mac.dialer
        .send(Message::LayoutSync(profile()))
        .await
        .unwrap();
    eventually("layout no config", || {
        ra.agent.core.config().layout.profiles == vec![profile()]
    })
    .await;
    sleep(Duration::from_millis(100)).await;

    // 1. O mouse do PC empurra a borda ligada ao Mac e o PC assume.
    ra.push_left_edge();
    mac.pump_until("FocusEnter no Mac", |m| m.rx.is_active())
        .await;
    eventually("input engolido no PC", || ra.swallow.load(Ordering::SeqCst)).await;

    // Ao assumir o Mac o PC perde o foco: oferece o clipboard (ClipboardSwitch) uma vez.
    eventually("ClipboardSwitch do PC", || {
        ra.engine_log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == relay_core::engine::EngineEvent::ClipboardSwitch)
            .count()
            == 1
    })
    .await;

    // 2. Ctrl+C no PC chega como Cmd+C no Mac (D25); a repetição do sistema não vai.
    ra.key(CTRL, true);
    ra.key(KEY_C, true);
    ra.key(KEY_C, true);
    ra.key(KEY_C, false);
    ra.key(CTRL, false);
    mac.pump_until("Cmd+C no Mac", |m| {
        m.events().contains(&key(WIN, false)) && m.events().len() >= 4
    })
    .await;
    let keys: Vec<_> = mac
        .events()
        .into_iter()
        .filter(|e| matches!(e, InputEvent::Key { .. }))
        .collect();
    assert_eq!(
        keys,
        [
            key(WIN, true),
            key(KEY_C, true),
            key(KEY_C, false),
            key(WIN, false)
        ]
    );

    // 3. Devolve por hotkey (Ctrl+Alt+2 = segunda tela, a do PC): o Mac solta tudo.
    ra.key(CTRL, true);
    ra.key(ALT, true);
    ra.key(DIGIT_2, true);
    mac.pump_until("Mac devolvido", |m| {
        !m.rx.is_active() && m.got(|x| matches!(x, Message::FocusReturn { .. }))
    })
    .await;
    eventually("input do PC liberado", || {
        !ra.swallow.load(Ordering::SeqCst)
    })
    .await;
    assert!(mac.events().contains(&key(ALT, false)));
    assert!(!mac.events().contains(&key(DIGIT_2, true)));
    ra.key(ALT, false);
    ra.key(CTRL, false);
    let _ = &ra.local_injected;

    // 4. Com o Mac controlando o PC, o PC não assume (D24).
    mac.dialer
        .send(Message::FocusEnter {
            display_id: "dell".into(),
            entry_px: (1000, 700),
        })
        .await
        .unwrap();
    eventually("Mac no controle", || {
        ra.agent.core.status().focus == Machine::Windows
    })
    .await;
    mac.received.clear();
    ra.push_left_edge();
    mac.pump_until("nenhum FocusEnter chegou", |m| {
        sleep_blocking(300);
        !m.got(|x| matches!(x, Message::FocusEnter { .. })) && !m.rx.is_active()
    })
    .await;
    assert!(!ra.swallow.load(Ordering::SeqCst));

    // 5. O Mac devolve o PC e o PC volta a poder assumir.
    mac.dialer
        .send(Message::FocusReturn {
            display_id: "mac".into(),
            point_px: (1439, 450),
        })
        .await
        .unwrap();
    eventually("PC livre", || ra.agent.core.status().focus == Machine::Mac).await;
    sleep(Duration::from_millis(50)).await;
    ra.push_left_edge();
    mac.pump_until("PC assumiu de novo", |m| m.rx.is_active())
        .await;
    ra.agent.core.shutdown();
}

fn sleep_blocking(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
