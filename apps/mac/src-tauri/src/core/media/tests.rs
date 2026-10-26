//! dono: MI3
//! Mídia em loopback contra um "agente" falso: mesmas chaves, mesmo formato de datagrama e o
//! `TransferServer`/`TransferClient`/`ClipboardSync` reais do core, com dispositivos mock.

use std::f32::consts::TAU;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use relay_core::audio::{FRAMES_PER_PACKET, PACKET_PAYLOAD_BYTES};
use relay_core::clipboard::ClipboardSync;
use relay_core::crypto::{
    MemoryKeyStore, ReplayMode, StaticKeypair, UdpAead, UdpChannel, derive_udp_keys, device_id_of,
    load_or_create_identity,
};
use relay_core::engine::testutil::MockClip;
use relay_core::platform::{
    AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, ClipboardBackend, PlatformError,
};
use relay_core::transfer::{TransferClient, TransferServer};
use relay_core::types::{
    AudioDevices, ClipboardConfig, ClipboardContent, ClipboardMode, CommandResult, Config,
    DisplaysPayload, FileEntry, LatencyStats, Message, PairedPeer, PairingEvent, PermissionsStatus,
    Ports, StatusSnapshot, TransferProgress,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::*;
use crate::core::AudioKeys;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const AMP: f32 = 0.5;
const HH: [u8; 32] = [9; 32];

// ---- mocks ----

#[derive(Default)]
struct Rec {
    progress: Mutex<Vec<TransferProgress>>,
}

impl CoreEvents for Rec {
    fn status(&self, _: &StatusSnapshot) {}
    fn latency(&self, _: &LatencyStats) {}
    fn displays(&self, _: &DisplaysPayload) {}
    fn permissions(&self, _: &PermissionsStatus) {}
    fn secure_input(&self, _: bool) {}
    fn pairing(&self, _: &PairingEvent) {}
    fn command_result(&self, _: &CommandResult) {}
    fn transfer_progress(&self, p: &TransferProgress) {
        self.progress.lock().unwrap().push(p.clone());
    }
}

struct MockCapture {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AudioCapture for MockCapture {
    fn start(&mut self, mut sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        let stop = self.stop.clone();
        stop.store(false, Ordering::Relaxed);
        self.thread = Some(thread::spawn(move || {
            let mut n = 0usize;
            let t0 = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                if t0.elapsed() >= Duration::from_millis(5 * n as u64) {
                    let mut f: AudioFrame = [0; AUDIO_FRAME_SAMPLES * 2];
                    for i in 0..AUDIO_FRAME_SAMPLES {
                        let s = sine(n * AUDIO_FRAME_SAMPLES + i);
                        f[2 * i] = s;
                        f[2 * i + 1] = s;
                    }
                    let _ = sink.push(f);
                    n += 1;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }));
        Ok(())
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
    fn set_device(&mut self, _: Option<&str>) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// 1 kHz a 48 kHz: 48 amostras por ciclo, fase contínua entre pacotes.
fn sine(n: usize) -> i16 {
    ((TAU * n as f32 / 48.0).sin() * AMP * 32767.0) as i16
}

struct MockPlatform {
    clip: MockClip,
    drag: Mutex<Vec<PathBuf>>,
    finder: AtomicBool,
    revealed: Mutex<Vec<PathBuf>>,
    downloads: PathBuf,
}

impl MediaPlatform for MockPlatform {
    fn capture(&self) -> Box<dyn AudioCapture> {
        Box::new(MockCapture {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }
    fn clipboard(&self) -> Box<dyn ClipboardBackend> {
        Box::new(self.clip.clone())
    }
    fn audio_devices(&self) -> AudioDevices {
        AudioDevices::default()
    }
    fn drag_files(&self) -> Vec<PathBuf> {
        self.drag.lock().unwrap().clone()
    }
    fn finder_frontmost(&self) -> bool {
        self.finder.load(Ordering::Relaxed)
    }
    fn reveal(&self, paths: &[PathBuf]) {
        self.revealed.lock().unwrap().extend_from_slice(paths);
    }
    fn downloads_dir(&self) -> PathBuf {
        self.downloads.clone()
    }
}

// ---- cenário ----

fn ident() -> StaticKeypair {
    load_or_create_identity(&MemoryKeyStore::default()).unwrap()
}

fn peer_of(k: &StaticKeypair, name: &str) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&k.public),
        name: name.into(),
        public_key: k.public,
        mac_address: [0; 6],
    }
}

fn mock_platform(dir: &Path) -> Arc<MockPlatform> {
    Arc::new(MockPlatform {
        clip: MockClip::default(),
        drag: Mutex::default(),
        finder: AtomicBool::new(false),
        revealed: Mutex::default(),
        downloads: dir.join("Downloads/Relay"),
    })
}

struct World {
    media: Arc<RealMedia>,
    plat: Arc<MockPlatform>,
    rec: Arc<Rec>,
    /// Mensagens que o Mac mandou pelo canal confiável.
    wire: UnboundedReceiver<Message>,
    /// Socket UDP do agente: recebe o microfone do Mac.
    agent_udp: UdpSocket,
    agent_id: StaticKeypair,
    mac_peer: PairedPeer,
    _dir: tempfile::TempDir,
    dir: PathBuf,
    rt: tokio::runtime::Runtime,
}

impl World {
    /// `agent_transfer_port`: porta em que o servidor de transferência do agente escuta.
    fn new(agent_transfer_port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let plat = mock_platform(dir.path());
        let rec = Arc::new(Rec::default());
        let media = RealMedia::new(plat.clone(), rec.clone(), rt.handle().clone());
        let (mac_id, agent_id) = (ident(), ident());
        let (mac_peer, agent_peer) = (peer_of(&mac_id, "Mac"), peer_of(&agent_id, "PC"));
        let agent_udp = UdpSocket::bind((LO, 0)).unwrap();
        agent_udp
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let (tx, wire) = unbounded_channel();
        let keys = AudioKeys {
            mac_to_pc: derive_udp_keys(&HH, UdpChannel::AudioMacToPc, true),
        };
        let ctx = SessionCtx {
            send: tx,
            peer_ip: LO,
            remote_ports: Ports {
                audio_mac_to_pc: agent_udp.local_addr().unwrap().port(),
                transfer: agent_transfer_port,
                ..Ports::default()
            },
            local_ports: Ports {
                transfer: 0,
                ..Ports::default()
            },
            bind_ip: LO,
            identity: mac_id,
            peer: agent_peer,
            audio_keys: Some(keys),
        };
        media.config_changed(&Config::default());
        media.session_up(ctx);
        media.link_changed(true);
        Self {
            media,
            plat,
            rec,
            wire,
            agent_udp,
            agent_id,
            mac_peer,
            dir: dir.path().to_path_buf(),
            _dir: dir,
            rt,
        }
    }

    fn drain(&mut self) -> Vec<Message> {
        let mut v = Vec::new();
        while let Ok(m) = self.wire.try_recv() {
            v.push(m);
        }
        v
    }

    /// Espera mensagens até `pred` achar algo.
    fn wait_msgs(&mut self, mut pred: impl FnMut(&[Message]) -> bool) -> Vec<Message> {
        let mut all = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            all.extend(self.drain());
            if pred(&all) {
                return all;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("mensagem esperada não chegou: {all:?}");
    }
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(10) {
        if cond() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("tempo esgotado: {what}");
}

fn deliver(media: &RealMedia, msgs: impl IntoIterator<Item = Message>) {
    for m in msgs {
        media.on_message(&m);
    }
}

fn write_file(path: &Path, len: usize, seed: u8) {
    let data: Vec<u8> = (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed)
        .collect();
    std::fs::write(path, data).unwrap();
}

// ---- áudio ----

#[test]
fn mic_frames_are_sent_sealed_to_the_agent() {
    let mut w = World::new(0);
    w.media.set_audio_streams(true);
    w.wait_msgs(|m| m.iter().any(|x| matches!(x, Message::AudioControl { .. })));
    let mut agent = UdpAead::new(
        derive_udp_keys(&HH, UdpChannel::AudioMacToPc, false),
        ReplayMode::Window64,
    );
    let mut buf = [0u8; 2048];
    let mut plain = Vec::new();
    let mut seqs = Vec::new();
    let mut first_samples = Vec::new();
    for _ in 0..4 {
        let (n, _) = w
            .agent_udp
            .recv_from(&mut buf)
            .expect("datagrama do microfone");
        assert_eq!(n, 17 + 4 + PACKET_PAYLOAD_BYTES + 16);
        let (hdr, cipher) = buf[..n].split_at(17);
        assert_eq!(hdr[0], 1, "stream Mac→PC");
        let seq = u64::from_le_bytes(hdr[1..9].try_into().unwrap());
        agent.open(seq, hdr, cipher, &mut plain).unwrap();
        assert_eq!(&plain[..4], &[0, 2, FRAMES_PER_PACKET as u8, 0]);
        seqs.push(seq);
        let pcm = &plain[4..];
        first_samples.push(i16::from_le_bytes([pcm[0], pcm[1]]));
    }
    assert!(seqs.windows(2).all(|p| p[1] == p[0] + 1), "{seqs:?}");
    // Fase contínua da senoide: o primeiro frame do pacote k é a amostra k*240 (múltiplo de 48).
    assert!(
        first_samples.iter().all(|&s| s.abs() < 700),
        "{first_samples:?}"
    );
    w.media.set_audio_streams(false);
    assert!(!w.media.sh.st().audio.tx_running());
}

#[test]
fn audio_without_keys_stays_off() {
    let w = World::new(0);
    let mut ctx = (*w.media.sh.session().unwrap()).clone();
    ctx.audio_keys = None;
    w.media.session_up(ctx);
    w.media.set_audio_streams(true);
    assert!(!w.media.sh.st().audio.tx_running());
}

// ---- clipboard ----

fn agent_sync() -> ClipboardSync {
    ClipboardSync::new(ClipboardConfig {
        mode: ClipboardMode::OnCopy,
        ..ClipboardConfig::default()
    })
}

#[test]
fn text_is_synced_on_switch_and_applied_with_marker() {
    let mut w = World::new(0);
    w.plat.clip.0.lock().unwrap().content = Some(ClipboardContent::Text("olá do Mac".into()));
    w.media.clipboard_switch();
    let msgs = w.wait_msgs(|m| {
        m.iter()
            .any(|x| matches!(x, Message::ClipboardChunk { .. }))
    });
    let mut agent = agent_sync();
    let got = msgs
        .into_iter()
        .fold(None, |acc, m| agent.on_message(m).or(acc));
    assert_eq!(got, Some(ClipboardContent::Text("olá do Mac".into())));

    // Sentido inverso: o PC copia, o Mac aplica no pasteboard (pelo caminho marcado do backend).
    let mut pc = agent_sync();
    let offer = pc.on_local_change(ClipboardContent::Text("olá do PC".into()));
    deliver(&w.media, offer);
    wait_until("texto do PC aplicado", || {
        w.plat.clip.0.lock().unwrap().written == [ClipboardContent::Text("olá do PC".into())]
    });
    // O que o PC mandou não volta como eco.
    w.media.clipboard_switch();
    thread::sleep(Duration::from_millis(200));
    assert!(
        w.drain()
            .iter()
            .all(|m| !matches!(m, Message::ClipboardOffer { .. }))
    );
}

#[test]
fn screenshot_png_from_the_pc_lands_on_the_pasteboard() {
    let w = World::new(0);
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let mut pc = agent_sync();
    deliver(
        &w.media,
        pc.on_local_change(ClipboardContent::Png(png.clone())),
    );
    wait_until("png aplicado", || {
        w.plat.clip.0.lock().unwrap().written == [ClipboardContent::Png(png.clone())]
    });
}

#[test]
fn send_now_offers_even_after_the_same_content() {
    let mut w = World::new(0);
    w.plat.clip.0.lock().unwrap().content = Some(ClipboardContent::Text("x".into()));
    w.media.clipboard_send_now();
    w.wait_msgs(|m| {
        m.iter()
            .any(|x| matches!(x, Message::ClipboardOffer { .. }))
    });
    w.media.clipboard_send_now();
    w.wait_msgs(|m| {
        m.iter()
            .any(|x| matches!(x, Message::ClipboardOffer { .. }))
    });
}

// ---- arquivos ----

fn hash(p: &Path) -> Vec<u8> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::fs::read(p).unwrap().hash(&mut h);
    h.finish().to_le_bytes().to_vec()
}

#[test]
fn mac_files_are_served_to_the_pc_on_demand() {
    let mut w = World::new(0);
    let src = w.dir.join("origem");
    std::fs::create_dir_all(&src).unwrap();
    let big = src.join("grande.bin");
    write_file(&big, 5 * 1024 * 1024, 7);
    {
        let mut c = w.plat.clip.0.lock().unwrap();
        c.paths = vec![big.clone()];
        c.content = Some(ClipboardContent::FileList(vec![FileEntry {
            id: 0,
            name: "grande.bin".into(),
            size: 5 << 20,
            is_dir: false,
        }]));
    }
    w.media.clipboard_switch();
    let msgs = w.wait_msgs(|m| {
        m.iter()
            .any(|x| matches!(x, Message::ClipboardChunk { .. }))
    });
    let mut agent = agent_sync();
    let Some(ClipboardContent::FileList(entries)) = msgs
        .into_iter()
        .fold(None, |acc, m| agent.on_message(m).or(acc))
    else {
        panic!("sem FileList")
    };
    assert_eq!(entries[0].id, 0);

    // O "Explorer" cola: o PC conecta no Mac e baixa pelo id.
    let addr = SocketAddr::new(LO, w.media.transfer_addr().unwrap().port());
    let out = w.dir.join("destino");
    std::fs::create_dir_all(&out).unwrap();
    let client = TransferClient::new(w.agent_id.clone(), w.mac_peer.clone());
    let cancel = AtomicBool::new(false);
    let paths = w.rt.block_on(async {
        let mut s = TransferClient::connect(addr).await.unwrap();
        client
            .download(&mut s, &[0], &out, 1, &cancel, &mut |_| {})
            .await
            .unwrap()
    });
    assert_eq!(hash(&paths[0]), hash(&big));
}

/// Servidor de transferência do agente, em porta efêmera, servindo `paths`.
fn start_agent_server(w: &World, paths: Vec<PathBuf>) -> u16 {
    let server = Arc::new(TransferServer::new(
        w.agent_id.clone(),
        vec![w.mac_peer.clone()],
        paths,
    ));
    w.rt.block_on(async {
        let l = TransferServer::bind(Some(SocketAddr::new(LO, 0)))
            .await
            .unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(server.serve(l));
        port
    })
}

#[test]
fn pc_files_download_to_downloads_relay_on_paste_in_finder() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("do-pc.bin");
    write_file(&file, 5 * 1024 * 1024, 3);
    // A porta do servidor precisa existir antes do `SessionCtx`: sobe o mundo depois.
    let boot = World::new(0);
    let port = start_agent_server(&boot, vec![file.clone()]);
    let mut ctx = (*boot.media.sh.session().unwrap()).clone();
    ctx.remote_ports.transfer = port;
    boot.media.session_up(ctx);

    let mut pc = agent_sync();
    let entry = FileEntry {
        id: 0,
        name: "do-pc.bin".into(),
        size: 5 << 20,
        is_dir: false,
    };
    deliver(
        &boot.media,
        pc.on_local_change(ClipboardContent::FileList(vec![entry])),
    );
    wait_until("oferta pendente", || boot.media.has_pending_files());

    // Finder fora de primeiro plano: Cmd+V não baixa.
    boot.media.paste_requested();
    thread::sleep(Duration::from_millis(200));
    assert!(boot.plat.revealed.lock().unwrap().is_empty());

    boot.plat.finder.store(true, Ordering::Relaxed);
    boot.media.paste_requested();
    wait_until("arquivo revelado", || {
        !boot.plat.revealed.lock().unwrap().is_empty()
    });
    let got = boot.plat.revealed.lock().unwrap()[0].clone();
    assert!(got.starts_with(boot.plat.downloads.clone()), "{got:?}");
    assert_eq!(hash(&got), hash(&file));
    assert!(!boot.media.has_pending_files());
    let last = boot.rec.progress.lock().unwrap().last().cloned().unwrap();
    assert!(last.done && last.error.is_none(), "{last:?}");
}

#[test]
fn receive_remote_files_button_works_without_finder() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("botao.txt");
    write_file(&file, 1000, 1);
    let boot = World::new(0);
    let port = start_agent_server(&boot, vec![file.clone()]);
    let mut ctx = (*boot.media.sh.session().unwrap()).clone();
    ctx.remote_ports.transfer = port;
    boot.media.session_up(ctx);
    deliver(
        &boot.media,
        [Message::FileOffer {
            offer_id: 1,
            files: vec![FileEntry {
                id: 0,
                name: "botao.txt".into(),
                size: 1000,
                is_dir: false,
            }],
        }],
    );
    boot.media.receive_remote_files();
    wait_until("arquivo revelado", || {
        !boot.plat.revealed.lock().unwrap().is_empty()
    });
    assert_eq!(hash(&boot.plat.revealed.lock().unwrap()[0]), hash(&file));
}

// ---- arraste ----

#[test]
fn drag_into_the_pc_offers_files_and_drops_on_mouse_up() {
    let mut w = World::new(0);
    let f = w.dir.join("arrastado.bin");
    write_file(&f, 4096, 9);
    *w.plat.drag.lock().unwrap() = vec![f.clone()];

    // Sem botão pressionado ao entrar: nada.
    w.media.focus_changed(true, false);
    w.media.left_button_up();
    assert!(
        w.drain()
            .iter()
            .all(|m| !matches!(m, Message::FileOffer { .. } | Message::DropFiles { .. }))
    );

    w.media.focus_changed(false, false);
    w.media.focus_changed(true, true);
    let msgs = w.wait_msgs(|m| m.iter().any(|x| matches!(x, Message::FileOffer { .. })));
    let Some(Message::FileOffer { offer_id, files }) = msgs
        .into_iter()
        .find(|m| matches!(m, Message::FileOffer { .. }))
    else {
        unreachable!()
    };
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].id, files[0].size, files[0].is_dir),
        (0, 4096, false)
    );
    w.media.left_button_up();
    let msgs = w.wait_msgs(|m| m.iter().any(|x| matches!(x, Message::DropFiles { .. })));
    assert!(
        msgs.iter()
            .any(|m| matches!(m, Message::DropFiles { offer_id: o, .. } if *o == offer_id))
    );
    // Um segundo mouse up não repete o drop.
    w.media.left_button_up();
    thread::sleep(Duration::from_millis(50));
    assert!(
        w.drain()
            .iter()
            .all(|m| !matches!(m, Message::DropFiles { .. }))
    );

    // O PC baixa o arraste pelo mesmo servidor.
    let addr = SocketAddr::new(LO, w.media.transfer_addr().unwrap().port());
    let out = w.dir.join("drop");
    std::fs::create_dir_all(&out).unwrap();
    let client = TransferClient::new(w.agent_id.clone(), w.mac_peer.clone());
    let cancel = AtomicBool::new(false);
    let paths = w.rt.block_on(async {
        let mut s = TransferClient::connect(addr).await.unwrap();
        client
            .download(&mut s, &[0], &out, 2, &cancel, &mut |_| {})
            .await
            .unwrap()
    });
    assert_eq!(hash(&paths[0]), hash(&f));
}

// ---- AppCore ↔ agente falso (sessão real: Noise, chaves de áudio do handshake) ----

mod appcore {
    use relay_core::crypto::LocalDevice;
    use relay_core::net::{AudioLink, ListenerConfig, SessionEvent, SessionListener};
    use relay_core::types::ConnectionStatus;

    use super::*;
    use crate::backend::AppBackend;
    use crate::core::tests::MockPlatform as MacPlatform;
    use crate::core::{AppCore, CoreOptions};

    struct MiniAgent {
        control_port: u16,
        input_port: u16,
        outbox: Arc<Mutex<Vec<Message>>>,
        received: Arc<Mutex<Vec<Message>>>,
        link: Arc<Mutex<Option<AudioLink>>>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl MiniAgent {
        fn start(code: &str) -> Self {
            let outbox: Arc<Mutex<Vec<Message>>> = Arc::default();
            let received: Arc<Mutex<Vec<Message>>> = Arc::default();
            let link: Arc<Mutex<Option<AudioLink>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (ports_tx, ports_rx) = std::sync::mpsc::channel();
            let (o, r, l, s, code) = (
                outbox.clone(),
                received.clone(),
                link.clone(),
                stop.clone(),
                code.to_owned(),
            );
            let thread = thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let identity = ident();
                    let device = LocalDevice {
                        name: "PC".into(),
                        public_key: identity.public,
                        mac_address: [1; 6],
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
                    let mut l_ = SessionListener::start(cfg, Box::new(|_| {})).await.unwrap();
                    l_.open_pairing(code, Duration::from_secs(120));
                    ports_tx
                        .send((l_.local_control_addr().port(), l_.local_input_addr().port()))
                        .unwrap();
                    while !s.load(Ordering::SeqCst) {
                        tokio::select! {
                            ev = l_.recv_event() => match ev {
                                None => break,
                                Some(SessionEvent::Paired(p)) => l_.add_peer(p),
                                Some(SessionEvent::Connected(_)) => {
                                    *l.lock().unwrap() = l_.audio_link();
                                }
                                Some(SessionEvent::Message(m)) => r.lock().unwrap().push(m),
                                Some(_) => {}
                            },
                            () = tokio::time::sleep(Duration::from_millis(5)) => {}
                        }
                        let out: Vec<Message> = o.lock().unwrap().drain(..).collect();
                        for m in out {
                            let _ = l_.send(m).await;
                        }
                    }
                    l_.shutdown().await;
                });
            });
            let (control_port, input_port) = ports_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Self {
                control_port,
                input_port,
                outbox,
                received,
                link,
                stop,
                thread: Some(thread),
            }
        }

        fn saw(&self, f: impl Fn(&Message) -> bool) -> bool {
            self.received.lock().unwrap().iter().any(f)
        }
    }

    impl Drop for MiniAgent {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    async fn until(what: &str, mut cond: impl FnMut() -> bool) {
        let t = Instant::now();
        while !cond() {
            assert!(
                t.elapsed() < Duration::from_secs(8),
                "não aconteceu: {what}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn clipboard_and_mic_control_over_a_real_session() {
        let agent = MiniAgent::start("123456");
        let mut cfg = Config::default();
        cfg.network.ports = Ports {
            control: agent.control_port,
            input: agent.input_port,
            ..Ports::default()
        };
        let mac = MacPlatform::new(&cfg);
        let dir = tempfile::tempdir().unwrap();
        let plat = mock_platform(dir.path());
        let rec = Arc::new(Rec::default());
        let media = RealMedia::new(plat.clone(), rec.clone(), Handle::current());
        let core = AppCore::start(
            mac,
            rec.clone(),
            media.clone(),
            CoreOptions {
                bind_ip: LO,
                local_ports: Some(Ports {
                    input: 0,
                    transfer: 0,
                    ..Ports::default()
                }),
                manual_grace: Duration::ZERO,
            },
            Handle::current(),
        );
        core.pair_with_code(
            "123456".into(),
            Some(format!("127.0.0.1:{}", agent.control_port)),
        )
        .await
        .unwrap();
        until("sessão conectada", || {
            core.status().status == ConnectionStatus::Connected
                && agent.link.lock().unwrap().is_some()
        })
        .await;

        // Texto do PC chega ao pasteboard do Mac.
        let mut pc = agent_sync();
        agent
            .outbox
            .lock()
            .unwrap()
            .extend(pc.on_local_change(ClipboardContent::Text("do PC".into())));
        until("texto aplicado", || {
            plat.clip.0.lock().unwrap().written == [ClipboardContent::Text("do PC".into())]
        })
        .await;

        // "Enviar agora" leva o texto do Mac ao agente.
        plat.clip.0.lock().unwrap().content = Some(ClipboardContent::Text("do Mac".into()));
        core.clipboard_send_now().unwrap();
        until("oferta no agente", || {
            agent.saw(|m| matches!(m, Message::ClipboardOffer { .. }))
        })
        .await;

        // Microfone: o pedido vira AudioControl no agente.
        core.set_audio_streams(true).unwrap();
        assert!(core.status().audio.mic_to_peer);
        until("AudioControl no agente", || {
            agent.saw(|m| {
                matches!(
                    m,
                    Message::AudioControl {
                        pc_to_mac: false,
                        mac_to_pc: true
                    }
                )
            })
        })
        .await;
        core.shutdown();
    }
}

// ---- revisão final: nonce, listas, handshake, PNG, Info.plist ----

#[test]
fn info_plist_declares_microphone_usage() {
    let plist = include_str!("../../../Info.plist");
    assert!(plist.contains("<key>NSMicrophoneUsageDescription</key>"));
}

/// Lê `n` datagramas do microfone e devolve os seq.
fn mic_seqs(w: &World, n: usize) -> Vec<u64> {
    let mut buf = [0u8; 2048];
    (0..n)
        .map(|_| {
            let (len, _) = w.agent_udp.recv_from(&mut buf).expect("datagrama");
            assert!(len > 17);
            u64::from_le_bytes(buf[1..9].try_into().unwrap())
        })
        .collect()
}

#[test]
fn mic_seq_keeps_counting_when_the_stream_is_toggled() {
    let w = World::new(0);
    w.media.set_audio_streams(true);
    let first = mic_seqs(&w, 3);
    w.media.set_audio_streams(false);
    // Pacotes que ficaram na fila do socket antes de parar.
    w.agent_udp.set_nonblocking(true).unwrap();
    let mut buf = [0u8; 2048];
    let mut last = *first.last().unwrap();
    while let Ok((_, _)) = w.agent_udp.recv_from(&mut buf) {
        last = last.max(u64::from_le_bytes(buf[1..9].try_into().unwrap()));
    }
    w.agent_udp.set_nonblocking(false).unwrap();
    w.media.set_audio_streams(true);
    let second = mic_seqs(&w, 3);
    assert!(second[0] > last, "seq voltou: {last} -> {second:?}");

    // Mesma sessão (mesmas chaves) reapresentada: o contador não zera.
    let ctx = (*w.media.sh.session().unwrap()).clone();
    w.media.session_up(ctx);
    let third = mic_seqs(&w, 3);
    assert!(third[0] > second[2], "{second:?} -> {third:?}");
}

fn file_list(id: u32, path: &Path) -> ClipboardContent {
    ClipboardContent::FileList(vec![FileEntry {
        id,
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        size: std::fs::metadata(path).unwrap().len(),
        is_dir: false,
    }])
}

fn offered_list(w: &mut World, path: &Path) -> Vec<FileEntry> {
    {
        let mut c = w.plat.clip.0.lock().unwrap();
        c.paths = vec![path.to_path_buf()];
        c.content = Some(file_list(0, path));
    }
    w.media.clipboard_switch();
    let msgs = w.wait_msgs(|m| {
        m.iter()
            .any(|x| matches!(x, Message::ClipboardChunk { .. }))
    });
    let mut agent = agent_sync();
    let Some(ClipboardContent::FileList(entries)) = msgs
        .into_iter()
        .fold(None, |acc, m| agent.on_message(m).or(acc))
    else {
        panic!("sem FileList")
    };
    entries
}

fn download_id(w: &World, id: u32, out: &Path) -> Result<Vec<PathBuf>, String> {
    let addr = SocketAddr::new(LO, w.media.transfer_addr().unwrap().port());
    std::fs::create_dir_all(out).unwrap();
    let client = TransferClient::new(w.agent_id.clone(), w.mac_peer.clone());
    let cancel = AtomicBool::new(false);
    w.rt.block_on(async {
        let mut s = TransferClient::connect(addr).await.unwrap();
        client
            .download(
                &mut s,
                &[id],
                out,
                u32::from(id as u16) + 10,
                &cancel,
                &mut |_| {},
            )
            .await
            .map_err(|e| e.to_string())
    })
}

#[test]
fn earlier_offers_stay_servable_until_they_expire() {
    let mut w = World::new(0);
    let (x, y, z) = (
        w.dir.join("x.bin"),
        w.dir.join("y.bin"),
        w.dir.join("z.bin"),
    );
    write_file(&x, 3000, 1);
    write_file(&y, 4000, 2);
    write_file(&z, 5000, 3);
    let ex = offered_list(&mut w, &x);
    // Arraste de outro arquivo depois da cópia: não pode trocar o que o PC ainda vai colar.
    *w.plat.drag.lock().unwrap() = vec![y.clone()];
    w.media.focus_changed(true, true);
    let msgs = w.wait_msgs(|m| m.iter().any(|m| matches!(m, Message::FileOffer { .. })));
    let Some(Message::FileOffer { files: ey, .. }) = msgs
        .into_iter()
        .find(|m| matches!(m, Message::FileOffer { .. }))
    else {
        unreachable!()
    };
    assert_ne!(ex[0].id, ey[0].id, "ids precisam ser estáveis e distintos");
    let got_x = download_id(&w, ex[0].id, &w.dir.join("o1")).unwrap();
    assert_eq!(hash(&got_x[0]), hash(&x));
    let got_y = download_id(&w, ey[0].id, &w.dir.join("o2")).unwrap();
    assert_eq!(hash(&got_y[0]), hash(&y));

    // Expiração: com TTL curto, uma lista nova esvazia as velhas e o id antigo falha.
    w.media
        .sh
        .files
        .set_timeouts(Duration::from_millis(50), Duration::from_secs(10));
    thread::sleep(Duration::from_millis(120));
    let ez = offered_list(&mut w, &z);
    assert!(download_id(&w, ex[0].id, &w.dir.join("o3")).is_err());
    let got_z = download_id(&w, ez[0].id, &w.dir.join("o4")).unwrap();
    assert_eq!(hash(&got_z[0]), hash(&z));
}

#[test]
fn silent_transfer_connection_is_dropped() {
    use std::io::Read;
    let mut w = World::new(0);
    let f = w.dir.join("a.bin");
    write_file(&f, 10, 1);
    offered_list(&mut w, &f);
    w.media
        .sh
        .files
        .set_timeouts(Duration::from_secs(600), Duration::from_millis(200));
    // O timeout vale para conexões aceitas depois do ajuste: reinicia o listener.
    w.media.sh.files.shutdown();
    let ctx = w.media.sh.session().unwrap();
    w.media.sh.files.restart_for_test(&w.media.sh, &ctx);
    let addr = SocketAddr::new(LO, w.media.transfer_addr().unwrap().port());
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut b = [0u8; 1];
    let r = s.read(&mut b);
    assert!(matches!(r, Ok(0)), "conexão muda devia fechar: {r:?}");
}

#[test]
fn capture_screen_png_applies_even_with_clipboard_sync_off() {
    let w = World::new(0);
    let mut cfg = Config::default();
    cfg.clipboard.enabled = false;
    w.media.config_changed(&cfg);
    let png = vec![0x89, b'P', b'N', b'G', 9];
    let mut pc = agent_sync();
    // Sem pedido de captura, o toggle desligado vale.
    deliver(
        &w.media,
        pc.on_local_change(ClipboardContent::Png(png.clone())),
    );
    thread::sleep(Duration::from_millis(300));
    assert!(w.plat.clip.0.lock().unwrap().written.is_empty());
    // Com CaptureScreen em andamento, o PNG entra; texto comum continua barrado.
    w.media.capture_screen_requested();
    let mut pc = agent_sync();
    deliver(
        &w.media,
        pc.on_local_change(ClipboardContent::Png(png.clone())),
    );
    wait_until("png aplicado", || {
        w.plat.clip.0.lock().unwrap().written == [ClipboardContent::Png(png.clone())]
    });
    deliver(
        &w.media,
        pc.on_local_change(ClipboardContent::Text("não".into())),
    );
    thread::sleep(Duration::from_millis(300));
    assert_eq!(w.plat.clip.0.lock().unwrap().written.len(), 1);
}
