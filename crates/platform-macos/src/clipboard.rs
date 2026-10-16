//! dono: M5
//! Clipboard do macOS (NSPasteboard): texto, HTML, RTF, PNG e URLs de arquivo, detecção de
//! mudança por `changeCount` (polling de 250 ms) e marcador anti-eco.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};
use relay_core::platform::{ChangeCallback, ClipboardBackend, PlatformError};
use relay_core::types::{ClipboardContent, FileEntry};

/// Tipo extra gravado em toda escrita vinda do remoto; o callback ignora quem o tem.
pub const REMOTE_MARKER_TYPE: &str = "com.joaocamargo.relay.remote";
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

const T_TEXT: &str = "public.utf8-plain-text";
const T_HTML: &str = "public.html";
const T_RTF: &str = "public.rtf";
const T_PNG: &str = "public.png";
const T_TIFF: &str = "public.tiff";

pub struct MacClipboard {
    /// `None` = pasteboard geral; `Some(nome)` = pasteboard nomeado (testes).
    name: Option<String>,
    pasteboard: Pb,
    paths: Vec<PathBuf>,
    stop: Arc<AtomicBool>,
    poller: Option<JoinHandle<()>>,
}

/// `NSPasteboard` não é `Send` no objc2, mas o AppKit documenta o pasteboard como seguro entre
/// threads; cada thread usa a sua própria referência obtida pelo nome.
struct Pb(Retained<NSPasteboard>);
// SAFETY: NSPasteboard é thread-safe (documentação da Apple); só chamamos métodos de instância.
unsafe impl Send for Pb {}

fn open(name: &Option<String>) -> Pb {
    Pb(match name {
        None => NSPasteboard::generalPasteboard(),
        Some(n) => NSPasteboard::pasteboardWithName(&NSString::from_str(n)),
    })
}

impl Default for MacClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MacClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacClipboard")
            .field("name", &self.name)
            .finish()
    }
}

impl MacClipboard {
    /// Sobre o pasteboard geral do sistema.
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Sobre um pasteboard nomeado (privado); usado pelos testes para não tocar no geral.
    pub fn with_pasteboard_name(name: &str) -> Self {
        Self::build(Some(name.to_owned()))
    }

    fn build(name: Option<String>) -> Self {
        Self {
            pasteboard: open(&name),
            name,
            paths: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            poller: None,
        }
    }

    /// Escreve URLs de arquivo marcadas como remotas.
    pub fn write_remote_files(&mut self, paths: &[PathBuf]) -> Result<(), PlatformError> {
        let urls: Vec<Retained<NSURL>> = paths
            .iter()
            .filter_map(|p| p.to_str())
            .map(|p| NSURL::fileURLWithPath(&NSString::from_str(p)))
            .collect();
        if urls.is_empty() {
            return Err(PlatformError::Platform("lista de arquivos vazia".into()));
        }
        let pb = &self.pasteboard.0;
        pb.clearContents();
        let objects: Vec<_> = urls
            .iter()
            .map(|u| objc2::runtime::ProtocolObject::from_ref(&**u))
            .collect();
        if !pb.writeObjects(&NSArray::from_slice(&objects)) {
            return Err(PlatformError::Platform(
                "falha ao escrever URLs de arquivo".into(),
            ));
        }
        mark_remote(pb);
        Ok(())
    }
}

fn mark_remote(pb: &NSPasteboard) {
    let marker = NSString::from_str(REMOTE_MARKER_TYPE);
    pb.setData_forType(Some(&NSData::from_vec(vec![1])), &marker);
}

fn has_marker(pb: &NSPasteboard) -> bool {
    let marker = NSString::from_str(REMOTE_MARKER_TYPE);
    pb.types().is_some_and(|t| t.iter().any(|x| *x == *marker))
}

fn data_of(pb: &NSPasteboard, kind: &str) -> Option<Vec<u8>> {
    pb.dataForType(&NSString::from_str(kind))
        .map(|d| d.to_vec())
}

fn string_of(pb: &NSPasteboard, kind: &str) -> Option<String> {
    pb.stringForType(&NSString::from_str(kind))
        .map(|s| s.to_string())
}

fn tiff_to_png(tiff: &[u8]) -> Option<Vec<u8>> {
    let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &NSData::with_bytes(tiff))?;
    let props = NSDictionary::new();
    // SAFETY: dicionário de propriedades vazio é válido para o tipo PNG.
    let png =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
    Some(png.to_vec())
}

fn entry_for(id: usize, path: &std::path::Path) -> FileEntry {
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    let size = if is_dir {
        dir_size(path)
    } else {
        meta.map_or(0, |m| m.len())
    };
    FileEntry {
        id: id as u32,
        name: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        size,
        is_dir,
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

impl ClipboardBackend for MacClipboard {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        let pb = &self.pasteboard.0;
        let files = crate::files::file_paths_from(pb);
        if !files.is_empty() {
            let entries = files
                .iter()
                .enumerate()
                .map(|(i, p)| entry_for(i, p))
                .collect();
            self.paths = files;
            return Ok(Some(ClipboardContent::FileList(entries)));
        }
        self.paths.clear();
        if let Some(png) = data_of(pb, T_PNG) {
            return Ok(Some(ClipboardContent::Png(png)));
        }
        if let Some(png) = data_of(pb, T_TIFF).and_then(|t| tiff_to_png(&t)) {
            return Ok(Some(ClipboardContent::Png(png)));
        }
        if let Some(html) = string_of(pb, T_HTML) {
            return Ok(Some(ClipboardContent::Html(html)));
        }
        if let Some(rtf) = data_of(pb, T_RTF) {
            return Ok(Some(ClipboardContent::Rtf(rtf)));
        }
        Ok(string_of(pb, T_TEXT).map(ClipboardContent::Text))
    }

    fn write_remote(&mut self, content: ClipboardContent) -> Result<(), PlatformError> {
        match content {
            // Arquivos do PC viram "pendentes" no app, nunca vão ao pasteboard.
            ClipboardContent::FileList(_) => return Err(PlatformError::Unsupported),
            other => {
                let pb = &self.pasteboard.0;
                pb.clearContents();
                let ok = match other {
                    ClipboardContent::Text(s) => {
                        pb.setString_forType(&NSString::from_str(&s), &NSString::from_str(T_TEXT))
                    }
                    ClipboardContent::Html(s) => {
                        pb.setString_forType(&NSString::from_str(&s), &NSString::from_str(T_HTML))
                    }
                    ClipboardContent::Rtf(b) => {
                        pb.setData_forType(Some(&NSData::from_vec(b)), &NSString::from_str(T_RTF))
                    }
                    ClipboardContent::Png(b) => {
                        pb.setData_forType(Some(&NSData::from_vec(b)), &NSString::from_str(T_PNG))
                    }
                    ClipboardContent::FileList(_) => unreachable!(),
                };
                if !ok {
                    return Err(PlatformError::Platform(
                        "falha ao escrever no pasteboard".into(),
                    ));
                }
                mark_remote(pb);
            }
        }
        Ok(())
    }

    fn set_change_callback(&mut self, callback: ChangeCallback) {
        self.stop_poller();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = stop.clone();
        let name = self.name.clone();
        self.poller = Some(thread::spawn(move || {
            let pb = open(&name);
            let mut last = pb.0.changeCount();
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(POLL_INTERVAL);
                let now = pb.0.changeCount();
                if now != last {
                    last = now;
                    if !has_marker(&pb.0) {
                        callback();
                    }
                }
            }
        }));
    }

    fn local_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }
}

impl MacClipboard {
    fn stop_poller(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.poller.take() {
            let _ = h.join();
        }
    }
}

impl Drop for MacClipboard {
    fn drop(&mut self) {
        self.stop_poller();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn clip(tag: &str) -> MacClipboard {
        MacClipboard::with_pasteboard_name(&format!(
            "com.joaocamargo.relay.test.{tag}.{}",
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_texto() {
        let mut c = clip("txt");
        c.write_remote(ClipboardContent::Text("olá relay".into()))
            .unwrap();
        assert_eq!(
            c.read().unwrap(),
            Some(ClipboardContent::Text("olá relay".into()))
        );
    }

    #[test]
    fn roundtrip_html() {
        let mut c = clip("html");
        c.write_remote(ClipboardContent::Html("<b>oi</b>".into()))
            .unwrap();
        assert_eq!(
            c.read().unwrap(),
            Some(ClipboardContent::Html("<b>oi</b>".into()))
        );
    }

    #[test]
    fn roundtrip_rtf() {
        let mut c = clip("rtf");
        let rtf = br"{\rtf1\ansi oi}".to_vec();
        c.write_remote(ClipboardContent::Rtf(rtf.clone())).unwrap();
        assert_eq!(c.read().unwrap(), Some(ClipboardContent::Rtf(rtf)));
    }

    #[test]
    fn roundtrip_png() {
        let mut c = clip("png");
        let png = tiny_png();
        c.write_remote(ClipboardContent::Png(png.clone())).unwrap();
        assert_eq!(c.read().unwrap(), Some(ClipboardContent::Png(png)));
    }

    #[test]
    fn tiff_vira_png() {
        let c = clip("tiff");
        let rep_png = tiny_png();
        // converte PNG -> TIFF via NSBitmapImageRep e volta
        let rep = NSBitmapImageRep::initWithData(
            NSBitmapImageRep::alloc(),
            &NSData::with_bytes(&rep_png),
        )
        .unwrap();
        let tiff = rep.TIFFRepresentation().unwrap().to_vec();
        c.pasteboard.0.clearContents();
        c.pasteboard.0.setData_forType(
            Some(&NSData::with_bytes(&tiff)),
            &NSString::from_str(T_TIFF),
        );
        let mut c = c;
        match c.read().unwrap() {
            Some(ClipboardContent::Png(p)) => assert_eq!(&p[1..4], b"PNG"),
            other => panic!("esperava PNG, veio {other:?}"),
        }
    }

    #[test]
    fn roundtrip_url_de_arquivo() {
        let dir = std::env::temp_dir().join(format!("relay-m5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"12345").unwrap();
        let file = file.canonicalize().unwrap();
        let mut c = clip("files");
        c.write_remote_files(std::slice::from_ref(&file)).unwrap();
        match c.read().unwrap() {
            Some(ClipboardContent::FileList(v)) => {
                assert_eq!(v.len(), 1);
                assert_eq!(
                    (v[0].id, v[0].name.as_str(), v[0].size, v[0].is_dir),
                    (0, "a.txt", 5, false)
                );
            }
            other => panic!("esperava FileList, veio {other:?}"),
        }
        assert_eq!(c.local_paths(), vec![file]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn marcador_gravado_e_ignorado_pelo_callback() {
        let mut c = clip("echo");
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        c.set_change_callback(Box::new(move || {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        thread::sleep(POLL_INTERVAL * 2);
        // escrita remota: marcada, não dispara
        c.write_remote(ClipboardContent::Text("remoto".into()))
            .unwrap();
        assert!(has_marker(&c.pasteboard.0));
        thread::sleep(POLL_INTERVAL * 3);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // escrita local (sem marcador): dispara
        c.pasteboard.0.clearContents();
        c.pasteboard
            .0
            .setString_forType(&NSString::from_str("local"), &NSString::from_str(T_TEXT));
        thread::sleep(POLL_INTERVAL * 3);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "mexe no pasteboard geral do usuário; rodar manualmente"]
    fn geral_le_sem_panico() {
        let mut c = MacClipboard::new();
        let _ = c.read();
    }

    fn tiny_png() -> Vec<u8> {
        // PNG 1x1 válido.
        const B64: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99,
            0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        B64.to_vec()
    }
}
