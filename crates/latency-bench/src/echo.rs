//! Lado que devolve: reflete cada MouseMove do canal rápido e cada `Input` do confiável.

use std::future::Future;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use relay_core::crypto::{LocalDevice, MemoryKeyStore, load_or_create_identity};
use relay_core::net::{FastEvent, FastSender, ListenerConfig, SessionEvent, SessionListener};
use relay_core::types::{InputEvent, Message, Ports};

use crate::cli::EchoOpts;

type SharedProducer = Arc<Mutex<Option<FastSender>>>;

/// Escuta com identidade efêmera; devolve o listener já com o refletor do canal rápido ligado.
pub async fn start_listener(
    bind_ip: IpAddr,
    ports: Ports,
    peers: Vec<relay_core::types::PairedPeer>,
) -> Result<SessionListener, Box<dyn std::error::Error>> {
    let identity = load_or_create_identity(&MemoryKeyStore::default())?;
    let device = LocalDevice {
        name: "relay-bench-echo".into(),
        public_key: identity.public,
        mac_address: [0; 6],
    };
    let mut cfg = ListenerConfig::new(identity, device, peers, ports);
    cfg.bind_ip = bind_ip;
    let producer: SharedProducer = Arc::new(Mutex::new(None));
    let cb_producer = producer.clone();
    let on_fast = Box::new(move |ev: FastEvent| {
        if let (InputEvent::MouseMove { .. }, Ok(mut g)) = (ev.event, cb_producer.lock()) {
            if let Some(p) = g.as_mut() {
                let _ = p.push(ev.event);
            }
        }
    });
    let mut listener = SessionListener::start(cfg, on_fast).await?;
    *producer.lock().unwrap_or_else(|e| e.into_inner()) = listener.take_input_producer();
    Ok(listener)
}

/// Atende eventos até `stop`; reabre o código de pareamento a cada uso. Devolve o listener.
pub async fn echo_loop(
    mut listener: SessionListener,
    code: Option<&str>,
    stop: impl Future<Output = ()>,
) -> SessionListener {
    const TTL: Duration = Duration::from_secs(3600);
    if let Some(c) = code {
        listener.open_pairing(c.to_string(), TTL);
    }
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => return listener,
            ev = listener.recv_event() => match ev {
                None => return listener,
                Some(SessionEvent::Message(m @ Message::Input { .. })) => {
                    let _ = listener.send(m).await;
                }
                Some(SessionEvent::Paired(p)) => {
                    eprintln!("pareado com {}", p.name);
                    listener.add_peer(p);
                    if let Some(c) = code {
                        listener.open_pairing(c.to_string(), TTL);
                    }
                }
                Some(SessionEvent::PairFailed(why)) => {
                    eprintln!("pareamento falhou: {why}");
                    if let Some(c) = code {
                        listener.open_pairing(c.to_string(), TTL);
                    }
                }
                Some(SessionEvent::Connected(p)) => eprintln!("sessão com {}", p.name),
                Some(_) => {}
            },
        }
    }
}

pub async fn run_echo(o: EchoOpts) -> Result<(), Box<dyn std::error::Error>> {
    let ports = Ports {
        control: o.port_control,
        input: o.port_input,
        ..Ports::default()
    };
    let listener = start_listener(IpAddr::from([0, 0, 0, 0]), ports, Vec::new()).await?;
    eprintln!(
        "echo escutando: controle {} / input {}; código {} (Ctrl+C encerra)",
        listener.local_control_addr(),
        listener.local_input_addr(),
        o.code
    );
    let listener = echo_loop(listener, Some(&o.code), async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await;
    listener.shutdown().await;
    Ok(())
}
