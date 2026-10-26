//! dono: I9
//! Sessão simétrica do app Mac em loopback: o PC disca no Mac (escuta), o Mac disca num PC 0.2
//! ou 0.3 descoberto, par salvo no formato da 0.2, Caps nos dois sentidos, sessão duplicada e
//! `RequestUpdate` recebido.

use relay_core::crypto::{StaticKeypair, device_id_of, save_active_peer};
use relay_core::net::{DialerConfig, FastEvent, SessionDialer};
use relay_core::types::{Caps, DiscoveredPeer, Os, PairedPeer};
use tokio::sync::mpsc;

use std::net::SocketAddr;

use super::*;

const V03: &str = "0.3.0";

fn new_identity() -> StaticKeypair {
    load_or_create_identity(&MemoryKeyStore::default()).unwrap()
}

fn peer_of(identity: &StaticKeypair, name: &str, mac: [u8; 6]) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&identity.public),
        name: name.into(),
        public_key: identity.public,
        mac_address: mac,
    }
}

/// Identidade do PC com o `device_id` maior (ou menor) que o do Mac.
fn pc_identity_vs(mac_id: &str, greater: bool) -> StaticKeypair {
    loop {
        let id = new_identity();
        if (device_id_of(&id.public).as_str() > mac_id) == greater {
            return id;
        }
    }
}

/// Mac com o Keychain no formato da 0.2: identidade e `active_peer` já gravados, sem pareamento.
struct Saved {
    platform: Arc<MockPlatform>,
    mac_peer: PairedPeer,
    mac_id: String,
}

/// Mac só com a identidade (o par entra depois, para o teste escolher o `device_id` do PC).
fn new_mac() -> Saved {
    let platform = MockPlatform::new(&Config::default());
    let mac = load_or_create_identity(&*platform.keys).unwrap();
    Saved {
        mac_peer: peer_of(&mac, "Mac de teste", [0; 6]),
        mac_id: device_id_of(&mac.public),
        platform,
    }
}

impl Saved {
    /// Grava o PC como par ativo, como a 0.2 gravava (`active_peer` + `peer:<id>`).
    fn save_pc(&self, pc: &StaticKeypair) {
        let pc_peer = peer_of(pc, "PC de teste", [1, 2, 3, 4, 5, 6]);
        save_active_peer(&*self.platform.keys, "active_peer", &pc_peer).unwrap();
    }
}

fn saved_pair(pc: &StaticKeypair, cfg: &Config) -> Saved {
    let saved = new_mac();
    config::save(&saved.platform.config_path, cfg).unwrap();
    saved.save_pc(pc);
    saved
}

fn opts(version: Option<&str>) -> CoreOptions {
    CoreOptions {
        app_version: version.map(Into::into),
        ..options()
    }
}

fn start(saved: &Saved, rec: &Arc<Rec>, version: Option<&str>) -> Arc<AppCore<MockPlatform>> {
    Arc::new(AppCore::start(
        saved.platform.clone(),
        rec.clone(),
        Arc::new(NoMedia),
        opts(version),
        Handle::current(),
    ))
}

// ---- PC falso que disca no Mac (0.3) ----

struct FakePc {
    seen: Arc<Mutex<Vec<Message>>>,
    fast: Arc<Mutex<Vec<InputEvent>>>,
    connected: Arc<AtomicBool>,
    out: mpsc::UnboundedSender<Message>,
}

impl FakePc {
    async fn dial(
        identity: StaticKeypair,
        mac: PairedPeer,
        (control, input): (u16, u16),
        version: &str,
    ) -> Self {
        let seen: Arc<Mutex<Vec<Message>>> = Arc::default();
        let fast: Arc<Mutex<Vec<InputEvent>>> = Arc::default();
        let connected = Arc::new(AtomicBool::new(false));
        let mut cfg = DialerConfig::new(
            identity,
            mac,
            LO,
            Ports {
                control,
                input,
                ..Ports::default()
            },
            Ports {
                input: 0,
                ..Ports::default()
            },
        );
        cfg.bind_ip = LO;
        cfg.app_version = version.into();
        let sink_fast = fast.clone();
        let dialer = SessionDialer::start(
            cfg,
            Box::new(move |e: FastEvent| sink_fast.lock().unwrap().push(e.event)),
        )
        .await
        .unwrap();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let (s, c) = (seen.clone(), connected.clone());
        // A `Session` não é `Sync`: roda numa thread bloqueante, como a bomba do app.
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            rt.block_on(async move {
            let mut dialer = dialer;
            let mut status = dialer.status();
            loop {
                tokio::select! {
                    ev = dialer.recv_event() => match ev {
                        Some(relay_core::net::SessionEvent::Message(m)) => s.lock().unwrap().push(m),
                        Some(_) => {}
                        None => break,
                    },
                    Some(m) = out_rx.recv() => { let _ = dialer.send(m).await; }
                    r = status.changed() => {
                        if r.is_err() { break; }
                        c.store(
                            *status.borrow_and_update() == ConnectionStatus::Connected,
                            Ordering::SeqCst,
                        );
                    }
                }
            }
        })
        });
        Self {
            seen,
            fast,
            connected,
            out,
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.seen.lock().unwrap().iter().any(f)
    }
}

// ---- PC falso que escuta (0.2 ou 0.3) ----

struct FakeListener {
    control: u16,
    input: u16,
    seen: Arc<Mutex<Vec<Message>>>,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeListener {
    fn start(identity: StaticKeypair, mac: PairedPeer, version: &str) -> Self {
        let seen: Arc<Mutex<Vec<Message>>> = Arc::default();
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (s, c, st, v) = (
            seen.clone(),
            connected.clone(),
            stop.clone(),
            version.to_owned(),
        );
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let device = LocalDevice {
                    name: "PC de teste".into(),
                    public_key: identity.public,
                    mac_address: [1, 2, 3, 4, 5, 6],
                };
                let mut cfg = ListenerConfig::new(
                    identity,
                    device,
                    vec![mac],
                    Ports {
                        control: 0,
                        input: 0,
                        ..Ports::default()
                    },
                );
                cfg.bind_ip = LO;
                cfg.app_version = v;
                let mut l = SessionListener::start(cfg, Box::new(|_| {})).await.unwrap();
                tx.send((l.local_control_addr().port(), l.local_input_addr().port()))
                    .unwrap();
                let mut status = l.status();
                while !st.load(Ordering::SeqCst) {
                    tokio::select! {
                        ev = l.recv_event() => match ev {
                            None => break,
                            Some(SessionEvent::Message(m)) => s.lock().unwrap().push(m),
                            Some(_) => {}
                        },
                        _ = status.changed() => {
                            c.store(
                                *status.borrow_and_update() == ConnectionStatus::Connected,
                                Ordering::SeqCst,
                            );
                        }
                        () = tokio::time::sleep(Duration::from_millis(20)) => {}
                    }
                }
                l.shutdown().await;
            });
        });
        let (control, input) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            control,
            input,
            seen,
            connected,
            stop,
            thread: Some(thread),
        }
    }

    fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.seen.lock().unwrap().iter().any(f)
    }
}

impl Drop for FakeListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn cfg_for(l: &FakeListener) -> Config {
    let mut cfg = Config::default();
    cfg.network.ports = Ports {
        control: l.control,
        input: l.input,
        ..Ports::default()
    };
    cfg
}

fn discovered(id: &str, l: &FakeListener, version: Option<&str>) -> DiscoveredPeer {
    DiscoveredPeer {
        name: "PC de teste".into(),
        addrs: vec!["127.0.0.1".into()],
        port: l.control,
        device_id: id.into(),
        protocol_version: relay_core::types::consts::PROTOCOL_VERSION,
        os: version.map(|_| Os::Windows),
        app_version: version.map(Into::into),
    }
}

async fn listen_ports(platform: &MockPlatform) -> (u16, u16) {
    wait_until("anúncio mDNS com as portas de escuta", 3000, || {
        !platform.advertised.lock().unwrap().is_empty()
    })
    .await;
    let (_, c, i) = platform.advertised.lock().unwrap()[0].clone();
    (c, i)
}

// ---- testes ----

/// O PC (0.3) disca no Mac: o par vem do Keychain no formato da 0.2, sem parear. O Mac anuncia as
/// portas reais, troca Caps, e o canal rápido volta ao endereço que ele aprendeu da sessão.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o_pc_disca_no_mac_troca_caps_e_o_cursor_cruza_pelo_endereco_aprendido() {
    let pc_id = new_identity();
    let saved = saved_pair(&pc_id, &Config::default());
    let rec = Arc::new(Rec::default());
    let core = start(&saved, &rec, Some(V03));
    let ports = listen_ports(&saved.platform).await;
    assert_eq!(
        saved.platform.advertised.lock().unwrap()[0].0,
        saved.mac_id,
        "anuncia o device_id do Mac"
    );
    // Offline, com o par salvo: o painel já sabe que ele é Windows.
    wait_until("peer salvo no estado", 3000, || {
        core.status().peer_name.is_some()
    })
    .await;
    assert_eq!(core.status().peer_os, Some(Os::Windows));
    assert_eq!(core.status().peer_caps, Some(Caps::baseline(Os::Windows)));

    let pc = FakePc::dial(pc_id, saved.mac_peer.clone(), ports, V03).await;
    wait_until("status Connected", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    assert_eq!(core.status().peer_name.as_deref(), Some("PC de teste"));

    // Caps: o Mac manda o dele depois do Hello e guarda o que o PC anuncia.
    wait_until("Caps do Mac no PC", 3000, || {
        pc.saw(|m| *m == Message::Caps(Caps::baseline(Os::Mac)))
    })
    .await;
    let theirs = Caps {
        os: Os::Windows,
        features: Caps::COMMANDS | Caps::WOL_TARGET,
    };
    pc.out.send(Message::Caps(theirs)).unwrap();
    wait_until("peer_caps do status", 3000, || {
        core.status().peer_caps == Some(theirs)
    })
    .await;
    assert_eq!(core.status().peer_os, Some(Os::Windows));
    assert!(
        rec.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.peer_caps == Some(theirs)),
        "relay:status emitido com os Caps"
    );

    // O Mac cruza para o PC que o discou: FocusEnter (TCP) e MouseMove (UDP no endereço aprendido).
    pc.out
        .send(Message::Displays(vec![dell_display()]))
        .unwrap();
    wait_until("Displays do PC no Mac", 3000, || {
        core.displays().peer.len() == 1
    })
    .await;
    core.save_layout(core.layout()).unwrap();
    move_to_edge(&saved.platform, 400);
    wait_until("FocusEnter no PC", 3000, || {
        pc.saw(|m| matches!(m, Message::FocusEnter { .. }))
    })
    .await;
    let t = Instant::now();
    while !pc
        .fast
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, InputEvent::MouseMove { .. }))
    {
        saved.platform.push(CapturedEvent::MouseMove {
            dx: 7,
            dy: 3,
            pos: (1439.0, 450.0),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(t.elapsed() < Duration::from_secs(4), "MouseMove não chegou");
    }

    // RequestUpdate do PC (mais novo que este Mac) vira `relay:update-available`.
    pc.out
        .send(Message::RequestUpdate {
            version: "9.9.9".into(),
        })
        .unwrap();
    wait_until("relay:update-available", 3000, || {
        rec.updates.lock().unwrap().as_slice() == ["9.9.9"]
    })
    .await;
    // Um pedido para versão que este Mac já alcançou é ignorado.
    pc.out
        .send(Message::RequestUpdate {
            version: "0.0.1".into(),
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(rec.updates.lock().unwrap().len(), 1);
    core.shutdown();
}

/// Mac com o par salvo e um PC que escuta, com as portas dele já no config do Mac.
fn mac_and_listener(pc: StaticKeypair, version: &str) -> (Saved, FakeListener) {
    let saved = saved_pair(&pc, &Config::default());
    let agent = FakeListener::start(pc, saved.mac_peer.clone(), version);
    config::save(&saved.platform.config_path, &cfg_for(&agent)).unwrap();
    (saved, agent)
}

/// PC 0.2 (anuncia sem `v`): o Mac disca na hora, como na 0.2; o par do Keychain 0.2 conecta sem
/// re-parear; nenhuma variante da 0.3 vai ao PC; o SO e os Caps vêm do papel legado.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pc_02_sem_v_e_discado_como_na_02_com_o_keychain_antigo() {
    let pc_id = new_identity();
    let pc_device = device_id_of(&pc_id.public);
    let (saved, agent) = mac_and_listener(pc_id, "0.1.9");
    saved
        .platform
        .browse_peers
        .lock()
        .unwrap()
        .push(discovered(&pc_device, &agent, None));
    let rec = Arc::new(Rec::default());
    // O Mac é 0.3 (Hello 0.3.0), mas o PC não anuncia `v`: DialNow, sem esperar o fallback.
    let core = start(&saved, &rec, Some(V03));
    wait_until("status Connected (Mac disca)", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    assert!(agent.connected.load(Ordering::SeqCst));
    let st = core.status();
    assert_eq!(st.peer_name.as_deref(), Some("PC de teste"));
    assert_eq!(st.peer_os, Some(Os::Windows));
    assert_eq!(st.peer_caps, Some(Caps::baseline(Os::Windows)));
    assert_eq!(st.peer_version.as_deref(), Some("0.1.9"));
    // PC atrás deste Mac: o pedido de atualização segue como na 0.2.
    wait_until("RequestUpdate no PC", 3000, || {
        agent.saw(|m| matches!(m, Message::RequestUpdate { .. }))
    })
    .await;
    // Peer 0.2 derrubaria a sessão ao ver Caps: nada da 0.3 sai para ele.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!agent.saw(|m| matches!(m, Message::Caps(_))));
    assert!(agent.connected.load(Ordering::SeqCst));
    core.shutdown();
}

/// Entre dois 0.3 disca o de `device_id` menor. O Mac de id maior espera `DIAL_FALLBACK` sem
/// sessão: não disca.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_de_id_maior_espera_o_pc_03_discar() {
    let saved = new_mac();
    let pc = pc_identity_vs(&saved.mac_id, false);
    let pc_device = device_id_of(&pc.public);
    saved.save_pc(&pc);
    let agent = FakeListener::start(pc, saved.mac_peer.clone(), V03);
    config::save(&saved.platform.config_path, &cfg_for(&agent)).unwrap();
    saved
        .platform
        .browse_peers
        .lock()
        .unwrap()
        .push(discovered(&pc_device, &agent, Some(V03)));
    let rec = Arc::new(Rec::default());
    let core = start(&saved, &rec, Some(V03));
    wait_until("PC descoberto", 3000, || !core.discovered().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!agent.connected.load(Ordering::SeqCst), "o Mac esperou");
    assert_ne!(core.status().status, ConnectionStatus::Connected);
    core.shutdown();
}

/// Sendo o de `device_id` menor, o Mac disca na hora e os dois lados trocam Caps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mac_de_id_menor_disca_na_hora_o_pc_03() {
    let saved = new_mac();
    let pc = pc_identity_vs(&saved.mac_id, true);
    let pc_device = device_id_of(&pc.public);
    saved.save_pc(&pc);
    let agent = FakeListener::start(pc, saved.mac_peer.clone(), V03);
    config::save(&saved.platform.config_path, &cfg_for(&agent)).unwrap();
    saved
        .platform
        .browse_peers
        .lock()
        .unwrap()
        .push(discovered(&pc_device, &agent, Some(V03)));
    let rec = Arc::new(Rec::default());
    let core = start(&saved, &rec, Some(V03));
    wait_until("status Connected", 8000, || {
        core.status().status == ConnectionStatus::Connected
    })
    .await;
    wait_until("Caps do Mac no PC", 3000, || {
        agent.saw(|m| *m == Message::Caps(Caps::baseline(Os::Mac)))
    })
    .await;
    assert_eq!(core.status().peer_os, Some(Os::Windows));
    core.shutdown();
}

/// Sessão duplicada: o PC já está ligado ao Mac que discou e discou também no Mac. Fica a sessão
/// cujo dialer tem o `device_id` menor, e a outra recebe Bye.
async fn duplicada(pc_is_lower: bool) {
    let saved = new_mac();
    let pc = pc_identity_vs(&saved.mac_id, !pc_is_lower);
    saved.save_pc(&pc);
    let pc_clone = pc.clone();
    let agent = FakeListener::start(pc, saved.mac_peer.clone(), V03);
    let mut cfg = cfg_for(&agent);
    // Sem mDNS: o Mac disca pelo endereço manual, como o usuário configuraria.
    cfg.network.manual_address = Some(format!("127.0.0.1:{}", agent.control));
    config::save(&saved.platform.config_path, &cfg).unwrap();
    let rec = Arc::new(Rec::default());
    let core = start(&saved, &rec, Some(V03));
    wait_until("Mac conectado ao PC (discou)", 8000, || {
        core.status().status == ConnectionStatus::Connected
            && agent.connected.load(Ordering::SeqCst)
    })
    .await;
    let ports = listen_ports(&saved.platform).await;
    let pc = FakePc::dial(pc_clone, saved.mac_peer.clone(), ports, V03).await;
    if pc_is_lower {
        // Vale a sessão que o PC discou: a que o Mac discou recebe Bye.
        wait_until("sessão discada pelo Mac encerrada (Bye)", 8000, || {
            !agent.connected.load(Ordering::SeqCst)
        })
        .await;
        wait_until("sessão do PC de pé", 8000, || {
            pc.connected.load(Ordering::SeqCst)
                && core.status().status == ConnectionStatus::Connected
        })
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(pc.connected.load(Ordering::SeqCst));
        assert!(
            !agent.connected.load(Ordering::SeqCst),
            "o Mac não rediscou"
        );
    } else {
        // Vale a que o Mac discou: a do PC recebe Bye e fica fora.
        wait_until("sessão do PC recusada", 8000, || {
            pc.saw(|_| true) || !pc.connected.load(Ordering::SeqCst)
        })
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(agent.connected.load(Ordering::SeqCst));
        assert_eq!(core.status().status, ConnectionStatus::Connected);
    }
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessao_duplicada_vale_a_do_dialer_de_id_menor_pc() {
    duplicada(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessao_duplicada_vale_a_do_dialer_de_id_menor_mac() {
    duplicada(false).await;
}

/// `new_pairing_code`: o Mac mostra o código (Listener), o PC digita (Dialer) e vira par.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn novo_codigo_pareia_o_pc_que_o_digita() {
    let saved = new_mac();
    let rec = Arc::new(Rec::default());
    let core = start(&saved, &rec, Some(V03));
    let (control, _) = listen_ports(&saved.platform).await;
    let pc = new_identity();
    let code = core.new_pairing_code().unwrap();
    assert_eq!(code.code.len(), 6);
    assert!(code.expires_at > 0);
    wait_until("relay:pairing code", 3000, || {
        rec.pairing
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Code && e.code.as_deref() == Some(&code.code))
    })
    .await;
    assert_eq!(core.status().status, ConnectionStatus::Pairing);
    let me = LocalDevice {
        name: "PC de teste".into(),
        public_key: pc.public,
        mac_address: [1, 2, 3, 4, 5, 6],
    };
    let wrong = SessionDialer::pair(
        SocketAddr::new(LO, control),
        if code.code == "000000" {
            "111111"
        } else {
            "000000"
        },
        &me,
    )
    .await;
    assert!(wrong.is_err());
    // O listener impõe um intervalo entre tentativas.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let peer = SessionDialer::pair(SocketAddr::new(LO, control), &code.code, &me)
        .await
        .unwrap();
    assert_eq!(peer.name, "Mac de teste");
    wait_until("par gravado no Mac", 3000, || {
        saved.platform.keys.get("active_peer").unwrap().is_some()
    })
    .await;
    assert_eq!(core.status().peer_name.as_deref(), Some("PC de teste"));
    assert!(
        rec.pairing
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == PairingState::Paired)
    );
    // Offline e pareado: o painel já sabe o SO do par.
    assert_eq!(core.status().peer_os, Some(Os::Windows));
    core.shutdown();
}
