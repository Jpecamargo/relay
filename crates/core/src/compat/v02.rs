//! Formato de fio das `Message` da 0.2.x: amostras fixas e o fixture golden gerado com o código
//! da base 0.2.0 (`testdata/v02_messages.hex`). Um peer 0.2 derruba a sessão ao ver bytes
//! diferentes dos que conhece, então estes testes não podem ter o fixture regerado.

#[cfg(test)]
mod tests {
    use crate::protocol::{FrameReader, FrameWriter};
    use crate::types::*;

    /// Uma amostra fixa e determinística por variante da 0.2.x, na ordem do enum.
    fn samples() -> Vec<(&'static str, Message)> {
        let display = DisplayInfo {
            id: "D1".into(),
            name: "Tela".into(),
            machine: Machine::Mac,
            origin_px: (0, -10),
            size_px: (1920, 1080),
            scale: 2.0,
            size_mm: Some((600.0, 340.0)),
            is_primary: true,
            is_builtin: false,
        };
        let profile = LayoutProfile {
            key: ProfileKey(vec!["D1".into(), "W1".into()]),
            name: "Mesa".into(),
            displays: vec![PlacedDisplay {
                display_id: "D1".into(),
                machine: Machine::Windows,
                rect_mm: RectMm {
                    x: 1.0,
                    y: 2.5,
                    w: 600.0,
                    h: 340.0,
                },
                sensitivity: 1.5,
                rotation: Some(90),
            }],
            windows_detached: true,
        };
        vec![
            (
                "Hello",
                Message::Hello {
                    protocol_version: 1,
                    device_id: "0123456789abcdef".into(),
                    role: Role::Dialer,
                    app_version: "0.2.0".into(),
                },
            ),
            ("HelloAck", Message::HelloAck),
            (
                "Ping",
                Message::Ping {
                    id: 7,
                    ts_us: 1_700_000_000_000,
                },
            ),
            (
                "Pong",
                Message::Pong {
                    id: 7,
                    ts_us: 1_700_000_000_001,
                },
            ),
            (
                "Input",
                Message::Input {
                    seq: 300,
                    ts_us: 123_456,
                    event: InputEvent::Key {
                        scan: 0xE04D,
                        down: true,
                    },
                },
            ),
            ("Displays", Message::Displays(vec![display])),
            ("LayoutSync", Message::LayoutSync(profile)),
            (
                "FocusEnter",
                Message::FocusEnter {
                    display_id: "W1".into(),
                    entry_px: (0, 540),
                },
            ),
            (
                "FocusReturn",
                Message::FocusReturn {
                    display_id: "D1".into(),
                    point_px: (1919, -3),
                },
            ),
            (
                "ForegroundApp",
                Message::ForegroundApp {
                    exe: "chrome.exe".into(),
                },
            ),
            (
                "Command",
                Message::Command {
                    id: 9,
                    cmd: RemoteCommand::Power {
                        action: PowerAction::Restart,
                        confirm: true,
                    },
                },
            ),
            (
                "CommandResult",
                Message::CommandResult {
                    id: 9,
                    result: CommandResult::err(9, "unsupported"),
                },
            ),
            (
                "ClipboardOffer",
                Message::ClipboardOffer {
                    offer_id: 3,
                    formats: vec![ClipboardFormat::Text, ClipboardFormat::Png],
                    total: 70_000,
                },
            ),
            (
                "ClipboardChunk",
                Message::ClipboardChunk {
                    offer_id: 3,
                    index: 1,
                    data: vec![0, 1, 2, 253, 254, 255],
                },
            ),
            (
                "ClipboardSkipped",
                Message::ClipboardSkipped {
                    reason: "too_big".into(),
                },
            ),
            (
                "FileOffer",
                Message::FileOffer {
                    offer_id: 4,
                    files: vec![FileEntry {
                        id: 0,
                        name: "a.txt".into(),
                        size: 12,
                        is_dir: false,
                    }],
                },
            ),
            (
                "DropFiles",
                Message::DropFiles {
                    offer_id: 4,
                    point_px: (10, 20),
                },
            ),
            (
                "AudioControl",
                Message::AudioControl {
                    pc_to_mac: false,
                    mac_to_pc: true,
                },
            ),
            (
                "Metrics",
                Message::Metrics(SystemMetrics {
                    cpu_pct: 12.5,
                    ram_used_mb: 8192,
                    ram_total_mb: 32768,
                    gpu_pct: Some(40.0),
                    gpu_mem_mb: Some(2048),
                    gpu_temp_c: None,
                    cpu_temp_c: Some(55.5),
                    volume: Some(0.5),
                    muted: Some(false),
                }),
            ),
            ("TakeControl", Message::TakeControl),
            ("Unpair", Message::Unpair),
            ("Bye", Message::Bye),
            (
                "RequestUpdate",
                Message::RequestUpdate {
                    version: "0.2.0".into(),
                },
            ),
        ]
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn from_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn encode(msg: &Message) -> Vec<u8> {
        let mut out = Vec::new();
        FrameWriter.encode(msg, &mut out).expect("encode");
        out
    }

    fn fixture() -> Vec<(String, String)> {
        include_str!("testdata/v02_messages.hex")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (name, hex) = l.split_once(' ').expect("'<Variante> <hex>'");
                (name.to_owned(), hex.trim().to_owned())
            })
            .collect()
    }

    #[test]
    fn golden_encode_v02() {
        let fixture = fixture();
        let samples = samples();
        assert_eq!(fixture.len(), samples.len());
        for ((name, msg), (fixture_name, hex)) in samples.iter().zip(&fixture) {
            assert_eq!(name, fixture_name);
            assert_eq!(&to_hex(&encode(msg)), hex, "{name}");
        }
    }

    #[test]
    fn golden_decode_v02() {
        let fixture = fixture();
        let samples = samples();
        assert_eq!(fixture.len(), samples.len());
        for ((name, msg), (_, hex)) in samples.iter().zip(&fixture) {
            let decoded = FrameReader::default().push(&from_hex(hex)).expect(name);
            assert_eq!(decoded, vec![msg.clone()], "{name}");
        }
    }
}
