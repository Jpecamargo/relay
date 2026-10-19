//! dono: WI3
//! Loopback no Mac: `Media` com mocks de clipboard e áudio contra um "Mac falso" (chaves de áudio,
//! `TransferServer`/`TransferClient` e `ClipboardSync` do core).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay_core::audio::{AudioHeader, FRAMES_PER_PACKET, HEADER_LEN, pcm_bytes_to_i16};
use relay_core::clipboard::ClipboardSync;
use relay_core::crypto::{
    MemoryKeyStore, ReplayMode, StaticKeypair, UdpAead, UdpChannel, derive_udp_keys, device_id_of,
    load_or_create_identity, save_peer,
};
use relay_core::engine::testutil::MockClip;
use relay_core::net::AudioLink;
use relay_core::platform::{
    AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, AudioPlayback, KeyStore, PlatformError,
};
use relay_core::transfer::{TransferClient, TransferServer};
use relay_core::types::{
    AudioDevice, AudioDevices, ClipboardContent, ClipboardMode, CommandResult, Config, FileEntry,
    Message, PairedPeer, PcAudioMode,
};
use tokio::sync::mpsc::UnboundedReceiver;

use super::audio::DATAGRAM_LEN;
use super::platform::{AudioHost, OutputGuard};
use super::{ACTIVE_PEER_ACCOUNT, Media, MediaDeps, MediaOut, MediaPort};
use relay_core::audio::{STREAM_MIC, seal_datagram};

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const HH: [u8; 32] = [9; 32];

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(
            // Folga para o runner compartilhado do CI.
            t.elapsed() < Duration::from_secs(20),
            "tempo esgotado: {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- mocks ----

type Log = Arc<Mutex<Vec<String>>>;

struct MockGuard(Log, bool);
impl OutputGuard for MockGuard {
    fn switch_to(&mut self, id: &str) -> Result<(), PlatformError> {
        self.0.lock().unwrap().push(format!("switch:{id}"));
        self.1 = true;
        Ok(())
    }
    fn restore(&mut self) -> Result<(), PlatformError> {
        if self.1 {
            self.1 = false;
            self.0.lock().unwrap().push("restore".into());
        }
        Ok(())
    }
}
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Captura sintética: senoide de 1 kHz em quadros de 5 ms.
struct MockCapture {
    log: Log,
    name: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioCapture for MockCapture {
    fn start(&mut self, mut sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("capture:{}", self.name));
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        self.thread = Some(std::thread::spawn(move || {
            let mut n = 0u64;
            while !stop.load(Ordering::Acquire) {
                let mut f = [0i16; AUDIO_FRAME_SAMPLES * 2];
                for i in 0..AUDIO_FRAME_SAMPLES {
                    let t = (n * AUDIO_FRAME_SAMPLES as u64 + i as u64) as f64 / 48_000.0;
                    let s = ((t * 1000.0 * std::f64::consts::TAU).sin() * 8000.0) as i16;
                    f[2 * i] = s;
                    f[2 * i + 1] = s;
                }
                let _ = sink.push(f);
                n += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
        }));
        Ok(())
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
    fn set_device(&mut self, _: Option<&str>) -> Result<(), PlatformError> {
        Ok(())
    }
}

struct MockRender {
    log: Log,
    id: String,
    out: Arc<Mutex<Vec<f32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioPlayback for MockRender {
    fn start(&mut self, mut source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        self.log.lock().unwrap().push(format!("render:{}", self.id));
        self.stop = Arc::new(AtomicBool::new(false));
        let (stop, out) = (self.stop.clone(), self.out.clone());
        self.thread = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                // O "dispositivo" consome 5 ms a cada 5 ms.
                std::thread::sleep(Duration::from_millis(5));
                let mut o = out.lock().unwrap();
                for _ in 0..AUDIO_FRAME_SAMPLES * 2 {
                    o.push(source.pop().unwrap_or(0.0));
                }
            }
        }));
        Ok(())
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
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

struct MockHost {
    devices: AudioDevices,
    log: Log,
    rendered: Arc<Mutex<Vec<f32>>>,
}

fn dev(id: &str, name: &str, is_default: bool) -> AudioDevice {
    AudioDevice {
        id: id.into(),
        name: name.into(),
        is_default,
    }
}

impl MockHost {
    fn new(outputs: Vec<AudioDevice>, inputs: Vec<AudioDevice>) -> Arc<Self> {
        Arc::new(Self {
            devices: AudioDevices { outputs, inputs },
            log: Log::default(),
            rendered: Arc::default(),
        })
    }
    fn capture(&self, name: &str) -> Box<dyn AudioCapture> {
        Box::new(MockCapture {
            log: self.log.clone(),
            name: name.into(),
            stop: Arc::default(),
            thread: None,
        })
    }
    fn events(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl AudioHost for MockHost {
    fn devices(&self) -> Result<AudioDevices, PlatformError> {
        Ok(self.devices.clone())
    }
    fn guard(&self) -> Result<Box<dyn OutputGuard>, PlatformError> {
        Ok(Box::new(MockGuard(self.log.clone(), false)))
    }
    fn loopback(&self, endpoint: Option<String>) -> Box<dyn AudioCapture> {
        self.capture(&format!("endpoint:{}", endpoint.unwrap_or_default()))
    }
    fn process_loopback(&self, exe: &str) -> Box<dyn AudioCapture> {
        self.capture(&format!("process:{exe}"))
    }
    fn render(&self, endpoint: Option<String>) -> Box<dyn AudioPlayback> {
        Box::new(MockRender {
            log: self.log.clone(),
            id: endpoint.unwrap_or_default(),
            out: self.rendered.clone(),
            stop: Arc::default(),
            thread: None,
        })
    }
}

fn speakers_and_cables() -> (Vec<AudioDevice>, Vec<AudioDevice>) {
    (
        vec![
            dev("spk", "Alto-falantes (Realtek)", true),
            dev("cab", "CABLE Input (VB-Audio Virtual Cable)", false),
            dev("cabA", "CABLE-A Input (VB-Audio Virtual Cable A)", false),
        ],
        vec![dev(
            "cabout",
            "CABLE Output (VB-Audio Virtual Cable)",
            false,
        )],
    )
}

fn agent_link() -> AudioLink {
    AudioLink {
        remote_ip: LO,
        pc_to_mac: derive_udp_keys(&HH, UdpChannel::AudioPcToMac, false),
        mac_to_pc: derive_udp_keys(&HH, UdpChannel::AudioMacToPc, false),
    }
}

struct Rig {
    media: Media,
    out: UnboundedReceiver<MediaOut>,
    hook: crate::core::MessageHook,
    clip: MockClip,
    store: Arc<MemoryKeyStore>,
    dropdir: PathBuf,
}

fn rig(host: Arc<MockHost>, config: Config, captures: super::CaptureSource) -> Rig {
    let clip = MockClip::default();
    let store = Arc::new(MemoryKeyStore::default());
    let dropdir = tempfile::tempdir().unwrap().keep();
    let d = dropdir.clone();
    let mut media = Media::start(
        MediaDeps {
            keystore: store.clone(),
            clipboard: Box::new(clip.clone()),
            audio: host,
            drop_target: Box::new(move || Some(d.clone())),
            temp_root: dropdir.join("tmp"),
            captures,
        },
        &config,
    );
    let out = media.take_outbox().unwrap();
    let hook = media.hook();
    Rig {
        media,
        out,
        hook,
        clip,
        store,
        dropdir,
    }
}

impl Rig {
    fn recv(&mut self, what: &str) -> MediaOut {
        wait(what, || self.out.try_recv().ok())
    }
    /// Próxima mensagem enviada (pula `State`).
    fn next_msg(&mut self, what: &str) -> Message {
        loop {
            if let MediaOut::Send(m) = self.recv(what) {
                return m;
            }
        }
    }
    fn feed(&mut self, m: Message) {
        assert!((self.hook)(&m).is_empty());
    }
}

fn no_captures() -> super::CaptureSource {
    Box::new(Vec::new)
}

fn audio_cfg(mode: PcAudioMode, virt: Option<&str>) -> Config {
    let mut c = Config::default();
    c.audio.pc_mode = mode;
    c.audio.virtual_output_name = virt.map(Into::into);
    c
}

fn expect_audio_error(r: &mut Rig, code: &str) {
    assert_eq!(
        r.next_msg("AudioControl"),
        Message::AudioControl {
            pc_to_mac: false,
            mac_to_pc: false
        }
    );
    assert_eq!(
        r.next_msg("erro"),
        Message::CommandResult {
            id: 0,
            result: CommandResult::err(0, code)
        }
    );
}

// ---- áudio ----

#[test]
fn system_mode_without_virtual_output_captures_the_default_output_without_switching() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut r = rig(
        host.clone(),
        audio_cfg(PcAudioMode::System, None),
        no_captures(),
    );
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    wait("ligado", || r.media.audio_state().pc_to_mac.then_some(()));
    assert_eq!(host.events(), ["capture:endpoint:"]);
}

#[test]
fn system_mode_leaves_cable_input_as_default_before_capturing_and_restores() {
    // O instalador do VB-CABLE deixa o "CABLE Input" como padrão: o loopback dele traria o mic do Mac.
    let (mut o, i) = speakers_and_cables();
    o[0].is_default = false;
    o[1].is_default = true;
    let host = MockHost::new(o, i);
    let mut r = rig(
        host.clone(),
        audio_cfg(PcAudioMode::System, None),
        no_captures(),
    );
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    wait("ligado", || r.media.audio_state().pc_to_mac.then_some(()));
    assert_eq!(host.events(), ["switch:spk", "capture:endpoint:spk"]);
    r.media.link_changed(None);
    wait("restaurado", || {
        (host.events().last().unwrap() == "restore").then_some(())
    });
}

#[test]
fn system_mode_with_a_missing_virtual_output_is_virtual_output_missing() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut r = rig(
        host.clone(),
        audio_cfg(PcAudioMode::System, Some("Saída que não existe")),
        no_captures(),
    );
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    expect_audio_error(&mut r, "virtual_output_missing");
    assert!(!r.media.audio_state().pc_to_mac);
    assert!(host.events().is_empty(), "não pode trocar a saída");
}

#[test]
fn system_mode_switches_output_streams_encrypted_and_restores() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut cfg = audio_cfg(PcAudioMode::System, Some("CABLE-A Input"));
    let mac = UdpSocket::bind("127.0.0.1:0").unwrap();
    mac.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    cfg.network.ports.audio_pc_to_mac = mac.local_addr().unwrap().port();
    let mut r = rig(host.clone(), cfg, no_captures());
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    wait("pc_to_mac ligado", || {
        r.media.audio_state().pc_to_mac.then_some(())
    });
    assert_eq!(
        host.events()[..2],
        [
            "switch:cabA".to_string(),
            "capture:endpoint:cabA".to_string()
        ]
    );

    // O Mac decifra com as chaves do lado dialer.
    let mut aead = UdpAead::new(
        derive_udp_keys(&HH, UdpChannel::AudioPcToMac, true),
        ReplayMode::Window64,
    );
    let (mut wire, mut plain) = ([0u8; 2048], Vec::new());
    let mut last = None;
    for _ in 0..20 {
        let n = mac.recv(&mut wire).unwrap();
        assert_eq!(n, DATAGRAM_LEN);
        let (seq, payload) = relay_core::audio::open_datagram(&mut aead, &wire[..n], 0, &mut plain)
            .expect("decifra");
        if let Some(l) = last {
            assert_eq!(seq, l + 1, "quadros contínuos");
        }
        last = Some(seq);
        let mut pcm = vec![0i16; FRAMES_PER_PACKET * 2];
        pcm_bytes_to_i16(payload, &mut pcm).unwrap();
        assert!(pcm.iter().any(|s| s.abs() > 4000), "senoide presente");
    }

    // Cair a conexão restaura a saída original.
    r.media.link_changed(None);
    wait("pc desligado", || {
        (!r.media.audio_state().pc_to_mac).then_some(())
    });
    assert_eq!(host.events().last().unwrap(), "restore");

    // Desligar o agente no meio do streaming também restaura.
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    wait("religado", || r.media.audio_state().pc_to_mac.then_some(()));
    drop(r);
    let ev = host.events();
    assert_eq!(ev.iter().filter(|e| *e == "restore").count(), 2, "{ev:?}");
}

#[test]
fn process_mode_captures_the_configured_process_without_touching_the_default_output() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut cfg = audio_cfg(PcAudioMode::Process, None);
    cfg.audio.process_names = vec!["Discord.exe".into()];
    let mut r = rig(host.clone(), cfg, no_captures());
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: true,
        mac_to_pc: false,
    });
    wait("ligado", || r.media.audio_state().pc_to_mac.then_some(()));
    assert_eq!(host.events(), ["capture:process:Discord.exe"]);
}

#[test]
fn mic_without_vbcable_is_vbcable_missing() {
    // Só o cabo A+B: "CABLE-A Input" não é o "CABLE Input".
    let host = MockHost::new(vec![dev("cabA", "CABLE-A Input (VB-Audio)", false)], vec![]);
    let mut r = rig(host, Config::default(), no_captures());
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: false,
        mac_to_pc: true,
    });
    expect_audio_error(&mut r, "vbcable_missing");
}

#[test]
fn mic_from_mac_reaches_cable_input_through_the_jitter_buffer() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut cfg = Config::default();
    cfg.network.ports.audio_mac_to_pc = 0;
    let mut r = rig(host.clone(), cfg, no_captures());
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::AudioControl {
        pc_to_mac: false,
        mac_to_pc: true,
    });
    wait("mic ligado", || {
        r.media.audio_state().mac_to_pc.then_some(())
    });
    assert!(host.events().contains(&"render:cab".to_string()));
    let port = wait("porta do mic", || r.media.addrs().mic).port();

    let aead = UdpAead::new(
        derive_udp_keys(&HH, UdpChannel::AudioMacToPc, true),
        ReplayMode::Window64,
    );
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let dest = SocketAddr::new(LO, port);
    let (mut plain, mut cipher, mut wire) = (Vec::new(), Vec::new(), Vec::new());
    for seq in 0..200u64 {
        let mut f: AudioFrame = [0; AUDIO_FRAME_SAMPLES * 2];
        for k in 0..AUDIO_FRAME_SAMPLES {
            let t = (seq * AUDIO_FRAME_SAMPLES as u64 + k as u64) as f64 / 48_000.0;
            let s = ((t * 1000.0 * std::f64::consts::TAU).sin() * 8000.0) as i16;
            f[2 * k] = s;
            f[2 * k + 1] = s;
        }
        assert!(seal_datagram(
            &aead,
            STREAM_MIC,
            seq,
            &f,
            &mut plain,
            &mut cipher,
            &mut wire
        ));
        sock.send_to(&wire, dest).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    // Pacote adulterado é descartado sem derrubar nada.
    let mut bad = wire.clone();
    bad[30] ^= 1;
    sock.send_to(&bad, dest).unwrap();

    let out = host.rendered.lock().unwrap().clone();
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(
        peak > 0.2,
        "o som do Mac chegou ao CABLE Input (pico {peak})"
    );
    let start = out.iter().position(|s| s.abs() > 0.05).unwrap();
    // Depois da partida quase tudo é som: os relógios do teste são de `sleep`, então um
    // rebuffer ocasional é tolerado, mas não silêncio contínuo.
    let body = &out[start..start + 480 * 100];
    let silent = body.chunks(2).filter(|s| s[0].abs() < 0.001).count();
    assert!(
        silent < 100 * FRAMES_PER_PACKET / 4,
        "{silent} quadros de silêncio"
    );
    // O cabeçalho do AudioPacket tem 21 bytes: 17 de AAD + 4 cifrados.
    assert_eq!(HEADER_LEN, 21);
    let _ = AudioHeader::parse;
}

// ---- clipboard ----

fn offer_roundtrip(text: &str) -> (Rig, Rig) {
    let mut cfg = Config::default();
    cfg.clipboard.mode = ClipboardMode::OnCopy;
    let a = rig(MockHost::new(vec![], vec![]), cfg.clone(), no_captures());
    let b = rig(MockHost::new(vec![], vec![]), cfg, no_captures());
    a.clip.copy(ClipboardContent::Text(text.into()), vec![]);
    (a, b)
}

#[test]
fn text_round_trips_between_two_clipboards_without_echo() {
    let (mut a, mut b) = offer_roundtrip("olá, relay");
    let offer = a.next_msg("oferta");
    assert!(matches!(offer, Message::ClipboardOffer { .. }));
    b.feed(offer);
    b.feed(a.next_msg("bloco"));
    wait("aplicado", || (!b.clip.written().is_empty()).then_some(()));
    assert_eq!(
        b.clip.written(),
        [ClipboardContent::Text("olá, relay".into())]
    );
    // O conteúdo remoto foi marcado: não volta como nova oferta.
    std::thread::sleep(Duration::from_millis(150));
    assert!(b.out.try_recv().is_err() || matches!(b.out.try_recv(), Ok(MediaOut::State)));
}

#[test]
fn on_switch_and_send_now_offer_the_current_clipboard() {
    let mut r = rig(
        MockHost::new(vec![], vec![]),
        Config::default(),
        no_captures(),
    );
    r.clip.0.lock().unwrap().content = Some(ClipboardContent::Text("troca".into()));
    let mut ev = r.media.engine_events();
    ev(relay_core::engine::EngineEvent::ClipboardSwitch);
    assert!(matches!(
        r.next_msg("oferta"),
        Message::ClipboardOffer { .. }
    ));
    assert!(matches!(
        r.next_msg("bloco"),
        Message::ClipboardChunk { .. }
    ));
    // Mesmo conteúdo: só "enviar agora" reenvia.
    ev(relay_core::engine::EngineEvent::ClipboardSwitch);
    ev(relay_core::engine::EngineEvent::Hotkey(
        relay_core::types::HotkeyAction::ClipboardSendNow,
    ));
    assert!(matches!(
        r.next_msg("reenvio"),
        Message::ClipboardOffer { .. }
    ));
}

#[test]
fn screen_capture_png_becomes_a_png_clipboard_offer() {
    let pending = Arc::new(Mutex::new(vec![vec![0x89, b'P', b'N', b'G', 1, 2, 3]]));
    let p = pending.clone();
    let mut r = rig(
        MockHost::new(vec![], vec![]),
        Config::default(),
        Box::new(move || std::mem::take(&mut *p.lock().unwrap())),
    );
    match r.next_msg("oferta Png") {
        Message::ClipboardOffer { formats, .. } => {
            assert_eq!(formats, [relay_core::types::ClipboardFormat::Png]);
        }
        m => panic!("{m:?}"),
    }
    // O Mac remonta o PNG original.
    let mut mac = ClipboardSync::new(Config::default().clipboard);
    let Message::ClipboardChunk { .. } = r.next_msg("bloco") else {
        panic!()
    };
    let _ = &mut mac;
}

// ---- arquivos ----

struct MacFiles {
    rt: tokio::runtime::Runtime,
    mac: StaticKeypair,
    addr: SocketAddr,
    agent_peer: PairedPeer,
    mac_peer: PairedPeer,
}

fn peer_of(k: &StaticKeypair, name: &str) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&k.public),
        name: name.into(),
        public_key: k.public,
        mac_address: [0; 6],
    }
}

/// "Mac" servindo `paths`, pareado com o KeyStore do agente do `rig`.
fn mac_files(r: &Rig, paths: Vec<PathBuf>) -> MacFiles {
    let agent = load_or_create_identity(&*r.store).unwrap();
    let mac = load_or_create_identity(&MemoryKeyStore::default()).unwrap();
    let (agent_peer, mac_peer) = (peer_of(&agent, "PC"), peer_of(&mac, "Mac"));
    save_peer(&*r.store, &mac_peer).unwrap();
    r.store
        .set(ACTIVE_PEER_ACCOUNT, mac_peer.device_id.as_bytes())
        .unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = Arc::new(TransferServer::new(
        mac.clone(),
        vec![agent_peer.clone()],
        paths,
    ));
    let listener = rt
        .block_on(TransferServer::bind(Some("127.0.0.1:0".parse().unwrap())))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(server.serve(listener));
    MacFiles {
        rt,
        mac,
        addr,
        agent_peer,
        mac_peer,
    }
}

fn big_file(dir: &Path, name: &str, len: usize) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(
        &p,
        (0..len)
            .map(|i| (i.wrapping_mul(31) >> 3) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    p
}

#[test]
fn drop_files_downloads_into_the_destination_folder() {
    let src = tempfile::tempdir().unwrap();
    let file = big_file(src.path(), "foto.bin", 5 * 1024 * 1024);
    let mut cfg = Config::default();
    let mut r = rig(MockHost::new(vec![], vec![]), cfg.clone(), no_captures());
    let m = mac_files(&r, vec![file.clone()]);
    cfg.network.ports.transfer = m.addr.port();
    r.media.config_changed(&cfg);
    r.media.link_changed(Some(agent_link()));
    r.feed(Message::FileOffer {
        offer_id: 7,
        files: vec![FileEntry {
            id: 0,
            name: "foto.bin".into(),
            size: 5 * 1024 * 1024,
            is_dir: false,
        }],
    });
    r.feed(Message::DropFiles {
        offer_id: 7,
        point_px: (10, 10),
    });
    let dst = r.dropdir.join("foto.bin");
    wait("arquivo baixado", || {
        std::fs::metadata(&dst)
            .ok()
            .filter(|m| m.len() == 5 * 1024 * 1024)
    });
    assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&file).unwrap());
    drop(m);
}

#[test]
fn remote_file_list_is_rendered_lazily_into_temp_relay_offer_dir() {
    let src = tempfile::tempdir().unwrap();
    let file = big_file(src.path(), "doc.bin", 300_000);
    let mut r = rig(
        MockHost::new(vec![], vec![]),
        Config::default(),
        no_captures(),
    );
    let m = mac_files(&r, vec![file.clone()]);
    let mut cfg = Config::default();
    cfg.network.ports.transfer = m.addr.port();
    r.media.config_changed(&cfg);
    r.media.link_changed(Some(agent_link()));

    // O Mac oferece uma FileList pelo clipboard.
    let entries = vec![FileEntry {
        id: 0,
        name: "doc.bin".into(),
        size: 300_000,
        is_dir: false,
    }];
    let mut mac_sync = ClipboardSync::new(Config::default().clipboard);
    for msg in mac_sync.send_now(Some(ClipboardContent::FileList(entries.clone()))) {
        r.feed(msg);
    }
    wait("FileList aplicada", || {
        (!r.clip.written().is_empty()).then_some(())
    });
    assert_eq!(
        r.clip.written(),
        [ClipboardContent::FileList(entries.clone())]
    );
    // Nada baixado até o Explorer colar.
    assert!(!r.dropdir.join("tmp").exists());

    // O Explorer cola: o provedor baixa e devolve os caminhos locais.
    let provider = r.clip.0.lock().unwrap().provider.clone().unwrap();
    let paths = provider(&entries).unwrap();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].starts_with(r.dropdir.join("tmp").join("Relay")));
    assert_eq!(
        std::fs::read(&paths[0]).unwrap(),
        std::fs::read(&file).unwrap()
    );
    drop(m);
}

#[test]
fn local_file_list_is_served_to_the_mac() {
    let src = tempfile::tempdir().unwrap();
    let file = big_file(src.path(), "saida.bin", 200_000);
    let mut cfg = Config::default();
    cfg.clipboard.mode = ClipboardMode::OnCopy;
    cfg.network.ports.transfer = 0;
    let mut r = rig(MockHost::new(vec![], vec![]), cfg, no_captures());
    let m = mac_files(&r, vec![]);
    r.media.link_changed(Some(agent_link()));
    let addr = wait("escuta de arquivos", || r.media.addrs().transfer);
    r.clip.copy(
        ClipboardContent::FileList(vec![FileEntry {
            id: 0,
            name: "saida.bin".into(),
            size: 200_000,
            is_dir: false,
        }]),
        vec![file.clone()],
    );
    assert!(matches!(
        r.next_msg("oferta"),
        Message::ClipboardOffer { .. }
    ));

    let dest = tempfile::tempdir().unwrap();
    let got = m.rt.block_on(async {
        let mut io = TransferClient::connect(SocketAddr::new(LO, addr.port()))
            .await
            .unwrap();
        TransferClient::new(m.mac.clone(), peer_of_agent(&m))
            .download(
                &mut io,
                &[0],
                dest.path(),
                1,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .await
            .unwrap()
    });
    assert_eq!(
        std::fs::read(&got[0]).unwrap(),
        std::fs::read(&file).unwrap()
    );
    let _ = &m.mac_peer;
}

fn peer_of_agent(m: &MacFiles) -> PairedPeer {
    m.agent_peer.clone()
}

#[test]
fn screen_capture_is_offered_even_with_clipboard_sync_off() {
    let pending = Arc::new(Mutex::new(vec![vec![0x89, b'P', b'N', b'G', 9]]));
    let p = pending.clone();
    let mut cfg = Config::default();
    cfg.clipboard.enabled = false;
    let mut r = rig(
        MockHost::new(vec![], vec![]),
        cfg,
        Box::new(move || std::mem::take(&mut *p.lock().unwrap())),
    );
    assert!(matches!(
        r.next_msg("oferta Png"),
        Message::ClipboardOffer { .. }
    ));
}

#[test]
fn pc_audio_seq_never_restarts_within_a_session() {
    let (o, i) = speakers_and_cables();
    let host = MockHost::new(o, i);
    let mut cfg = audio_cfg(PcAudioMode::Process, None);
    let mac = UdpSocket::bind("127.0.0.1:0").unwrap();
    mac.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    cfg.network.ports.audio_pc_to_mac = mac.local_addr().unwrap().port();
    let mut r = rig(host, cfg, no_captures());
    r.media.link_changed(Some(agent_link()));
    let on = |r: &mut Rig, v: bool| {
        r.feed(Message::AudioControl {
            pc_to_mac: v,
            mac_to_pc: false,
        });
        wait("estado", || {
            (r.media.audio_state().pc_to_mac == v).then_some(())
        });
    };
    let mut wire = [0u8; 2048];
    let mut max_seq = |mac: &UdpSocket| {
        let mut hi = 0;
        for _ in 0..10 {
            let n = mac.recv(&mut wire).unwrap();
            hi = hi.max(u64::from_le_bytes(wire[1..9].try_into().unwrap()));
            assert_eq!(n, DATAGRAM_LEN);
        }
        hi
    };
    on(&mut r, true);
    let first = max_seq(&mac);
    on(&mut r, false);
    std::thread::sleep(Duration::from_millis(50));
    while mac.set_nonblocking(true).is_ok() && mac.recv(&mut wire).is_ok() {}
    mac.set_nonblocking(false).unwrap();
    on(&mut r, true);
    let n = mac.recv(&mut wire).unwrap();
    assert_eq!(n, DATAGRAM_LEN);
    let again = u64::from_le_bytes(wire[1..9].try_into().unwrap());
    assert!(again > first, "seq recomeçou: {again} <= {first}");
}
