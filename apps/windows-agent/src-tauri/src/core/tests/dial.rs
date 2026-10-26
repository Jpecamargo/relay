//! dono: I11
//! O PC como Dialer (D41): pareamento discando, discagem pelo mDNS, sessão duplicada e a
//! compatibilidade com o Mac 0.2 (que só disca). O "Mac" aqui é um `SessionListener` real.

use relay_core::net::ListenerConfig;
use relay_core::node::session::keep_new_session;
use relay_core::types::{Caps, Os, RemoteCommand};

use super::super::dialing::local_caps;
use super::*;

struct OkExec;

impl relay_core::platform::CommandExecutor for OkExec {
    fn execute(&mut self, id: u32, _: &RemoteCommand) -> relay_core::types::CommandResult {
        relay_core::types::CommandResult::ok(id, None)
    }
}

/// Mac 0.3 que escuta: `Controller` do core sobre uma `SessionListener`.
struct MacL {
    listener: SessionListener,
    ctl: Controller<NopCap, NopCursor, NopInj>,
    control: Consumer<Message>,
    received: Vec<Message>,
}

use relay_core::net::SessionListener;

/// Escuta em portas vizinhas (UDP = controle + 1), como o PC espera ao discar com `ports.input = 0`.
async fn mac_listening(mac: &StaticKeypair, peers: Vec<PairedPeer>) -> MacL {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u16;
    for n in 0..500u16 {
        let port = 20_000 + seed.wrapping_add(n.wrapping_mul(7)) % 30_000;
        let mut cfg = ListenerConfig::new(
            mac.clone(),
            mac_device(mac),
            peers.clone(),
            Ports {
                control: port,
                input: port + 1,
                ..Ports::default()
            },
        );
        cfg.bind_ip = LO;
        cfg.app_version = "0.3.0".into();
        if let Ok(mut listener) = SessionListener::start(cfg, Box::new(|_| {})).await {
            let fast = listener.take_input_producer().unwrap();
            let (ctrl_p, control) = RingBuffer::new(4096);
            let mut cfg = Config::default();
            cfg.transition.dwell_ms = 0;
            cfg.transition.edge_resistance_pt = 0.0;
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
            return MacL {
                listener,
                ctl,
                control,
                received: Vec::new(),
            };
        }
    }
    panic!("sem portas vizinhas livres");
}

impl MacL {
    fn port(&self) -> u16 {
        self.listener.local_control_addr().port()
    }

    /// O PC como o mDNS o veria: o Mac 0.3 anunciando `os` e `v`.
    fn announced(&self, id: &StaticKeypair, version: Option<&str>) -> DiscoveredPeer {
        DiscoveredPeer {
            name: "macbook".into(),
            addrs: vec!["127.0.0.1".into()],
            port: self.port(),
            device_id: device_id_of(&id.public),
            protocol_version: relay_core::types::consts::PROTOCOL_VERSION,
            os: Some(Os::Mac),
            app_version: version.map(Into::into),
        }
    }

    async fn pump_until(&mut self, what: &str, mut cond: impl FnMut(&mut MacL) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            while let Ok(m) = self.control.pop() {
                let _ = self.listener.send(m).await;
            }
            if cond(self) {
                return;
            }
            assert!(Instant::now() < deadline, "tempo esgotado: {what}");
            if let Ok(Some(SessionEvent::Message(m))) =
                timeout(Duration::from_millis(10), self.listener.recv_event()).await
            {
                self.ctl.on_message(m.clone(), Instant::now());
                self.received.push(m);
            }
        }
    }

    fn got(&self, f: impl Fn(&Message) -> bool) -> bool {
        self.received.iter().any(f)
    }
}

impl Agent {
    fn pending_layout_host(&self) -> Option<Host> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.core
            .shared
            .tx
            .send(Cmd::PendingLayoutHost(tx))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap()
    }
}

/// Identidade cujo `device_id` é maior (ou menor) que `than`: define quem disca na hora (D41).
fn identity_ordered(than: &str, greater: bool) -> StaticKeypair {
    loop {
        let k = identity();
        if (device_id_of(&k.public).as_str() > than) == greater {
            return k;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_with_code_makes_the_pc_dial_and_everything_flows_with_the_pc_as_dialer() {
    let agent = start_agent_full(Arc::new(MemoryKeyStore::default()), Some(Box::new(OkExec)));
    let mac_id = identity();
    let mut mac = mac_listening(&mac_id, vec![]).await;
    mac.listener
        .open_pairing("123456".into(), Duration::from_secs(30));

    // Código errado não pareia nem guarda o host.
    let wrong = tokio::task::block_in_place(|| {
        agent.core.pair_with_code(
            "000000".into(),
            Some(format!("127.0.0.1:{}", mac.port())),
            Host::Peer,
        )
    });
    assert!(wrong.is_err());
    assert_eq!(agent.pending_layout_host(), None);
    assert!(agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_none());
    assert!(agent.paired_events().contains(&PairingState::Failed));
    sleep(Duration::from_millis(600)).await; // intervalo mínimo entre tentativas do listener

    tokio::task::block_in_place(|| {
        agent.core.pair_with_code(
            "123456".into(),
            Some(format!("127.0.0.1:{}", mac.port())),
            Host::Peer,
        )
    })
    .expect("pareamento discando");
    assert_eq!(agent.pending_layout_host(), Some(Host::Peer));
    assert!(agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_some());
    assert_eq!(agent.core.status().peer_name.as_deref(), Some("macbook"));

    // O PC disca a sessão: o Mac (listener) recebe Displays e Caps (sem MIC_SINK: sem VB-CABLE).
    mac.pump_until("Displays + Caps do PC", |m| {
        m.got(|x| matches!(x, Message::Displays(d) if d.len() == 1 && d[0].id == "dell"))
            && m.got(|x| matches!(x, Message::Caps(c) if *c == local_caps(false)))
    })
    .await;
    assert!(!local_caps(false).has(Caps::MIC_SINK));
    eventually("conectado", || {
        agent.core.status().status == ConnectionStatus::Connected
    })
    .await;
    assert_eq!(agent.engaged.load(Ordering::SeqCst), 1);

    // Caps do Mac 0.3 chegam ao status.
    let mac_caps = Caps {
        os: Os::Mac,
        features: Caps::MIC_SOURCE,
    };
    mac.listener.send(Message::Caps(mac_caps)).await.unwrap();
    eventually("peer_caps", || {
        agent.core.status().peer_caps == Some(mac_caps)
    })
    .await;
    assert_eq!(agent.core.status().peer_os, Some(Os::Mac));
    assert_eq!(agent.core.status().local_caps, local_caps(false));

    // Input do Mac (TCP + UDP, o probe do PC ensina o endereço ao Mac), teclado e retorno.
    mac.listener
        .send(Message::Displays(vec![mac_display()]))
        .await
        .unwrap();
    mac.listener
        .send(Message::LayoutSync(profile()))
        .await
        .unwrap();
    eventually("layout no config", || {
        agent.core.config().layout.profiles == vec![profile()]
    })
    .await;
    mac.ctl.handle(CapturedEvent::MouseMove {
        dx: 5,
        dy: 0,
        pos: (1439.0, 450.0),
    });
    assert!(mac.ctl.is_remote());
    mac.pump_until("FocusEnter aplicado", |_| {
        agent.core.status().focus == Host::Local
    })
    .await;
    mac.ctl.handle(CapturedEvent::Key {
        code: KeyCode::Scan(KEY_A),
        down: true,
        flags: 0,
        autorepeat: false,
    });
    mac.pump_until("tecla e MouseMove (UDP) injetados", |m| {
        m.ctl.handle(CapturedEvent::MouseMove {
            dx: 10,
            dy: 0,
            pos: (0.0, 0.0),
        });
        let seen = agent.injected();
        seen.contains(&InputEvent::Key {
            scan: KEY_A,
            down: true,
        }) && seen.contains(&InputEvent::MouseMove { dx: 10, dy: 0 })
    })
    .await;

    // Executor: o resultado volta pela sessão discada.
    mac.listener
        .send(Message::Command {
            id: 7,
            cmd: RemoteCommand::GetMetrics,
        })
        .await
        .unwrap();
    mac.pump_until("CommandResult", |m| {
        m.got(|x| matches!(x, Message::CommandResult { id: 7, result } if result.ok))
    })
    .await;

    // Queda: o ponteiro é restaurado, e o par continua salvo (reconecta sozinho).
    mac.listener.force_reconnect();
    eventually("ponteiro restaurado", || {
        agent.live_guards.load(Ordering::SeqCst) == 0
    })
    .await;
    assert!(agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_some());
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pc_dials_a_discovered_peer_and_drops_the_duplicate_it_loses() {
    let agent = start_agent();
    // Quem tem o menor id disca na hora: o Mac fica com id maior que o do agente.
    let mac_id = identity_ordered(agent.core.device_id(), true);
    let peer = pair(&agent, &mac_id).await;
    let mut mac_dialer = connect(&agent, &mac_id, peer.clone()).await;
    mac_dialer
        .pump_until("sessão que o Mac discou", |m| {
            m.got(|x| matches!(x, Message::Displays(_)))
        })
        .await;
    eventually("conectado (Listener)", || {
        agent.core.status().status == ConnectionStatus::Connected
    })
    .await;

    // O Mac também escuta e é visto pelo mDNS: o PC (menor id) disca e a sessão dele vence.
    let mut mac = mac_listening(&mac_id, vec![peer]).await;
    agent
        .browse
        .send(DiscoveredPeer {
            // Outro par no ar não é discado.
            device_id: "outro".into(),
            ..mac.announced(&mac_id, Some("0.3.0"))
        })
        .unwrap();
    agent
        .browse
        .send(mac.announced(&mac_id, Some("0.3.0")))
        .unwrap();
    mac.pump_until("sessão discada pelo PC", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    // Existente = a que o Mac discou; nova = a do PC (menor id): a nova vence.
    assert!(keep_new_session(
        &device_id_of(&mac_id.public),
        agent.core.device_id()
    ));
    // O dialer do Mac (maior id) é o descartado: recebe Bye e a sessão dele cai.
    let mut status = mac_dialer.dialer.status();
    timeout(Duration::from_secs(5), async {
        while *status.borrow_and_update() == ConnectionStatus::Connected {
            status.changed().await.unwrap();
        }
    })
    .await
    .expect("a sessão perdedora recebeu Bye");
    mac_dialer.dialer.shutdown().await;
    sleep(Duration::from_millis(300)).await;
    // Fica a sessão do PC, e o status segue Connected.
    assert_eq!(agent.core.status().status, ConnectionStatus::Connected);
    assert_eq!(agent.core.list_discovered().len(), 2);
    mac.listener
        .send(Message::Displays(vec![mac_display()]))
        .await
        .unwrap();
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn unpaired_pc_does_not_dial_what_it_sees() {
    let agent = start_agent();
    let mac_id = identity();
    let mac = mac_listening(&mac_id, vec![]).await;
    agent.browse.send(mac.announced(&mac_id, None)).unwrap();
    eventually("par visto", || !agent.core.list_discovered().is_empty()).await;
    sleep(Duration::from_millis(500)).await;
    assert_ne!(agent.core.status().status, ConnectionStatus::Connected);
    assert_ne!(agent.core.status().status, ConnectionStatus::Connecting);
    assert_eq!(agent.core.status().peer_os, None);
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mac_02_that_never_announces_only_listens_and_gets_the_02_baseline() {
    let agent = start_agent();
    let mac_id = identity();
    let peer = pair(&agent, &mac_id).await;
    // Sem Caps nem versão: um Mac 0.2.
    let mut mac = connect_as(&agent, &mac_id, peer, "").await;
    mac.pump_until("conectado", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    sleep(Duration::from_millis(300)).await;
    let status = agent.core.status();
    assert_eq!(status.peer_os, Some(Os::Mac));
    assert_eq!(status.peer_caps, Some(Caps::baseline(Os::Mac)));
    assert!(
        !mac.got(|x| matches!(x, Message::Caps(_) | Message::RequestUpdate { .. })),
        "um 0.2 derrubaria a sessão ao ver variante nova"
    );
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_pair_shows_os_and_caps_offline_and_reconnects_without_pairing_again() {
    let agent = start_agent();
    let mac_id = identity();
    let peer = pair(&agent, &mac_id).await;
    eventually("peer salvo", || {
        agent.store.get(ACTIVE_PEER_ACCOUNT).unwrap().is_some()
    })
    .await;
    agent.core.shutdown();
    let store = agent.store.clone();
    drop(agent);

    // Mesmas contas do keystore (active-peer): o par 0.2 conecta sem parear de novo.
    let again = start_agent_with(store);
    eventually("pareado ao subir", || {
        again.core.status().peer_name.is_some()
    })
    .await;
    let offline = again.core.status();
    assert_ne!(offline.status, ConnectionStatus::Connected);
    assert_eq!(offline.peer_os, Some(Os::Mac));
    assert_eq!(offline.peer_caps, Some(Caps::baseline(Os::Mac)));
    let mut mac = connect_as(&again, &mac_id, peer, "").await;
    mac.pump_until("conectado sem reparear", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    assert!(again.paired_events().is_empty(), "nenhum pareamento novo");
    again.core.shutdown();
}

#[test]
fn handshake_sends_caps_to_a_03_peer_and_asks_it_to_update_only_if_it_can() {
    use super::super::pump::handshake_messages;
    let caps = local_caps(true);
    // Mac 0.3 atrás: Caps e pedido de atualização (D49).
    assert_eq!(
        handshake_messages(caps, "0.3.1", Some("0.3.0"), Os::Mac),
        vec![
            Message::Caps(caps),
            Message::RequestUpdate {
                version: "0.3.1".into()
            }
        ]
    );
    // Mesma versão: só Caps.
    assert_eq!(
        handshake_messages(caps, "0.3.0", Some("0.3.0"), Os::Mac),
        vec![Message::Caps(caps)]
    );
    // Mac 0.2: nada (não entende Caps nem RequestUpdate; ele mesmo mostra "atualize").
    assert!(handshake_messages(caps, "0.3.0", Some("0.2.9"), Os::Mac).is_empty());
    assert!(handshake_messages(caps, "0.3.0", None, Os::Mac).is_empty());
}
