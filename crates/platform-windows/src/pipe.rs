//! dono: W7
//! Pipe `\\.\pipe\relay-inject`: `InjectFrame` de 32 bytes (puro) e o cliente usado pelo agente.
//!
//! O helper do `RelayService` (SYSTEM, na sessão ativa) é o servidor; o agente (sem elevação) é o
//! cliente. O pipe só aceita SYSTEM e o usuário interativo da sessão (ver `service::pipe_sddl`).
//! Tudo aqui compila e é testado em qualquer target: `std::fs::File` abre named pipes no Windows,
//! e nos demais targets a abertura simplesmente falha (helper indisponível).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::{InputEvent, MediaKey, MouseButton, ScrollPhase};

/// Nome do pipe do helper (glossário).
pub const PIPE_NAME: &str = r"\\.\pipe\relay-inject";
/// Tamanho fixo de um frame no fio.
pub const FRAME_LEN: usize = 32;
/// Intervalo mínimo entre tentativas de (re)conexão do cliente.
pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

const MAGIC: [u8; 2] = *b"RI";
const VERSION: u8 = 1;

const KIND_MOUSE_MOVE: u8 = 1;
const KIND_MOUSE_ABS: u8 = 2;
const KIND_BUTTON: u8 = 3;
const KIND_SCROLL: u8 = 4;
const KIND_KEY: u8 = 5;
const KIND_MEDIA: u8 = 6;
const KIND_TEXT_UNIT: u8 = 7;

/// Frame do pipe: um `InputEvent` ou uma unidade UTF-16 de texto (`type_text`).
///
/// Layout (little-endian): `[0..2]` magia `RI`, `[2]` versão, `[3]` tipo, `[4]` down (0/1),
/// `[5]` sub (botão, tecla de mídia ou fase do scroll), `[6..8]` scan/unidade u16,
/// `[8..12]` a i32, `[12..16]` b i32, `[16..32]` reservado (zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectFrame {
    Event(InputEvent),
    /// Uma unidade de código UTF-16 (pares substitutos viajam em dois frames seguidos).
    TextUnit(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    BadMagic,
    BadVersion(u8),
    BadKind(u8),
    BadField,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::BadMagic => write!(f, "magia inválida"),
            FrameError::BadVersion(v) => write!(f, "versão {v} não suportada"),
            FrameError::BadKind(k) => write!(f, "tipo {k} desconhecido"),
            FrameError::BadField => write!(f, "campo fora do intervalo"),
        }
    }
}

impl std::error::Error for FrameError {}

fn button_to_u8(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
    }
}

fn button_from_u8(v: u8) -> Option<MouseButton> {
    Some(match v {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::X1,
        4 => MouseButton::X2,
        _ => return None,
    })
}

fn media_to_u8(k: MediaKey) -> u8 {
    match k {
        MediaKey::VolumeUp => 0,
        MediaKey::VolumeDown => 1,
        MediaKey::Mute => 2,
        MediaKey::PlayPause => 3,
        MediaKey::Next => 4,
        MediaKey::Previous => 5,
    }
}

fn media_from_u8(v: u8) -> Option<MediaKey> {
    Some(match v {
        0 => MediaKey::VolumeUp,
        1 => MediaKey::VolumeDown,
        2 => MediaKey::Mute,
        3 => MediaKey::PlayPause,
        4 => MediaKey::Next,
        5 => MediaKey::Previous,
        _ => return None,
    })
}

impl InjectFrame {
    pub fn encode(&self) -> [u8; FRAME_LEN] {
        let (kind, down, sub, scan, a, b) = match *self {
            InjectFrame::Event(ev) => match ev {
                InputEvent::MouseMove { dx, dy } => {
                    (KIND_MOUSE_MOVE, false, 0, 0, i32::from(dx), i32::from(dy))
                }
                InputEvent::MouseAbs { x, y } => (KIND_MOUSE_ABS, false, 0, 0, x, y),
                InputEvent::Button { button, down } => {
                    (KIND_BUTTON, down, button_to_u8(button), 0, 0, 0)
                }
                InputEvent::Scroll { dx, dy, phase } => (
                    KIND_SCROLL,
                    false,
                    phase.as_u8(),
                    0,
                    i32::from(dx),
                    i32::from(dy),
                ),
                InputEvent::Key { scan, down } => (KIND_KEY, down, 0, scan, 0, 0),
                InputEvent::Media { key, down } => (KIND_MEDIA, down, media_to_u8(key), 0, 0, 0),
            },
            InjectFrame::TextUnit(u) => (KIND_TEXT_UNIT, false, 0, u, 0, 0),
        };
        let mut buf = [0u8; FRAME_LEN];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = VERSION;
        buf[3] = kind;
        buf[4] = u8::from(down);
        buf[5] = sub;
        buf[6..8].copy_from_slice(&scan.to_le_bytes());
        buf[8..12].copy_from_slice(&a.to_le_bytes());
        buf[12..16].copy_from_slice(&b.to_le_bytes());
        buf
    }

    pub fn decode(buf: &[u8; FRAME_LEN]) -> Result<Self, FrameError> {
        if buf[0..2] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        if buf[2] != VERSION {
            return Err(FrameError::BadVersion(buf[2]));
        }
        let down = match buf[4] {
            0 => false,
            1 => true,
            _ => return Err(FrameError::BadField),
        };
        let sub = buf[5];
        let scan = u16::from_le_bytes([buf[6], buf[7]]);
        let a = i32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let b = i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let small = |v: i32| i16::try_from(v).map_err(|_| FrameError::BadField);
        Ok(match buf[3] {
            KIND_MOUSE_MOVE => InjectFrame::Event(InputEvent::MouseMove {
                dx: small(a)?,
                dy: small(b)?,
            }),
            KIND_MOUSE_ABS => InjectFrame::Event(InputEvent::MouseAbs { x: a, y: b }),
            KIND_BUTTON => InjectFrame::Event(InputEvent::Button {
                button: button_from_u8(sub).ok_or(FrameError::BadField)?,
                down,
            }),
            KIND_SCROLL => InjectFrame::Event(InputEvent::Scroll {
                dx: small(a)?,
                dy: small(b)?,
                phase: ScrollPhase::from_u8(sub).ok_or(FrameError::BadField)?,
            }),
            KIND_KEY => InjectFrame::Event(InputEvent::Key { scan, down }),
            KIND_MEDIA => InjectFrame::Event(InputEvent::Media {
                key: media_from_u8(sub).ok_or(FrameError::BadField)?,
                down,
            }),
            KIND_TEXT_UNIT => InjectFrame::TextUnit(scan),
            k => return Err(FrameError::BadKind(k)),
        })
    }
}

/// Remonta frames de 32 bytes a partir de leituras de tamanho arbitrário (o pipe é em modo byte).
#[derive(Debug, Default)]
pub struct FrameAssembler {
    pending: Vec<u8>,
}

impl FrameAssembler {
    /// Acrescenta bytes e devolve os frames completos. O primeiro frame inválido encerra com erro
    /// (o servidor derruba o cliente: o fluxo perdeu o alinhamento ou não é o agente).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<InjectFrame>, FrameError> {
        self.pending.extend_from_slice(bytes);
        let mut frames = Vec::with_capacity(self.pending.len() / FRAME_LEN);
        let mut consumed = 0;
        let mut result = Ok(());
        for chunk in self.pending.chunks_exact(FRAME_LEN) {
            let arr: &[u8; FRAME_LEN] = chunk.try_into().expect("chunks_exact");
            match InjectFrame::decode(arr) {
                Ok(f) => frames.push(f),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
            consumed += FRAME_LEN;
        }
        self.pending.drain(..consumed);
        result.map(|()| frames)
    }
}

/// `true` se já passou tempo suficiente desde a última tentativa de conexão.
pub fn reconnect_due(since_last_attempt: Option<Duration>, interval: Duration) -> bool {
    since_last_attempt.is_none_or(|d| d >= interval)
}

#[derive(Debug, Default)]
struct ClientState {
    file: Option<File>,
    last_attempt: Option<Instant>,
}

impl ClientState {
    fn try_connect(&mut self, path: &str) -> bool {
        self.last_attempt = Some(Instant::now());
        self.file = open_pipe(path).ok();
        self.file.is_some()
    }

    fn connect_if_due(&mut self, path: &str) -> bool {
        if self.file.is_some() {
            return true;
        }
        let since = self.last_attempt.map(|t| t.elapsed());
        reconnect_due(since, RECONNECT_INTERVAL) && self.try_connect(path)
    }
}

/// Cliente do pipe usado pelo agente: com o serviço presente, toda injeção passa por aqui (D10).
#[derive(Debug)]
pub struct InjectPipeClient {
    path: String,
    state: Mutex<ClientState>,
}

impl Default for InjectPipeClient {
    fn default() -> Self {
        Self::with_path(PIPE_NAME)
    }
}

impl InjectPipeClient {
    pub fn with_path(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(ClientState::default()),
        }
    }

    fn state(&mut self) -> &mut ClientState {
        self.state.get_mut().unwrap_or_else(|e| e.into_inner())
    }

    /// Conecta agora (ignora o intervalo de reconexão). `true` se o helper aceitou.
    pub fn connect(&mut self) -> bool {
        let path = self.path.clone();
        self.state().try_connect(&path)
    }

    /// `true` se o helper do serviço está conectado. Se não estiver, tenta reconectar no máximo
    /// uma vez por `RECONNECT_INTERVAL` (o serviço pode ter subido depois do agente).
    pub fn is_available(&self) -> bool {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.connect_if_due(&self.path)
    }

    /// Envia um evento. Em falha de escrita (helper reiniciado) reconecta uma vez e reenvia.
    pub fn send(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.send_frame(InjectFrame::Event(*event))
    }

    fn send_frame(&mut self, frame: InjectFrame) -> Result<(), InjectError> {
        self.send_bytes(&frame.encode())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), InjectError> {
        let path = self.path.clone();
        let st = self.state();
        if !st.connect_if_due(&path) {
            return Err(unavailable());
        }
        if write_to(st, bytes).is_ok() {
            return Ok(());
        }
        st.file = None;
        if !st.try_connect(&path) {
            return Err(unavailable());
        }
        write_to(st, bytes).map_err(|e| {
            st.file = None;
            InjectError::Platform(format!("falha ao escrever no pipe: {e}"))
        })
    }
}

/// Abre o pipe só com FILE_WRITE_DATA|SYNCHRONIZE: é tudo que o ACL do servidor concede ao
/// usuário (sem FILE_CREATE_PIPE_INSTANCE, ninguém cria instância falsa do nome).
fn open_pipe(path: &str) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new().access_mode(0x0010_0002).open(path)
    }
    #[cfg(not(windows))]
    {
        OpenOptions::new().write(true).open(path)
    }
}

fn write_to(st: &mut ClientState, bytes: &[u8]) -> std::io::Result<()> {
    match st.file.as_mut() {
        Some(f) => f.write_all(bytes),
        None => Err(std::io::ErrorKind::NotConnected.into()),
    }
}

fn unavailable() -> InjectError {
    InjectError::Platform("helper do serviço indisponível".into())
}

impl InputInjector for InjectPipeClient {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        self.send(event)
    }

    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        // Um único write com todos os frames: o texto não se intercala com outros eventos.
        let mut bytes = Vec::with_capacity(text.len() * FRAME_LEN);
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&InjectFrame::TextUnit(unit).encode());
        }
        if bytes.is_empty() {
            return Ok(());
        }
        self.send_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_events() -> Vec<InputEvent> {
        vec![
            InputEvent::MouseMove { dx: -5, dy: 32767 },
            InputEvent::MouseAbs { x: -1920, y: 1079 },
            InputEvent::Button {
                button: MouseButton::X2,
                down: true,
            },
            InputEvent::Button {
                button: MouseButton::Left,
                down: false,
            },
            InputEvent::Scroll {
                dx: -120,
                dy: 240,
                phase: ScrollPhase::Momentum,
            },
            InputEvent::Key {
                scan: 0xE05B,
                down: true,
            },
            InputEvent::Media {
                key: MediaKey::Previous,
                down: false,
            },
        ]
    }

    #[test]
    fn roundtrip_all_events() {
        for ev in all_events() {
            let buf = InjectFrame::Event(ev).encode();
            assert_eq!(buf.len(), 32);
            assert_eq!(InjectFrame::decode(&buf), Ok(InjectFrame::Event(ev)));
        }
        let buf = InjectFrame::TextUnit(0xD83D).encode();
        assert_eq!(InjectFrame::decode(&buf), Ok(InjectFrame::TextUnit(0xD83D)));
    }

    #[test]
    fn decode_rejects_garbage() {
        let good = InjectFrame::Event(InputEvent::Key {
            scan: 1,
            down: true,
        })
        .encode();
        let mut b = good;
        b[0] = 0;
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadMagic));
        let mut b = good;
        b[2] = 9;
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadVersion(9)));
        let mut b = good;
        b[3] = 99;
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadKind(99)));
        let mut b = good;
        b[4] = 2;
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadField));
        let mut b = InjectFrame::Event(InputEvent::Button {
            button: MouseButton::Left,
            down: true,
        })
        .encode();
        b[5] = 77;
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadField));
        let mut b = InjectFrame::Event(InputEvent::MouseMove { dx: 0, dy: 0 }).encode();
        b[8..12].copy_from_slice(&100_000i32.to_le_bytes());
        assert_eq!(InjectFrame::decode(&b), Err(FrameError::BadField));
    }

    #[test]
    fn assembler_handles_split_and_batched_reads() {
        let evs = all_events();
        let mut stream = Vec::new();
        for ev in &evs {
            stream.extend_from_slice(&InjectFrame::Event(*ev).encode());
        }
        let mut asm = FrameAssembler::default();
        let mut got = Vec::new();
        for chunk in stream.chunks(45) {
            got.extend(asm.push(chunk).unwrap());
        }
        let want: Vec<_> = evs.into_iter().map(InjectFrame::Event).collect();
        assert_eq!(got, want);
        assert!(asm.pending.is_empty());
    }

    #[test]
    fn assembler_errors_on_bad_frame_and_keeps_good_prefix_out() {
        let mut asm = FrameAssembler::default();
        let mut bytes = InjectFrame::Event(InputEvent::Key {
            scan: 1,
            down: true,
        })
        .encode()
        .to_vec();
        bytes.extend_from_slice(&[0xFF; FRAME_LEN]);
        assert_eq!(asm.push(&bytes), Err(FrameError::BadMagic));
    }

    #[test]
    fn reconnect_throttle() {
        let i = Duration::from_secs(1);
        assert!(reconnect_due(None, i));
        assert!(!reconnect_due(Some(Duration::from_millis(999)), i));
        assert!(reconnect_due(Some(i), i));
    }

    #[test]
    fn client_without_server_is_unavailable() {
        let mut c = InjectPipeClient::with_path("/nonexistent/relay-inject-test");
        assert!(!c.is_available());
        assert!(c.inject(&InputEvent::MouseMove { dx: 1, dy: 1 }).is_err());
        assert!(c.type_text("oi").is_err());
        assert!(c.type_text("").is_ok());
    }

    #[test]
    fn client_sends_frames_to_a_file_sink() {
        let dir = std::env::temp_dir().join(format!("relay-pipe-test-{}", std::process::id()));
        std::fs::write(&dir, b"").unwrap();
        let mut c = InjectPipeClient::with_path(dir.to_string_lossy().to_string());
        assert!(c.connect());
        assert!(c.is_available());
        c.inject(&InputEvent::Key {
            scan: 0x1E,
            down: true,
        })
        .unwrap();
        c.type_text("a😀").unwrap();
        let data = std::fs::read(&dir).unwrap();
        std::fs::remove_file(&dir).ok();
        let mut asm = FrameAssembler::default();
        let frames = asm.push(&data).unwrap();
        assert_eq!(frames.len(), 1 + 3);
        assert_eq!(frames[1], InjectFrame::TextUnit(u16::from(b'a')));
    }
}
