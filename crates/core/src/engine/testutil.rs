//! dono: C10
//! Mocks de plataforma e fixtures compartilhados pelos testes do motor e dos apps (os apps ligam
//! a feature `test-util` como dev-dependency).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::path::PathBuf;

use crate::platform::{
    CaptureError, ChangeCallback, ClipboardBackend, CursorControl, FileProvider, InjectError,
    InputCapture, InputInjector, KeyRepeat, PlatformError,
};
use crate::types::{
    CapturedEvent, ClipboardContent, DisplayInfo, FileEntry, InputEvent, LayoutProfile, Machine,
    PlacedDisplay, ProfileKey, RectMm,
};

pub const MAC: &str = "mac";
pub const DELL: &str = "dell";

/// Registro compartilhado das chamadas de plataforma, em ordem.
pub type Log = Arc<Mutex<Vec<String>>>;

pub fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn calls(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

pub struct MockCapture {
    pub log: Log,
    pub swallow: Arc<AtomicBool>,
}

impl MockCapture {
    pub fn new(log: &Log) -> Self {
        Self {
            log: log.clone(),
            swallow: Arc::default(),
        }
    }
}

impl InputCapture for MockCapture {
    fn start(&mut self, _sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn set_swallow(&self, swallow: bool) {
        self.swallow.store(swallow, Ordering::SeqCst);
        self.log.lock().unwrap().push(format!("swallow({swallow})"));
    }
    fn is_swallowing(&self) -> bool {
        self.swallow.load(Ordering::SeqCst)
    }
}

pub struct MockCursor {
    pub log: Log,
    pub pos: Pos,
}

impl MockCursor {
    pub fn new(log: &Log) -> Self {
        Self {
            log: log.clone(),
            pos: Arc::default(),
        }
    }
    /// Cursor que compartilha a posição `pos` (o injetor também a desloca) e não registra log.
    pub fn at(pos: Pos) -> Self {
        Self {
            log: new_log(),
            pos,
        }
    }
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
}

impl CursorControl for MockCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        self.note("hide".into());
        Ok(())
    }
    fn show(&mut self) -> Result<(), PlatformError> {
        self.note("show".into());
        Ok(())
    }
    fn associate(&mut self, associated: bool) -> Result<(), PlatformError> {
        self.note(format!("associate({associated})"));
        Ok(())
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        *self.pos.lock().unwrap() = (x, y);
        self.note(format!("warp({x},{y})"));
        Ok(())
    }
    fn position(&self) -> Result<(i32, i32), PlatformError> {
        Ok(*self.pos.lock().unwrap())
    }
}

pub type Pos = Arc<Mutex<(i32, i32)>>;
type CursorBounds = (Pos, (i32, i32, i32, i32));

/// Injetor que guarda os eventos. `MouseMove` também desloca a posição do cursor mock (como o
/// Windows faria), limitada a `bounds`.
pub struct MockInjector {
    pub events: Arc<Mutex<Vec<InputEvent>>>,
    pub typed: Arc<Mutex<Vec<String>>>,
    pub repeat: KeyRepeat,
    pub cursor: Option<CursorBounds>,
}

impl MockInjector {
    pub fn new() -> Self {
        Self {
            events: Arc::default(),
            typed: Arc::default(),
            repeat: KeyRepeat {
                delay_ms: 500,
                rate_hz: 30.0,
            },
            cursor: None,
        }
    }

    /// Injetor que grava em `events` (compartilhado com o teste).
    pub fn recording(events: Arc<Mutex<Vec<InputEvent>>>) -> Self {
        Self {
            events,
            ..Self::new()
        }
    }

    /// `MouseMove` também desloca `pos`, limitada a `bounds` = (x0, y0, x1, y1) exclusivo.
    pub fn moving(mut self, pos: Pos, bounds: (i32, i32, i32, i32)) -> Self {
        self.cursor = Some((pos, bounds));
        self
    }
}

impl Default for MockInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl InputInjector for MockInjector {
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        if let (InputEvent::MouseMove { dx, dy }, Some((pos, b))) = (event, &self.cursor) {
            let mut p = pos.lock().unwrap();
            p.0 = (p.0 + i32::from(*dx)).clamp(b.0, b.2 - 1);
            p.1 = (p.1 + i32::from(*dy)).clamp(b.1, b.3 - 1);
        }
        self.events.lock().unwrap().push(*event);
        Ok(())
    }
    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        self.typed.lock().unwrap().push(text.to_owned());
        Ok(())
    }
    fn key_repeat(&self) -> KeyRepeat {
        self.repeat
    }
}

pub fn display(
    id: &str,
    machine: Machine,
    size: (u32, u32),
    mm: (f32, f32),
    builtin: bool,
) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: id.into(),
        machine,
        origin_px: (0, 0),
        size_px: size,
        scale: 1.0,
        size_mm: Some(mm),
        is_primary: builtin,
        is_builtin: builtin,
    }
}

pub fn mac_display() -> DisplayInfo {
    display(MAC, Machine::Mac, (1440, 900), (300.0, 190.0), true)
}

pub fn dell_display() -> DisplayInfo {
    display(DELL, Machine::Windows, (2560, 1440), (597.0, 336.0), false)
}

/// Mac à esquerda, Dell à direita (mais alto: o segmento compartilhado é a altura do Mac).
pub fn profile(sens: f32) -> LayoutProfile {
    let placed = |id: &str, machine, x, y, w, h, sensitivity| PlacedDisplay {
        display_id: id.into(),
        machine,
        rect_mm: RectMm { x, y, w, h },
        sensitivity,
        rotation: None,
    };
    LayoutProfile {
        key: ProfileKey::default(),
        name: "t".into(),
        displays: vec![
            placed(MAC, Machine::Mac, 0.0, 0.0, 300.0, 190.0, 1.0),
            placed(DELL, Machine::Windows, 300.0, -100.0, 597.0, 336.0, sens),
        ],
        windows_detached: false,
    }
}

type Provider = Arc<dyn Fn(&[FileEntry]) -> Result<Vec<PathBuf>, String> + Send + Sync>;

#[derive(Default)]
pub struct ClipState {
    pub content: Option<ClipboardContent>,
    pub paths: Vec<PathBuf>,
    pub written: Vec<ClipboardContent>,
    pub callback: Option<Arc<dyn Fn() + Send + Sync>>,
    pub provider: Option<Provider>,
}

/// Clipboard em memória: o teste copia pelo `copy` (dispara o callback) e lê o que o Relay
/// escreveu em `written`.
#[derive(Clone, Default)]
pub struct MockClip(pub Arc<Mutex<ClipState>>);

impl MockClip {
    /// Cópia local do usuário: guarda e dispara o callback.
    pub fn copy(&self, c: ClipboardContent, paths: Vec<PathBuf>) {
        let cb = {
            let mut s = self.0.lock().unwrap();
            s.content = Some(c);
            s.paths = paths;
            s.callback.clone()
        };
        if let Some(cb) = cb {
            cb();
        }
    }

    pub fn written(&self) -> Vec<ClipboardContent> {
        self.0.lock().unwrap().written.clone()
    }
}

impl ClipboardBackend for MockClip {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        Ok(self.0.lock().unwrap().content.clone())
    }
    fn write_remote(&mut self, content: ClipboardContent) -> Result<(), PlatformError> {
        let mut s = self.0.lock().unwrap();
        s.content = Some(content.clone());
        s.written.push(content);
        Ok(())
    }
    fn set_change_callback(&mut self, callback: ChangeCallback) {
        self.0.lock().unwrap().callback = Some(Arc::from(callback));
    }
    fn local_paths(&self) -> Vec<PathBuf> {
        self.0.lock().unwrap().paths.clone()
    }
    fn set_file_provider(&mut self, provider: FileProvider) {
        self.0.lock().unwrap().provider = Some(Arc::from(provider));
    }
}
