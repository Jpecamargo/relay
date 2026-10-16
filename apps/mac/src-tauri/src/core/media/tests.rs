//! dono: MI3
//! Mídia em loopback contra um "agente" falso: mesmas chaves, mesmo formato de datagrama e o
//! `TransferServer`/`TransferClient`/`ClipboardSync` reais do core, com dispositivos mock.

use std::f32::consts::TAU;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use relay_core::audio::{FRAMES_PER_PACKET, PACKET_PAYLOAD_BYTES};
use relay_core::clipboard::ClipboardSync;
use relay_core::crypto::{
    MemoryKeyStore, ReplayMode, StaticKeypair, UdpAead, UdpChannel, derive_udp_keys, device_id_of,
    load_or_create_identity,
};
use relay_core::platform::{
    AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, AudioPlayback, ChangeCallback, ClipboardBackend,
    PlatformError,
};
use relay_core::transfer::{TransferClient, TransferServer};
use relay_core::types::{
    AudioDevice, AudioDevices, AudioStats, ClipboardConfig, ClipboardContent, ClipboardMode,
    CommandResult, Config, DisplaysPayload, FileEntry, LatencyStats, Message, PairedPeer,
    PairingEvent, PermissionsStatus, Ports, StatusSnapshot, TransferProgress,
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
    audio: Mutex<Vec<AudioStats>>,
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
    fn audio_stats(&self, s: &AudioStats) {
        self.audio.lock().unwrap().push(*s);
    }
    fn transfer_progress(&self, p: &TransferProgress) {
        self.progress.lock().unwrap().push(p.clone());
    }
}

struct MockPlayback {
    out: Arc<Mutex<Vec<f32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AudioPlayback for MockPlayback {
    fn start(&mut self, mut source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        let (out, stop) = (self.out.clone(), self.stop.clone());
        stop.store(false, Ordering::Relaxed);
        self.thread = Some(thread::spawn(move || {
            // Relógio do "dispositivo": 48 frames por milissegundo, silêncio se faltar.
            let t0 = Instant::now();
            let mut played = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let due = t0.elapsed().as_micros() as usize * 48 / 1000;
                let mut buf = out.lock().unwrap();
                while played < due {
                    let l = source.pop().unwrap_or(0.0);
                    let r = source.pop().unwrap_or(0.0);
                    buf.push(l);
                    buf.push(r);
                    played += 1;
                }
                drop(buf);
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
    fn set_volume(&mut self, _: f32) {}
    fn set_device(&mut self, _: Option<&str>) -> Result<(), PlatformError> {
        Ok(())
    }
    fn list_devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        Ok(Vec::new())
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

#[derive(Default)]
struct ClipState {
    content: Option<ClipboardContent>,
    paths: Vec<PathBuf>,
    written: Vec<ClipboardContent>,
}

#[derive(Clone, Default)]
struct MockClip(Arc<Mutex<ClipState>>);

impl ClipboardBackend for MockClip {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        Ok(self.0.lock().unwrap().content.clone())
    }
    fn write_remote(&mut self, content: ClipboardContent) -> Result<(), PlatformError> {
        let mut s = self.0.lock().unwrap();
        s.written.push(content.clone());
        s.content = Some(content);
        Ok(())
    }
    fn set_change_callback(&mut self, _: ChangeCallback) {}
    fn local_paths(&self) -> Vec<PathBuf> {
        self.0.lock().unwrap().paths.clone()
    }
}

struct MockPlatform {
    clip: MockClip,
    playback_out: Arc<Mutex<Vec<f32>>>,
    drag: Mutex<Vec<PathBuf>>,
    finder: AtomicBool,
    revealed: Mutex<Vec<PathBuf>>,
    downloads: PathBuf,
    playbacks: AtomicUsize,
}

impl MediaPlatform for MockPlatform {
    fn playback(&self) -> Box<dyn AudioPlayback> {
        self.playbacks.fetch_add(1, Ordering::Relaxed);
        Box::new(MockPlayback {
            out: self.playback_out.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }
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
        playback_out: Arc::default(),
        drag: Mutex::default(),
        finder: AtomicBool::new(false),
        revealed: Mutex::default(),
        downloads: dir.join("Downloads/Relay"),
        playbacks: AtomicUsize::new(0),
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
            pc_to_mac: derive_udp_keys(&HH, UdpChannel::AudioPcToMac, true),
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
                audio_pc_to_mac: 0,
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
fn pc_audio_sine_reaches_playback_in_continuous_frames() {
    let mut w = World::new(0);
    w.media.set_audio_streams(true, false);
    let msgs = w.wait_msgs(|m| m.iter().any(|x| matches!(x, Message::AudioControl { .. })));
    assert!(msgs.contains(&Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false
    }));
    let dest = w.media.audio_rx_addr().expect("escutando");

    // O agente: pacotes de 5 ms em tempo real, cifrados com as chaves do outro lado.
    let agent = UdpAead::new(
        derive_udp_keys(&HH, UdpChannel::AudioPcToMac, false),
        ReplayMode::Window64,
    );
    let sock = UdpSocket::bind((LO, 0)).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let sender = thread::spawn(move || {
        let t0 = Instant::now();
        let mut out = Vec::new();
        for seq in 0u64..300 {
            while t0.elapsed() < Duration::from_millis(5 * seq) {
                thread::sleep(Duration::from_micros(300));
            }
            if s2.load(Ordering::Relaxed) {
                break;
            }
            let mut hdr = vec![0u8]; // stream 0
            hdr.extend_from_slice(&seq.to_le_bytes());
            hdr.extend_from_slice(&(seq * FRAMES_PER_PACKET as u64).to_le_bytes());
            let mut plain = vec![0u8, 2];
            plain.extend_from_slice(&(FRAMES_PER_PACKET as u16).to_le_bytes());
            for i in 0..FRAMES_PER_PACKET {
                let s = sine(seq as usize * FRAMES_PER_PACKET + i).to_le_bytes();
                plain.extend_from_slice(&s);
                plain.extend_from_slice(&s);
            }
            agent.seal(seq, &hdr, &plain, &mut out).unwrap();
            let mut dg = hdr.clone();
            dg.extend_from_slice(&out);
            sock.send_to(&dg, dest).unwrap();
        }
    });
    sender.join().unwrap();
    stop.store(true, Ordering::Relaxed);

    // Cauda do que tocou (a metade final do tempo de envio): senoide sem buracos nem saltos.
    let out = w.plat.playback_out.lock().unwrap().clone();
    let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
    let tail = &left[left.len().saturating_sub(48 * 600)..left.len().saturating_sub(48 * 120)];
    assert!(tail.len() > 48 * 400, "tocou pouco: {}", left.len());
    let max_step = tail
        .windows(2)
        .map(|p| (p[1] - p[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < AMP * 0.3, "salto na onda: {max_step}");
    for win in tail.chunks(240) {
        let peak = win.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > AMP * 0.8, "quadro mudo ou fraco: {peak}");
    }
    let stats = w
        .rec
        .audio
        .lock()
        .unwrap()
        .last()
        .copied()
        .expect("relay:audio-stats");
    assert!(
        (10.0..=30.0).contains(&stats.buffer_ms),
        "buffer fora de 20±10 ms: {stats:?}"
    );
    assert!(stats.loss_pct < 1.0, "{stats:?}");
}

#[test]
fn mic_frames_are_sent_sealed_to_the_agent() {
    let mut w = World::new(0);
    w.media.set_audio_streams(false, true);
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
    w.media.set_audio_streams(false, false);
    assert!(!w.media.sh.st().audio.tx_running());
}

#[test]
fn audio_without_keys_stays_off() {
    let w = World::new(0);
    let mut ctx = (*w.media.sh.session().unwrap()).clone();
    ctx.audio_keys = None;
    w.media.session_up(ctx);
    w.media.set_audio_streams(true, true);
    assert!(w.media.audio_rx_addr().is_none());
    assert_eq!(w.plat.playbacks.load(Ordering::Relaxed), 0);
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
    async fn clipboard_and_pc_audio_over_a_real_session() {
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
                    audio_pc_to_mac: 0,
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

        // Áudio: o pedido vira AudioControl; o agente manda senoide cifrada com as chaves do
        // handshake e ela toca no dispositivo mock.
        core.set_audio_streams(true, false).unwrap();
        assert!(core.status().audio.pc_to_mac);
        until("AudioControl no agente", || {
            agent.saw(|m| {
                matches!(
                    m,
                    Message::AudioControl {
                        pc_to_mac: true,
                        mac_to_pc: false
                    }
                )
            })
        })
        .await;
        until("áudio escutando", || media.audio_rx_addr().is_some()).await;
        let dest = media.audio_rx_addr().unwrap();
        let link = agent.link.lock().unwrap().clone().unwrap();
        let aead = UdpAead::new(link.pc_to_mac, ReplayMode::Window64);
        let sender = thread::spawn(move || {
            let sock = UdpSocket::bind((LO, 0)).unwrap();
            let t0 = Instant::now();
            let mut out = Vec::new();
            for seq in 0u64..200 {
                while t0.elapsed() < Duration::from_millis(5 * seq) {
                    thread::sleep(Duration::from_micros(300));
                }
                let mut hdr = vec![0u8];
                hdr.extend_from_slice(&seq.to_le_bytes());
                hdr.extend_from_slice(&(seq * 240).to_le_bytes());
                let mut plain = vec![0u8, 2, 240, 0];
                for i in 0..240 {
                    let s = sine(seq as usize * 240 + i).to_le_bytes();
                    plain.extend_from_slice(&s);
                    plain.extend_from_slice(&s);
                }
                aead.seal(seq, &hdr, &plain, &mut out).unwrap();
                hdr.extend_from_slice(&out);
                sock.send_to(&hdr, dest).unwrap();
            }
        });
        sender.join().unwrap();
        let out = plat.playback_out.lock().unwrap().clone();
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        let tail = &left[left.len().saturating_sub(48 * 500)..left.len().saturating_sub(48 * 100)];
        let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > AMP * 0.8, "sem senoide no playback: {peak}");
        core.shutdown();
    }
}
