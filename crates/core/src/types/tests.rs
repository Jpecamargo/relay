//! dono: F2
//! Round-trip de serialização dos tipos do contrato.

use super::consts::*;
use super::*;

fn bincode_roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let cfg = bincode::config::standard();
    let bytes = bincode::serde::encode_to_vec(value, cfg).expect("encode");
    let (back, used): (T, usize) = bincode::serde::decode_from_slice(&bytes, cfg).expect("decode");
    assert_eq!(&back, value);
    assert_eq!(used, bytes.len());
}

fn display(id: &str, machine: Machine) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: "Tela".into(),
        machine,
        origin_px: (-1920, 0),
        size_px: (2560, 1440),
        scale: 1.5,
        size_mm: Some((597.0, 336.0)),
        is_primary: false,
        is_builtin: false,
    }
}

fn profile() -> LayoutProfile {
    LayoutProfile {
        key: ProfileKey(vec!["a".into(), "b".into()]),
        name: "Setup completo".into(),
        displays: vec![PlacedDisplay {
            display_id: "a".into(),
            machine: Machine::Mac,
            rect_mm: RectMm {
                x: 0.0,
                y: 0.0,
                w: 302.0,
                h: 196.0,
            },
            sensitivity: 1.0,
            rotation: None,
        }],
        windows_detached: false,
    }
}

/// Exaustivo de propósito: variante nova em `Message` quebra a compilação aqui, lembrando de
/// incluí-la em `all_messages`.
fn variant_name(m: &Message) -> &'static str {
    match m {
        Message::Hello { .. } => "Hello",
        Message::HelloAck => "HelloAck",
        Message::Ping { .. } => "Ping",
        Message::Pong { .. } => "Pong",
        Message::Input { .. } => "Input",
        Message::Displays(_) => "Displays",
        Message::LayoutSync(_) => "LayoutSync",
        Message::FocusEnter { .. } => "FocusEnter",
        Message::FocusReturn { .. } => "FocusReturn",
        Message::ForegroundApp { .. } => "ForegroundApp",
        Message::Command { .. } => "Command",
        Message::CommandResult { .. } => "CommandResult",
        Message::ClipboardOffer { .. } => "ClipboardOffer",
        Message::ClipboardChunk { .. } => "ClipboardChunk",
        Message::ClipboardSkipped { .. } => "ClipboardSkipped",
        Message::FileOffer { .. } => "FileOffer",
        Message::DropFiles { .. } => "DropFiles",
        Message::AudioControl { .. } => "AudioControl",
        Message::Metrics(_) => "Metrics",
        Message::TakeControl => "TakeControl",
        Message::Unpair => "Unpair",
        Message::Bye => "Bye",
        Message::RequestUpdate { .. } => "RequestUpdate",
    }
}

fn all_messages() -> Vec<Message> {
    vec![
        Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id: "0011223344556677".into(),
            role: Role::Dialer,
            app_version: "0.1.0".into(),
        },
        Message::HelloAck,
        Message::Ping {
            id: 1,
            ts_us: u64::MAX,
        },
        Message::Pong {
            id: 2,
            ts_us: 123_456_789,
        },
        Message::Input {
            seq: u64::MAX - 1,
            ts_us: 42,
            event: InputEvent::Scroll {
                dx: -120,
                dy: 240,
                phase: ScrollPhase::Momentum,
            },
        },
        Message::Displays(vec![
            display("d1", Machine::Windows),
            display("d2", Machine::Mac),
        ]),
        Message::LayoutSync(profile()),
        Message::FocusEnter {
            display_id: "d1".into(),
            entry_px: (0, 540),
        },
        Message::FocusReturn {
            display_id: "d2".into(),
            point_px: (-5, 10),
        },
        Message::ForegroundApp {
            exe: "WindowsTerminal.exe".into(),
        },
        Message::Command {
            id: 7,
            cmd: RemoteCommand::GetMetrics,
        },
        Message::CommandResult {
            id: 7,
            result: CommandResult::ok(7, Some(CommandOutput::Text("ok".into()))),
        },
        Message::ClipboardOffer {
            offer_id: 3,
            formats: vec![ClipboardFormat::Text, ClipboardFormat::Png],
            total: 5 * 1024 * 1024 * 1024,
        },
        Message::ClipboardChunk {
            offer_id: 3,
            index: 9,
            data: vec![0, 1, 2, 255],
        },
        Message::ClipboardSkipped {
            reason: "too_large".into(),
        },
        Message::FileOffer {
            offer_id: 4,
            files: vec![FileEntry {
                id: 0,
                name: "a.txt".into(),
                size: 10,
                is_dir: false,
            }],
        },
        Message::DropFiles {
            offer_id: 4,
            point_px: (100, 200),
        },
        Message::AudioControl {
            pc_to_mac: true,
            mac_to_pc: false,
        },
        Message::Metrics(SystemMetrics {
            cpu_pct: 12.5,
            ram_used_mb: 8000,
            ram_total_mb: 32000,
            gpu_pct: Some(40.0),
            gpu_mem_mb: None,
            gpu_temp_c: Some(61.0),
            cpu_temp_c: None,
            volume: Some(0.5),
            muted: Some(false),
        }),
        Message::TakeControl,
        Message::Unpair,
        Message::Bye,
        Message::RequestUpdate {
            version: "0.2.0".into(),
        },
    ]
}

#[test]
fn message_variants_roundtrip_bincode() {
    let all = all_messages();
    let mut names: Vec<_> = all.iter().map(variant_name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        all.len(),
        "uma amostra por variante, sem repetição"
    );
    assert_eq!(all.len(), 23);
    for m in &all {
        bincode_roundtrip(m);
    }
}

#[test]
fn remote_command_variants_roundtrip_bincode() {
    let all = vec![
        RemoteCommand::SetVolume { level: 0.5 },
        RemoteCommand::SetMute { muted: true },
        RemoteCommand::ListOutputDevices,
        RemoteCommand::SetOutputDevice {
            id: "{0.0.0}".into(),
        },
        RemoteCommand::Media {
            action: MediaAction::PlayPause,
        },
        RemoteCommand::Power {
            action: PowerAction::Shutdown,
            confirm: true,
        },
        RemoteCommand::CaptureScreen {
            display_id: "d1".into(),
        },
        RemoteCommand::OpenTarget {
            target: "https://example.com".into(),
        },
        RemoteCommand::TypeText {
            text: "olá 😀".into(),
        },
        RemoteCommand::GetMetrics,
    ];
    for c in &all {
        bincode_roundtrip(c);
        bincode_roundtrip(&Message::Command {
            id: 1,
            cmd: c.clone(),
        });
    }
}

#[test]
fn input_events_roundtrip_bincode() {
    let all = [
        InputEvent::MouseMove { dx: -3, dy: 4 },
        InputEvent::MouseAbs { x: -100, y: 2000 },
        InputEvent::Button {
            button: MouseButton::X2,
            down: true,
        },
        InputEvent::Scroll {
            dx: 0,
            dy: -120,
            phase: ScrollPhase::Began,
        },
        InputEvent::Key {
            scan: 0xE04D,
            down: false,
        },
        InputEvent::Media {
            key: MediaKey::Next,
            down: true,
        },
    ];
    for e in &all {
        bincode_roundtrip(e);
    }
    for p in 0..=4u8 {
        assert_eq!(ScrollPhase::from_u8(p).map(ScrollPhase::as_u8), Some(p));
    }
    assert_eq!(ScrollPhase::from_u8(5), None);
}

#[test]
fn command_output_variants_roundtrip_bincode() {
    bincode_roundtrip(&CommandOutput::Devices(vec![AudioDevice {
        id: "x".into(),
        name: "Alto-falantes".into(),
        is_default: true,
    }]));
    bincode_roundtrip(&CommandOutput::Png(vec![137, 80, 78, 71]));
    bincode_roundtrip(&CommandResult::err(9, error_code::TIMEOUT));
}

#[test]
fn config_roundtrips_through_toml_and_bincode() {
    let mut cfg = Config::default();
    cfg.hotkeys.push(HotkeyBinding {
        combo: KeyCombo {
            modifiers: vec![Modifier::Ctrl, Modifier::Shift],
            key: "KeyM".into(),
        },
        action: HotkeyAction::Command(RemoteCommand::Media {
            action: MediaAction::PlayPause,
        }),
    });
    cfg.layout.profiles.push(profile());
    cfg.keymap.rules.push(KeymapRule {
        id: "copy".into(),
        enabled: false,
        from: None,
        to: None,
    });
    cfg.network.manual_address = Some("192.168.0.10".into());
    let text = toml::to_string(&cfg).expect("toml encode");
    let back: Config = toml::from_str(&text).expect("toml decode");
    assert_eq!(back, cfg);
    bincode_roundtrip(&cfg);
    // Campos ausentes caem nos padrões (compatibilidade com arquivos antigos).
    let partial: Config = toml::from_str("[transition]\ndwell_ms = 250\n").expect("parcial");
    assert_eq!(partial.transition.dwell_ms, 250);
    assert_eq!(partial.network.ports.control, PORT_CONTROL);
    assert_eq!(partial.schema_version, CONFIG_SCHEMA_VERSION);
}

#[test]
fn consts_match_the_glossary() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(
        [
            PORT_CONTROL,
            PORT_INPUT,
            PORT_AUDIO_PC_TO_MAC,
            PORT_AUDIO_MAC_TO_PC,
            PORT_TRANSFER
        ],
        [47800, 47801, 47802, 47803, 47804]
    );
    assert_eq!(MDNS_SERVICE_TYPE, "_relay._tcp.local.");
    assert_eq!(KEYCHAIN_SERVICE, "com.joaocamargo.relay");
    assert_eq!((MAX_FRAME, CHUNK_SIZE, HEARTBEAT_MS), (65000, 61440, 500));
    const { assert!(CHUNK_SIZE < MAX_FRAME) };
}
