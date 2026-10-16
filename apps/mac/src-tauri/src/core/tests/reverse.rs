//! dono: R1
//! Sentido inverso em loopback: `AppCore` (plataforma mock, injetor mock) contra um "PC falso"
//! (`SessionListener` + `Controller` em modo reverso, alimentado pelo teste como os hooks LL).

use std::sync::mpsc::{Receiver as StdRx, Sender as StdTx, channel};

use relay_core::engine::Controller;
use relay_core::types::KeyCode;
use rtrb::RingBuffer;

use super::*;

const CTRL: u16 = 0x1D;
const ALT: u16 = 0x38;
const WIN: u16 = 0xE05B;
const KEY_C: u16 = 0x2E;
const DIGIT_2: u16 = 0x03;

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

struct NopInj;
impl InputInjector for NopInj {
    fn inject(&mut self, _: &InputEvent) -> Result<(), InjectError> {
        Ok(())
    }
    fn type_text(&mut self, _: &str) -> Result<(), InjectError> {
        Ok(())
    }
}

struct PcAgent {
    control_port: u16,
    input_port: u16,
    received: Arc<Mutex<Vec<Message>>>,
    /// O Controller reverso do PC está com o foco (controlando o Mac).
    remote: Arc<AtomicBool>,
    events: StdTx<CapturedEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PcAgent {
    fn start(code: &str) -> Self {
        let received: Arc<Mutex<Vec<Message>>> = Arc::default();
        let remote = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (ports_tx, ports_rx) = channel();
        let (events, events_rx) = channel();
        let (r, rm, s, code) = (
            received.clone(),
            remote.clone(),
            stop.clone(),
            code.to_owned(),
        );
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(pc_main(code, ports_tx, r, rm, events_rx, s));
        });
        let (control_port, input_port) = ports_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            control_port,
            input_port,
            received,
            remote,
            events,
            stop,
            thread: Some(thread),
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.received.lock().unwrap().iter().any(f)
    }

    fn count(&self, f: impl Fn(&Message) -> bool) -> usize {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|m| f(m))
            .count()
    }

    fn feed(&self, ev: CapturedEvent) {
        let _ = self.events.send(ev);
    }

    fn key(&self, scan: u16, down: bool) {
        self.feed(CapturedEvent::Key {
            code: KeyCode::Scan(scan),
            down,
            flags: 0,
            autorepeat: false,
        });
    }

    /// Empurra a borda esquerda do Dell (ligada ao Mac) perto do topo.
    fn push_left_edge(&self) {
        self.feed(CapturedEvent::MouseMove {
            dx: -5,
            dy: 0,
            pos: (0.0, 100.0),
        });
    }

    fn config(&self) -> Config {
        let mut cfg = Config::default();
        cfg.network.ports = Ports {
            control: self.control_port,
            input: self.input_port,
            ..Ports::default()
        };
        cfg
    }
}

impl Drop for PcAgent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

async fn pc_main(
    code: String,
    ports_tx: StdTx<(u16, u16)>,
    received: Arc<Mutex<Vec<Message>>>,
    remote: Arc<AtomicBool>,
    events: StdRx<CapturedEvent>,
    stop: Arc<AtomicBool>,
) {
    let identity = load_or_create_identity(&MemoryKeyStore::default()).unwrap();
    let device = LocalDevice {
        name: "PC de teste".into(),
        public_key: identity.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    };
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
    let mut listener = SessionListener::start(cfg, Box::new(|_| {})).await.unwrap();
    listener.open_pairing(code, Duration::from_secs(120));
    ports_tx
        .send((
            listener.local_control_addr().port(),
            listener.local_input_addr().port(),
        ))
        .unwrap();
    let fast = listener.take_input_producer().unwrap();
    let (ctrl_p, mut control) = RingBuffer::new(1024);
    let mut config = Config::default();
    config.transition.dwell_ms = 0;
    let mut ctl = Controller::new(
        &config,
        relay_core::types::LayoutProfile {
            key: Default::default(),
            name: String::new(),
            displays: Vec::new(),
            windows_detached: false,
        },
        vec![dell_display()],
        NopCap,
        MockCursor::new(&Default::default()),
        NopInj,
        fast,
        ctrl_p,
    );
    ctl.set_reverse(config.keymap.reverse_modifiers.clone());
    while !stop.load(Ordering::SeqCst) {
        tokio::select! {
            ev = listener.recv_event() => match ev {
                None => break,
                Some(SessionEvent::Paired(peer)) => listener.add_peer(peer),
                Some(SessionEvent::Connected(_)) => {
                    let _ = listener.send(Message::Displays(vec![dell_display()])).await;
                }
                Some(SessionEvent::Message(m)) => {
                    received.lock().unwrap().push(m.clone());
                    match m {
                        Message::LayoutSync(p) => ctl.set_profile(p),
                        m => ctl.on_message(m, Instant::now()),
                    }
                }
                Some(_) => {}
            },
            () = tokio::time::sleep(Duration::from_millis(2)) => {}
        }
        while let Ok(ev) = events.try_recv() {
            ctl.handle(ev);
        }
        ctl.tick(Instant::now());
        while let Ok(m) = control.pop() {
            let _ = listener.send(m).await;
        }
        remote.store(ctl.is_remote(), Ordering::SeqCst);
    }
    listener.shutdown().await;
}

#[derive(Default)]
struct CountSwitch(std::sync::atomic::AtomicUsize);
impl MediaHooks for CountSwitch {
    fn clipboard_switch(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn key(scan: u16, down: bool) -> InputEvent {
    InputEvent::Key { scan, down }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pc_controla_o_mac_traduz_teclas_devolve_e_um_lado_so_controla() {
    let pc = PcAgent::start("123456");
    let mut cfg = pc.config();
    cfg.clipboard.enabled = true;
    cfg.clipboard.mode = relay_core::types::ClipboardMode::OnSwitch;
    let platform = MockPlatform::new(&cfg);
    let rec = Arc::new(Rec::default());
    let media = Arc::new(CountSwitch::default());
    let core = AppCore::start(
        platform.clone(),
        rec.clone(),
        media.clone(),
        options(),
        Handle::current(),
    );
    let addr = format!("127.0.0.1:{}", pc.control_port);
    core.pair_with_code("123456".into(), Some(addr))
        .await
        .unwrap();
    wait_until("conectado", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    wait_until("layout no PC", 5000, || {
        pc.saw(|m| matches!(m, Message::LayoutSync(p) if p.displays.len() == 2))
    })
    .await;
    // Deixa o PC processar o LayoutSync e o Displays antes de empurrar a borda.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 1. O mouse do PC empurra a borda ligada ao Mac: o Mac vira Receiver e posiciona o cursor.
    pc.push_left_edge();
    wait_until("PC assumiu", 3000, || pc.remote.load(Ordering::SeqCst)).await;
    wait_until("cursor posicionado no Mac", 3000, || {
        platform
            .cursor_log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("warp(1439,"))
    })
    .await;

    // Ganhar o foco não oferece o clipboard (só quem perde oferece).
    assert_eq!(media.0.load(Ordering::SeqCst), 0);

    // 2. Ctrl+C no PC chega como Cmd+C no Mac.
    pc.key(CTRL, true);
    pc.key(KEY_C, true);
    pc.key(KEY_C, false);
    pc.key(CTRL, false);
    wait_until("Cmd+C injetado", 3000, || {
        platform.injected.lock().unwrap().contains(&key(WIN, false))
    })
    .await;
    let keys: Vec<_> = platform
        .injected
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, InputEvent::Key { .. }))
        .copied()
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

    // 3. O movimento do PC chega pelo canal rápido (UDP) ao injetor do Mac.
    let t = Instant::now();
    loop {
        pc.feed(CapturedEvent::MouseMove {
            dx: -4,
            dy: 1,
            pos: (0.0, 100.0),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        if platform
            .injected
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, InputEvent::MouseMove { .. }))
        {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(4), "MouseMove não chegou");
    }

    // 4. Com o PC no controle, o Mac não assume: empurrar a borda do Mac não manda FocusEnter.
    move_to_edge(&platform, 300);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!pc.saw(|m| matches!(m, Message::FocusEnter { .. })));
    assert_eq!(core.status().focus, Machine::Mac);

    // 5. Devolve por hotkey (Ctrl+Alt+2): o Mac solta o Option e volta a poder assumir.
    pc.key(CTRL, true);
    pc.key(ALT, true);
    pc.key(DIGIT_2, true);
    wait_until("PC devolveu", 3000, || !pc.remote.load(Ordering::SeqCst)).await;
    assert!(platform.injected.lock().unwrap().contains(&key(ALT, false)));
    // O Mac perdeu o foco para o PC e devolveu: o clipboard foi oferecido uma vez, ao perder.
    wait_until("ClipboardSwitch ao perder o foco", 3000, || {
        media.0.load(Ordering::SeqCst) == 1
    })
    .await;
    pc.key(ALT, false);
    pc.key(CTRL, false);
    tokio::time::sleep(Duration::from_millis(100)).await;
    move_to_edge(&platform, 400);
    wait_until("Mac assumiu o PC", 3000, || {
        pc.saw(|m| matches!(m, Message::FocusEnter { .. }))
    })
    .await;
    wait_until("foco no Windows", 2000, || {
        core.status().focus == Machine::Windows
    })
    .await;

    // 6. Com o Mac controlando o PC, o PC não assume: o Mac recusa o FocusEnter com FocusReturn.
    let returns = pc.count(|m| matches!(m, Message::FocusReturn { .. }));
    let injected = platform.injected.lock().unwrap().len();
    pc.push_left_edge();
    wait_until("Mac recusou", 3000, || {
        pc.count(|m| matches!(m, Message::FocusReturn { .. })) > returns
    })
    .await;
    wait_until("PC voltou", 3000, || !pc.remote.load(Ordering::SeqCst)).await;
    assert_eq!(platform.injected.lock().unwrap().len(), injected);
    assert_eq!(core.status().focus, Machine::Windows);
    core.shutdown();
}
