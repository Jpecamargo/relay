//! Medição de RTT: canal rápido (UDP, MouseMove com o id em dx/dy) e confiável (TCP, `Input`
//! com o id em `seq`). Os dois são medidos em sequência, cada um com seu histograma.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use relay_core::crypto::{LocalDevice, MemoryKeyStore, load_or_create_identity};
use relay_core::net::{
    DialerConfig, FastEvent, FastSender, Session, SessionDialer, SessionEvent, now_us,
};
use relay_core::types::{ConnectionStatus, InputEvent, Message, Ports};

use crate::cli::{LoopbackOpts, SendOpts};
use crate::echo::{echo_loop, start_listener};
use crate::report::{Summary, to_json};

type Boxed<T> = Result<T, Box<dyn std::error::Error>>;

/// Id reservado do aquecimento (fora da tabela).
const WARMUP_ID: u32 = u32::MAX;
const SETTLE: Duration = Duration::from_secs(1);

/// Tabela de envio/retorno por id. `rtt = 0` quer dizer "não voltou".
struct Table {
    sent_us: Vec<AtomicU64>,
    rtt_us: Vec<AtomicU64>,
    warm: AtomicU64,
}

impl Table {
    fn new(n: u32) -> Arc<Self> {
        let v = || (0..n).map(|_| AtomicU64::new(0)).collect();
        Arc::new(Self {
            sent_us: v(),
            rtt_us: v(),
            warm: AtomicU64::new(0),
        })
    }

    fn on_reply(&self, id: u32, recv_us: u64) {
        if id == WARMUP_ID {
            self.warm.store(1, Ordering::Relaxed);
        } else if let Some(slot) = self.rtt_us.get(id as usize) {
            let sent = self.sent_us[id as usize].load(Ordering::Relaxed);
            slot.store(recv_us.saturating_sub(sent).max(1), Ordering::Relaxed);
        }
    }

    fn mark_sent(&self, id: u32) {
        if let Some(s) = self.sent_us.get(id as usize) {
            s.store(now_us(), Ordering::Relaxed);
        }
    }

    fn received(&self) -> u32 {
        self.rtt_us
            .iter()
            .filter(|r| r.load(Ordering::Relaxed) != 0)
            .count() as u32
    }

    fn rtts(&self) -> Vec<u64> {
        self.rtt_us
            .iter()
            .map(|r| r.load(Ordering::Relaxed))
            .collect()
    }
}

fn fast_id(ev: InputEvent) -> Option<u32> {
    match ev {
        InputEvent::MouseMove { dx, dy } => {
            Some(u32::from(dx as u16) | (u32::from(dy as u16) << 16))
        }
        _ => None,
    }
}

fn fast_event(id: u32) -> InputEvent {
    InputEvent::MouseMove {
        dx: (id & 0xFFFF) as u16 as i16,
        dy: (id >> 16) as u16 as i16,
    }
}

/// Espera até `target`: dorme o grosso e gira o final (precisão de µs sem ocupar a CPU toda).
fn wait_until(target: Instant) {
    loop {
        let left = target.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        if left > Duration::from_micros(300) {
            std::thread::sleep(left - Duration::from_micros(200));
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Canal rápido: aquece até o eco voltar, depois envia `count` pings no ritmo `rate`.
fn run_fast(
    mut producer: FastSender,
    table: Arc<Table>,
    count: u32,
    rate: u32,
) -> Result<Summary, String> {
    // O agente só aprende o endereço UDP do Mac pelo probe do heartbeat: insiste até voltar.
    let deadline = Instant::now() + Duration::from_secs(15);
    while table.warm.load(Ordering::Relaxed) == 0 {
        if Instant::now() > deadline {
            return Err("canal rápido não respondeu ao aquecimento (UDP bloqueado?)".to_string());
        }
        let _ = producer.push(fast_event(WARMUP_ID));
        std::thread::sleep(Duration::from_millis(50));
    }
    let period = Duration::from_secs_f64(1.0 / f64::from(rate));
    let start = Instant::now();
    for id in 0..count {
        wait_until(start + period * id);
        table.mark_sent(id);
        if producer.push(fast_event(id)).is_err() {
            table.sent_us[id as usize].store(0, Ordering::Relaxed);
        }
    }
    let end = Instant::now() + SETTLE;
    while table.received() < count && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(Summary::from_rtts("fast", count, &table.rtts()))
}

/// Canal confiável: `Input{seq}` pelo TCP cifrado, eco devolvido pelo outro lado.
async fn run_reliable(session: &mut Session, count: u32, rate: u32) -> Boxed<Summary> {
    let table = Table::new(count);
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(rate)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut next = 0u32;
    let settle = tokio::time::sleep(Duration::from_secs(3600));
    tokio::pin!(settle);
    loop {
        if next == count && settle.is_elapsed() {
            break;
        }
        if table.received() >= count {
            break;
        }
        tokio::select! {
            _ = tick.tick(), if next < count => {
                table.mark_sent(next);
                let msg = Message::Input { seq: u64::from(next), ts_us: 0, event: InputEvent::MouseMove { dx: 0, dy: 0 } };
                session.send(msg).await?;
                next += 1;
                if next == count {
                    settle.as_mut().reset(tokio::time::Instant::now() + SETTLE);
                }
            }
            ev = session.recv_event() => match ev {
                Some(SessionEvent::Message(Message::Input { seq, .. })) => table.on_reply(seq as u32, now_us()),
                Some(_) => {}
                None => return Err("sessão encerrada durante a medição".into()),
            },
            () = &mut settle, if next == count => {}
        }
    }
    Ok(Summary::from_rtts("reliable", count, &table.rtts()))
}

async fn wait_connected(session: &Session) -> Boxed<()> {
    let mut st = session.status();
    let r = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if *st.borrow_and_update() == ConnectionStatus::Connected {
                return;
            }
            if st.changed().await.is_err() {
                return;
            }
        }
    })
    .await;
    match r {
        Ok(()) if *st.borrow() == ConnectionStatus::Connected => Ok(()),
        _ => Err("sessão não conectou em 15 s".into()),
    }
}

/// Mede os dois canais numa sessão já configurada.
async fn measure(
    dialer: &mut SessionDialer,
    table: Arc<Table>,
    count: u32,
    rate: u32,
) -> Boxed<(Summary, Summary)> {
    wait_connected(dialer).await?;
    let producer = dialer
        .take_input_producer()
        .ok_or("sem fila do canal rápido")?;
    let fast =
        tokio::task::spawn_blocking(move || run_fast(producer, table, count, rate)).await??;
    let reliable = run_reliable(dialer, count, rate).await?;
    Ok((fast, reliable))
}

fn finish(
    mode: &str,
    rate: u32,
    fast: &Summary,
    reliable: &Summary,
    json: Option<&std::path::Path>,
) -> Boxed<()> {
    println!("RTT em µs (ida e volta, sessão cifrada), {rate} Hz:");
    fast.print();
    reliable.print();
    if let Some(path) = json {
        std::fs::write(path, to_json(mode, rate, fast, reliable))?;
        eprintln!("JSON gravado em {}", path.display());
    }
    Ok(())
}

/// Dialer com identidade efêmera para o peer já conhecido.
async fn dial(
    peer: relay_core::types::PairedPeer,
    identity: relay_core::crypto::StaticKeypair,
    remote_ip: IpAddr,
    remote_ports: Ports,
    bind_ip: IpAddr,
    table: Arc<Table>,
) -> Boxed<SessionDialer> {
    let local = Ports {
        control: 0,
        input: 0,
        ..Ports::default()
    };
    let mut cfg = DialerConfig::new(identity, peer, remote_ip, remote_ports, local);
    cfg.bind_ip = bind_ip;
    let on_fast = Box::new(move |ev: FastEvent| {
        if let Some(id) = fast_id(ev.event) {
            table.on_reply(id, ev.recv_us);
        }
    });
    Ok(SessionDialer::start(cfg, on_fast).await?)
}

pub async fn run_send(o: SendOpts) -> Boxed<()> {
    let target = if o.peer.contains(':') && o.peer.parse::<SocketAddr>().is_ok() {
        o.peer.parse::<SocketAddr>()?
    } else {
        (o.peer.as_str(), o.port_control)
            .to_socket_addrs()?
            .next()
            .ok_or("peer não resolveu")?
    };
    let identity = load_or_create_identity(&MemoryKeyStore::default())?;
    let me = LocalDevice {
        name: "relay-bench".into(),
        public_key: identity.public,
        mac_address: [0; 6],
    };
    eprintln!("pareando com {target}...");
    let peer = SessionDialer::pair(target, &o.code, &me).await?;
    let remote_ports = Ports {
        control: target.port(),
        input: o.port_input,
        ..Ports::default()
    };
    let table = Table::new(o.count);
    let mut dialer = dial(
        peer,
        identity,
        target.ip(),
        remote_ports,
        IpAddr::from([0, 0, 0, 0]),
        table.clone(),
    )
    .await?;
    let (fast, reliable) = measure(&mut dialer, table, o.count, o.rate).await?;
    dialer.shutdown().await;
    finish("send", o.rate, &fast, &reliable, o.json.as_deref())
}

pub async fn run_loopback(o: LoopbackOpts) -> Boxed<()> {
    let lo = IpAddr::from([127, 0, 0, 1]);
    let identity = load_or_create_identity(&MemoryKeyStore::default())?;
    let me = LocalDevice {
        name: "relay-bench".into(),
        public_key: identity.public,
        mac_address: [0; 6],
    };
    let ephemeral = Ports {
        control: 0,
        input: 0,
        ..Ports::default()
    };
    let listener = start_listener(lo, ephemeral, Vec::new()).await?;
    let control = listener.local_control_addr();
    let remote_ports = Ports {
        control: control.port(),
        input: listener.local_input_addr().port(),
        ..Ports::default()
    };
    // `Session` não é `Sync` (a fila do canal rápido), então o eco roda no mesmo task, sem spawn.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let echo = echo_loop(listener, Some("123456"), async {
        let _ = stop_rx.await;
    });
    let drive = async {
        let peer = SessionDialer::pair(control, "123456", &me).await?;
        let table = Table::new(o.count);
        let mut dialer = dial(peer, identity, lo, remote_ports, lo, table.clone()).await?;
        let result = measure(&mut dialer, table, o.count, o.rate).await;
        dialer.shutdown().await;
        let _ = stop_tx.send(());
        result
    };
    let (listener, result) = tokio::join!(echo, drive);
    listener.shutdown().await;
    let (fast, reliable) = result?;
    finish("loopback", o.rate, &fast, &reliable, o.json.as_deref())
}
