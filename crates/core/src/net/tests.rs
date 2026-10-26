use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::{sleep, timeout};

use super::*;
use crate::crypto::{
    LocalDevice, MemoryKeyStore, StaticKeypair, load_or_create_identity, noise_accept, write_frame,
};
use crate::protocol::FrameWriter;
use crate::types::{ConnectionStatus, InputEvent, Ports};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn ident() -> StaticKeypair {
    load_or_create_identity(&MemoryKeyStore::default()).unwrap()
}

fn ephemeral() -> Ports {
    Ports {
        control: 0,
        input: 0,
        ..Ports::default()
    }
}

fn peer_of(k: &StaticKeypair, name: &str) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&k.public),
        name: name.into(),
        public_key: k.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    }
}

fn device_of(k: &StaticKeypair, name: &str) -> LocalDevice {
    LocalDevice {
        name: name.into(),
        public_key: k.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    }
}

fn noop() -> Box<dyn FnMut(FastEvent) + Send> {
    Box::new(|_| {})
}

struct Rig {
    mac: StaticKeypair,
    pc: StaticKeypair,
}

impl Rig {
    fn new() -> Self {
        Self {
            mac: ident(),
            pc: ident(),
        }
    }

    fn listener_cfg(&self, ports: Ports) -> ListenerConfig {
        let mut c = ListenerConfig::new(
            self.pc.clone(),
            device_of(&self.pc, "PC"),
            vec![peer_of(&self.mac, "Mac")],
            ports,
        );
        c.bind_ip = LO;
        c
    }

    fn dialer_cfg(&self, l: &SessionListener) -> DialerConfig {
        let remote = Ports {
            control: l.local_control_addr().port(),
            input: l.local_input_addr().port(),
            ..Ports::default()
        };
        let mut c = DialerConfig::new(
            self.mac.clone(),
            peer_of(&self.pc, "PC"),
            LO,
            remote,
            ephemeral(),
        );
        c.bind_ip = LO;
        c
    }
}

async fn wait_status(rx: &mut watch::Receiver<ConnectionStatus>, want: ConnectionStatus, ms: u64) {
    let r = timeout(Duration::from_millis(ms), async {
        loop {
            if *rx.borrow_and_update() == want {
                return;
            }
            rx.changed().await.unwrap();
        }
    })
    .await;
    assert!(
        r.is_ok(),
        "status {want:?} não veio em {ms} ms (agora {:?})",
        *rx.borrow()
    );
}

async fn wait_connections(s: &Session, n: u64, ms: u64) {
    let t = Instant::now();
    while s.connections() < n {
        assert!(
            t.elapsed() < Duration::from_millis(ms),
            "conexão {n} não veio"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn pair_then_session_with_nodelay_stats_and_one_hello_version() {
    let rig = Rig::new();
    // O agente ainda não conhece o Mac.
    let mut cfg = rig.listener_cfg(ephemeral());
    cfg.peers.clear();
    let mut listener = SessionListener::start(cfg, noop()).await.unwrap();
    listener.open_pairing("012345".into(), PAIRING_CODE_TTL);
    let addr = listener.local_control_addr();

    let me = device_of(&rig.mac, "Mac");
    let peer = SessionDialer::pair(addr, "012345", &me).await.unwrap();
    assert_eq!(peer.name, "PC");
    assert_eq!(peer.public_key, rig.pc.public);
    match listener.recv_event().await {
        Some(SessionEvent::Paired(p)) => assert_eq!(p.public_key, rig.mac.public),
        other => panic!("esperava Paired, veio {other:?}"),
    }

    // Agora, a sessão (primeiro frame = Session).
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.peer = peer;
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 3000).await;
    wait_status(&mut listener.status(), ConnectionStatus::Connected, 3000).await;
    assert_eq!(dialer.tcp_nodelay(), Some(true));
    assert_eq!(listener.tcp_nodelay(), Some(true));

    // Chaves de áudio: as duas pontas concordam (send de um = recv do outro) e IP do par.
    let (da, la) = (dialer.audio_link().unwrap(), listener.audio_link().unwrap());
    assert_eq!(da.mac_to_pc.send, la.mac_to_pc.recv);
    assert!(da.remote_ip.is_loopback() && la.remote_ip.is_loopback());

    // Estatísticas: depois de uns Pings, rtt > 0 e valores finitos.
    let mut stats = dialer.stats();
    let s = timeout(Duration::from_secs(3), async {
        loop {
            stats.changed().await.unwrap();
            let s = *stats.borrow();
            if s.rtt_ms > 0.0 {
                return s;
            }
        }
    })
    .await
    .expect("rtt publicado");
    assert!(s.rtt_ms.is_finite() && s.jitter_ms.is_finite() && s.loss_pct.is_finite());
    assert!(s.rtt_ms > 0.0);
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn wrong_code_does_not_burn_the_code_but_three_wrong_ones_invalidate_it() {
    let rig = Rig::new();
    let mut cfg = rig.listener_cfg(ephemeral());
    cfg.peers.clear();
    let mut listener = SessionListener::start(cfg, noop()).await.unwrap();
    listener.open_pairing("111111".into(), PAIRING_CODE_TTL);
    let addr = listener.local_control_addr();
    let me = device_of(&rig.mac, "Mac");
    let retry = control::PAIR_RETRY_DELAY + Duration::from_millis(150);

    // Um estranho erra uma vez: o código legítimo continua valendo.
    assert!(SessionDialer::pair(addr, "222222", &me).await.is_err());
    assert!(matches!(
        listener.recv_event().await,
        Some(SessionEvent::PairFailed(_))
    ));
    sleep(retry).await;
    let peer = SessionDialer::pair(addr, "111111", &me).await.unwrap();
    assert_eq!(peer.public_key, rig.pc.public);
    assert!(matches!(
        listener.recv_event().await,
        Some(SessionEvent::Paired(_))
    ));

    // Sucesso consome: o mesmo código não serve de novo.
    sleep(retry).await;
    assert!(SessionDialer::pair(addr, "111111", &me).await.is_err());

    // Três erradas invalidam; a certa depois também falha.
    listener.open_pairing("333333".into(), PAIRING_CODE_TTL);
    for _ in 0..control::MAX_PAIR_FAILURES {
        sleep(retry).await;
        assert!(SessionDialer::pair(addr, "000000", &me).await.is_err());
    }
    sleep(retry).await;
    assert!(SessionDialer::pair(addr, "333333", &me).await.is_err());
    listener.shutdown().await;
}

#[tokio::test]
async fn pairing_attempts_are_rate_limited() {
    let rig = Rig::new();
    let mut cfg = rig.listener_cfg(ephemeral());
    cfg.peers.clear();
    let listener = SessionListener::start(cfg, noop()).await.unwrap();
    listener.open_pairing("111111".into(), PAIRING_CODE_TTL);
    let addr = listener.local_control_addr();
    let me = device_of(&rig.mac, "Mac");
    assert!(SessionDialer::pair(addr, "222222", &me).await.is_err());
    // Imediatamente depois, nem o código certo é atendido (atraso entre tentativas).
    assert!(SessionDialer::pair(addr, "111111", &me).await.is_err());
    sleep(control::PAIR_RETRY_DELAY + Duration::from_millis(150)).await;
    assert!(SessionDialer::pair(addr, "111111", &me).await.is_ok());
    listener.shutdown().await;
}

#[tokio::test]
async fn pending_handshakes_are_capped_and_recover() {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;
    let rig = Rig::new();
    let listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let addr = listener.local_control_addr();
    // Enche as vagas com conexões mudas.
    let mut idle = Vec::new();
    for _ in 0..control::MAX_PENDING_HANDSHAKES {
        idle.push(TcpStream::connect(addr).await.unwrap());
    }
    sleep(Duration::from_millis(200)).await;
    // A próxima é fechada na hora pelo agente (EOF), sem esperar o timeout do handshake.
    let mut extra = TcpStream::connect(addr).await.unwrap();
    let mut b = [0u8; 1];
    let r = timeout(Duration::from_millis(500), extra.read(&mut b)).await;
    assert!(matches!(r, Ok(Ok(0)) | Ok(Err(_))), "{r:?}");
    // Liberadas as vagas, uma sessão legítima entra normalmente.
    drop(idle);
    sleep(Duration::from_millis(200)).await;
    let dialer = SessionDialer::start(rig.dialer_cfg(&listener), noop())
        .await
        .unwrap();
    wait_connections(&dialer, 1, 5000).await;
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn unpaired_peer_is_refused() {
    let rig = Rig::new();
    let mut listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let stranger = ident();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.identity = stranger;
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    let ev = timeout(Duration::from_secs(3), listener.recv_event())
        .await
        .expect("evento");
    assert_eq!(ev, Some(SessionEvent::Refused));
    assert_eq!(listener.connections(), 0);
    assert_ne!(*dialer.status().borrow(), ConnectionStatus::Connected);
    assert_ne!(*listener.status().borrow(), ConnectionStatus::Connected);
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn different_protocol_versions_end_in_incompatible_on_both_sides() {
    let rig = Rig::new();
    let listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.protocol_version = PROTOCOL_VERSION + 1;
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    wait_status(
        &mut dialer.status(),
        ConnectionStatus::IncompatibleVersion {
            local: PROTOCOL_VERSION + 1,
            remote: PROTOCOL_VERSION,
        },
        3000,
    )
    .await;
    wait_status(
        &mut listener.status(),
        ConnectionStatus::IncompatibleVersion {
            local: PROTOCOL_VERSION,
            remote: PROTOCOL_VERSION + 1,
        },
        3000,
    )
    .await;
    assert_eq!(dialer.connections(), 0);
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn each_side_learns_the_other_app_version_and_loses_it_when_the_session_ends() {
    let rig = Rig::new();
    let mut lcfg = rig.listener_cfg(ephemeral());
    lcfg.app_version = "2.0.0".into();
    let listener = SessionListener::start(lcfg, noop()).await.unwrap();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.app_version = "1.5.0".into();
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    assert_eq!(dialer.peer_version(), None);
    wait_connections(&dialer, 1, 3000).await;
    wait_connections(&listener, 1, 3000).await;
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 1000).await;
    wait_status(&mut listener.status(), ConnectionStatus::Connected, 1000).await;
    assert_eq!(dialer.peer_version().as_deref(), Some("2.0.0"));
    assert_eq!(listener.peer_version().as_deref(), Some("1.5.0"));
    listener.shutdown().await;
    wait_status(&mut dialer.status(), ConnectionStatus::Reconnecting, 1000).await;
    assert_eq!(dialer.peer_version(), None);
    dialer.shutdown().await;
}

#[tokio::test]
async fn a_peer_that_reports_no_version_still_connects_with_no_peer_version() {
    let rig = Rig::new();
    let listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.app_version = String::new();
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    wait_connections(&listener, 1, 3000).await;
    wait_status(&mut listener.status(), ConnectionStatus::Connected, 1000).await;
    assert_eq!(listener.peer_version(), None);
    assert_eq!(
        dialer.peer_version().as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn dialer_reconnects_in_under_3s_after_the_listener_restarts() {
    let rig = Rig::new();
    let listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let ports = Ports {
        control: listener.local_control_addr().port(),
        input: listener.local_input_addr().port(),
        ..Ports::default()
    };
    let dialer = SessionDialer::start(rig.dialer_cfg(&listener), noop())
        .await
        .unwrap();
    wait_connections(&dialer, 1, 3000).await;

    let t = Instant::now();
    listener.shutdown().await;
    wait_status(&mut dialer.status(), ConnectionStatus::Reconnecting, 1000).await;
    sleep(Duration::from_millis(400)).await;
    let listener = SessionListener::start(rig.listener_cfg(ports), noop())
        .await
        .unwrap();
    wait_connections(&dialer, 2, 3000).await;
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 1000).await;
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn force_reconnect_tears_down_and_rebuilds_the_session() {
    let rig = Rig::new();
    let listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let dialer = SessionDialer::start(rig.dialer_cfg(&listener), noop())
        .await
        .unwrap();
    wait_connections(&dialer, 1, 3000).await;
    dialer.force_reconnect();
    wait_connections(&dialer, 2, 3000).await;
    wait_connections(&listener, 2, 3000).await;
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 1000).await;
    dialer.shutdown().await;
    listener.shutdown().await;
}

/// Agente falso: faz o handshake e o Hello e depois não diz mais nada.
#[tokio::test]
async fn silent_peer_for_over_2s_sends_the_dialer_to_reconnecting() {
    let rig = Rig::new();
    let tcp = TcpListener::bind((LO, 0)).await.unwrap();
    let port = tcp.local_addr().unwrap().port();
    let known = vec![peer_of(&rig.mac, "Mac")];
    let pc = rig.pc.clone();
    let fake = tokio::spawn(async move {
        let (mut s, _) = tcp.accept().await.unwrap();
        let tag = crate::crypto::read_frame(&mut s).await.unwrap();
        assert_eq!(tag, [control::TAG_SESSION]);
        let mut noise = noise_accept(&mut s, &pc, &known).await.unwrap();
        let hello = Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id: device_id_of(&pc.public),
            role: Role::Listener,
            app_version: "fake".into(),
        };
        let (mut frame, mut cipher) = (Vec::new(), Vec::new());
        FrameWriter.encode(&hello, &mut frame).unwrap();
        noise.encrypt(&frame, &mut cipher).unwrap();
        write_frame(&mut s, &cipher).await.unwrap();
        sleep(Duration::from_secs(6)).await; // mudo, conexão aberta
        drop(s);
    });
    let mut cfg = DialerConfig::new(
        rig.mac.clone(),
        peer_of(&rig.pc, "PC"),
        LO,
        Ports {
            control: port,
            input: 9,
            ..Ports::default()
        },
        ephemeral(),
    );
    cfg.bind_ip = LO;
    let dialer = SessionDialer::start(cfg, noop()).await.unwrap();
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 3000).await;
    let t = Instant::now();
    wait_status(&mut dialer.status(), ConnectionStatus::Reconnecting, 4000).await;
    let silent = t.elapsed();
    assert!(
        silent >= Duration::from_millis(1500) && silent <= Duration::from_millis(3000),
        "{silent:?}"
    );
    dialer.shutdown().await;
    fake.abort();
}

#[tokio::test]
async fn loopback_10k_mouse_moves_fast_and_1k_keys_reliable() {
    const MOVES: usize = 10_000;
    const KEYS: usize = 1_000;
    let rig = Rig::new();
    let arrivals: Arc<Mutex<Vec<(i16, u64)>>> = Arc::new(Mutex::new(Vec::with_capacity(MOVES)));
    let sink = arrivals.clone();
    let on_fast: Box<dyn FnMut(FastEvent) + Send> = Box::new(move |ev| {
        if let InputEvent::MouseMove { dx, .. } = ev.event {
            sink.lock().unwrap().push((dx, ev.recv_us));
        }
    });
    let mut listener = SessionListener::start(rig.listener_cfg(ephemeral()), on_fast)
        .await
        .unwrap();
    let back: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let back_sink = back.clone();
    let mut dialer = SessionDialer::start(
        rig.dialer_cfg(&listener),
        Box::new(move |_| *back_sink.lock().unwrap() += 1),
    )
    .await
    .unwrap();
    wait_connections(&dialer, 1, 3000).await;
    wait_connections(&listener, 1, 3000).await;
    assert_eq!(dialer.fast_dropped().0, 0);

    // Eventos recebidos pelo confiável, em ordem.
    let keys_seen: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
    let ks = keys_seen.clone();
    let reader = tokio::spawn(async move {
        while let Some(ev) = listener.recv_event().await {
            if let SessionEvent::Message(Message::Input {
                event: InputEvent::Key { scan, .. },
                ..
            }) = ev
            {
                ks.lock().unwrap().push(scan);
            }
        }
        listener
    });

    let mut ring = dialer.take_input_producer().unwrap();
    let mut sent_at = vec![0u64; MOVES];
    let mut next_key = 0usize;
    for (i, slot) in sent_at.iter_mut().enumerate() {
        // ~1 kHz de verdade seria mais lento; 20 us por evento já é uma rajada.
        let until = Instant::now() + Duration::from_micros(20);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
        *slot = now_us();
        let mut ev = InputEvent::MouseMove {
            dx: i as i16,
            dy: 1,
        };
        while let Err(rtrb::PushError::Full(back)) = ring.push(ev) {
            ev = back;
            std::thread::yield_now();
        }
        if i % (MOVES / KEYS) == 0 && next_key < KEYS {
            let msg = Message::Input {
                seq: next_key as u64,
                ts_us: now_us(),
                event: InputEvent::Key {
                    scan: next_key as u16,
                    down: next_key % 2 == 0,
                },
            };
            dialer.send(msg).await.unwrap();
            next_key += 1;
        }
    }
    assert_eq!(next_key, KEYS);
    // Espera drenar.
    let t = Instant::now();
    while (arrivals.lock().unwrap().len() < MOVES || keys_seen.lock().unwrap().len() < KEYS)
        && t.elapsed() < Duration::from_secs(3)
    {
        sleep(Duration::from_millis(20)).await;
    }

    let keys = keys_seen.lock().unwrap().clone();
    assert_eq!(
        keys,
        (0..KEYS as u16).collect::<Vec<_>>(),
        "Key: 100% e em ordem"
    );
    let got = arrivals.lock().unwrap().clone();
    assert!(
        got.len() * 100 >= MOVES * 99,
        "MouseMove: {} de {MOVES}",
        got.len()
    );
    let mut lat: Vec<u64> = got
        .iter()
        .map(|(i, recv)| recv.saturating_sub(sent_at[*i as usize]))
        .collect();
    lat.sort_unstable();
    let p99 = lat[lat.len() * 99 / 100];
    eprintln!(
        "fast: {} de {MOVES}, p50={}us p99={p99}us",
        got.len(),
        lat[lat.len() / 2]
    );
    if !cfg!(debug_assertions) {
        assert!(p99 < 1000, "p99 {p99} us");
    }
    let fs = dialer.fast_dropped();
    assert_eq!(fs.1, 0, "pacotes sem destino: {fs:?}");

    dialer.shutdown().await;
    reader.abort();
    let _ = back;
}

#[tokio::test]
async fn listener_learns_the_dialer_address_and_sends_fast_back() {
    let rig = Rig::new();
    let mut listener = SessionListener::start(rig.listener_cfg(ephemeral()), noop())
        .await
        .unwrap();
    let got: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = got.clone();
    let dialer = SessionDialer::start(
        rig.dialer_cfg(&listener),
        Box::new(move |ev| {
            if let InputEvent::MouseMove { dx, .. } = ev.event {
                sink.lock().unwrap().push(dx);
            }
        }),
    )
    .await
    .unwrap();
    wait_connections(&dialer, 1, 3000).await;
    wait_connections(&listener, 1, 3000).await;
    // O probe do dialer fixa o destino do agente; dá tempo de chegar.
    sleep(Duration::from_millis(200)).await;
    let mut ring = listener.take_input_producer().unwrap();
    for i in 0..50i16 {
        ring.push(InputEvent::MouseMove { dx: i, dy: 0 }).unwrap();
        sleep(Duration::from_millis(1)).await;
    }
    sleep(Duration::from_millis(200)).await;
    assert!(
        got.lock().unwrap().len() >= 49,
        "{}",
        got.lock().unwrap().len()
    );
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[test]
fn fast_module_does_not_import_tokio() {
    let needle = concat!("to", "kio");
    for (name, src) in [
        ("fast.rs", include_str!("fast.rs")),
        ("stats.rs", include_str!("stats.rs")),
    ] {
        assert!(!src.contains(needle), "{name} não pode usar {needle}");
    }
}

fn sample_display() -> crate::types::DisplayInfo {
    crate::types::DisplayInfo {
        id: "D1".into(),
        name: "Tela".into(),
        machine: crate::types::Machine::Mac,
        origin_px: (0, 0),
        size_px: (1920, 1080),
        scale: 1.0,
        size_mm: None,
        is_primary: true,
        is_builtin: false,
    }
}

fn sample_layout() -> crate::types::LayoutProfile {
    crate::types::LayoutProfile {
        key: Default::default(),
        name: "L".into(),
        displays: vec![],
        windows_detached: false,
    }
}

async fn next_message(s: &mut impl EventSource) -> Message {
    timeout(Duration::from_secs(3), async {
        loop {
            if let Some(SessionEvent::Message(m)) = s.next().await {
                return m;
            }
        }
    })
    .await
    .expect("mensagem não chegou")
}

trait EventSource {
    async fn next(&mut self) -> Option<SessionEvent>;
}
impl EventSource for SessionDialer {
    async fn next(&mut self) -> Option<SessionEvent> {
        self.recv_event().await
    }
}
impl EventSource for SessionListener {
    async fn next(&mut self) -> Option<SessionEvent> {
        self.recv_event().await
    }
}

/// Dialer que se apresenta como 0.2.9, agente 0.3.0.
async fn pair_with_legacy_dialer(rig: &Rig) -> (SessionDialer, SessionListener) {
    let mut lcfg = rig.listener_cfg(ephemeral());
    lcfg.app_version = "0.3.0".into();
    let listener = SessionListener::start(lcfg, noop()).await.unwrap();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.app_version = "0.2.9".into();
    let dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    wait_connections(&dialer, 1, 3000).await;
    wait_connections(&listener, 1, 3000).await;
    wait_status(&mut dialer.status(), ConnectionStatus::Connected, 1000).await;
    wait_status(&mut listener.status(), ConnectionStatus::Connected, 1000).await;
    (dialer, listener)
}

#[tokio::test]
async fn caps_is_not_written_to_a_0_2_9_peer_but_ping_pong_continue() {
    let rig = Rig::new();
    let (mut dialer, listener) = pair_with_legacy_dialer(&rig).await;
    let caps = Message::Caps(crate::types::Caps::baseline(crate::types::Os::Windows));
    listener.send(caps).await.unwrap();
    listener
        .send(Message::Displays(vec![sample_display()]))
        .await
        .unwrap();
    // O primeiro que chega ao 0.2.9 é Displays: Caps foi barrado.
    assert!(matches!(
        next_message(&mut dialer).await,
        Message::Displays(_)
    ));
    // Ping/Pong seguem: o rtt é publicado e a sessão não cai.
    let mut stats = dialer.stats();
    timeout(Duration::from_secs(3), async {
        loop {
            stats.changed().await.unwrap();
            if stats.borrow().rtt_ms > 0.0 {
                return;
            }
        }
    })
    .await
    .expect("rtt");
    assert_eq!(dialer.connections(), 1);
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn caps_reaches_a_0_3_0_peer() {
    let rig = Rig::new();
    let mut lcfg = rig.listener_cfg(ephemeral());
    lcfg.app_version = "0.3.0".into();
    let listener = SessionListener::start(lcfg, noop()).await.unwrap();
    let mut dcfg = rig.dialer_cfg(&listener);
    dcfg.app_version = "0.3.0".into();
    let mut dialer = SessionDialer::start(dcfg, noop()).await.unwrap();
    wait_connections(&dialer, 1, 3000).await;
    wait_connections(&listener, 1, 3000).await;
    wait_status(&mut listener.status(), ConnectionStatus::Connected, 1000).await;
    let caps = Message::Caps(crate::types::Caps::baseline(crate::types::Os::Windows));
    listener.send(caps.clone()).await.unwrap();
    assert_eq!(next_message(&mut dialer).await, caps);
    dialer.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test]
async fn a_node_presenting_as_0_2_9_exchanges_everything_both_ways_without_dropping() {
    let rig = Rig::new();
    let (mut dialer, mut listener) = pair_with_legacy_dialer(&rig).await;
    let input = Message::Input {
        seq: 1,
        ts_us: 2,
        event: InputEvent::Key {
            scan: 30,
            down: true,
        },
    };
    let to_dialer = [
        Message::HelloAck,
        Message::Displays(vec![sample_display()]),
        Message::LayoutSync(sample_layout()),
        input.clone(),
    ];
    for m in &to_dialer {
        listener.send(m.clone()).await.unwrap();
        dialer.send(m.clone()).await.unwrap();
    }
    // HelloAck é absorvido pelo loop; o resto chega, em ordem, nos dois sentidos.
    for want in &to_dialer[1..] {
        assert_eq!(&next_message(&mut dialer).await, want);
        assert_eq!(&next_message(&mut listener).await, want);
    }
    sleep(Duration::from_millis(1200)).await; // alguns Pings
    assert_eq!(dialer.connections(), 1);
    assert_eq!(listener.connections(), 1);
    assert_eq!(*dialer.status().borrow(), ConnectionStatus::Connected);
    dialer.shutdown().await;
    listener.shutdown().await;
}
