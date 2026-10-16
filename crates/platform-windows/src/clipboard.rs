//! dono: W5
//! Clipboard do Windows: janela oculta com `AddClipboardFormatListener`, formatos de texto/HTML/
//! RTF/PNG/DIB/CF_HDROP, marcador anti-eco `RelayRemote` e renderização atrasada de arquivos (D20).
//! A lógica de formato está em `clipboard_formats` (pura); aqui fica o estado e o Win32.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use relay_core::platform::{ChangeCallback, ClipboardBackend, FileProvider, PlatformError};
use relay_core::types::{ClipboardContent, FileEntry};

type SharedCallback = Arc<dyn Fn() + Send + Sync>;
type SharedProvider = Arc<dyn Fn(&[FileEntry]) -> Result<Vec<PathBuf>, String> + Send + Sync>;

/// Estado compartilhado entre a API pública e a thread da janela (lido só no Windows).
#[derive(Default)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Shared {
    callback: Mutex<Option<SharedCallback>>,
    provider: Mutex<Option<SharedProvider>>,
    /// Entradas da `FileList` remota que o CF_HDROP pendente (renderização atrasada) vai pedir.
    pending: Mutex<Vec<FileEntry>>,
    /// Caminhos da última `FileList` lida (índice = `FileEntry.id`).
    local_paths: Mutex<Vec<PathBuf>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
pub struct WindowsClipboard {
    shared: Arc<Shared>,
    #[cfg(windows)]
    window: Option<win::ClipboardWindow>,
}

impl std::fmt::Debug for WindowsClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowsClipboard")
    }
}

impl ClipboardBackend for WindowsClipboard {
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError> {
        #[cfg(windows)]
        {
            self.ensure_window()?;
            win::read(&self.shared)
        }
        #[cfg(not(windows))]
        Err(PlatformError::NotImplemented)
    }

    fn write_remote(&mut self, content: ClipboardContent) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            let hwnd = self.ensure_window()?;
            win::write_remote(&self.shared, hwnd, content)
        }
        #[cfg(not(windows))]
        {
            let _ = content;
            Err(PlatformError::NotImplemented)
        }
    }

    fn set_change_callback(&mut self, callback: ChangeCallback) {
        *lock(&self.shared.callback) = Some(Arc::from(callback));
        #[cfg(windows)]
        {
            // Sem janela não há listener: sobe agora para o callback começar a disparar.
            let _ = self.ensure_window();
        }
    }

    fn local_paths(&self) -> Vec<PathBuf> {
        lock(&self.shared.local_paths).clone()
    }

    fn set_file_provider(&mut self, provider: FileProvider) {
        *lock(&self.shared.provider) = Some(Arc::from(provider));
    }
}

#[cfg(windows)]
impl WindowsClipboard {
    fn ensure_window(&mut self) -> Result<isize, PlatformError> {
        if self.window.is_none() {
            self.window = Some(win::ClipboardWindow::spawn(self.shared.clone())?);
        }
        Ok(self.window.as_ref().map_or(0, |w| w.hwnd))
    }
}

#[cfg(windows)]
mod win {
    use std::sync::{Arc, OnceLock, mpsc};
    use std::time::Duration;

    use relay_core::platform::PlatformError;
    use relay_core::types::{ClipboardContent, ClipboardFormat, FileEntry};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
        RemoveClipboardFormatListener, SetClipboardData,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
        GetMessageW, GetWindowLongPtrW, HWND_MESSAGE, MSG, PostMessageW, PostQuitMessage,
        RegisterClassW, SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_CLIPBOARDUPDATE, WM_CLOSE, WM_DESTROY, WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS,
        WM_RENDERFORMAT, WNDCLASSW,
    };
    use windows::core::{PCWSTR, w};

    use super::{Shared, lock};
    use crate::clipboard_formats::{
        HTML_FORMAT, PNG_FORMAT, RELAY_REMOTE_FORMAT, RTF_FORMAT, build_cf_html, build_dropfiles,
        dib_to_png, parse_cf_html, parse_dropfiles, pick_format, png_to_dibv5,
        should_notify_change,
    };

    const CF_UNICODETEXT: u32 = 13;
    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const CF_DIBV5: u32 = 17;

    fn err(e: impl std::fmt::Display) -> PlatformError {
        PlatformError::Platform(e.to_string())
    }

    /// Ids dos formatos registrados (uma vez por processo).
    struct Formats {
        html: u32,
        rtf: u32,
        png: u32,
        remote: u32,
    }

    fn formats() -> &'static Formats {
        static F: OnceLock<Formats> = OnceLock::new();
        F.get_or_init(|| {
            let reg = |name: &str| {
                let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
                // SAFETY: `wide` é NUL-terminado e vive durante a chamada.
                unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
            };
            Formats {
                html: reg(HTML_FORMAT),
                rtf: reg(RTF_FORMAT),
                png: reg(PNG_FORMAT),
                remote: reg(RELAY_REMOTE_FORMAT),
            }
        })
    }

    /// Janela oculta (somente mensagens) com seu laço de mensagens numa thread própria.
    pub struct ClipboardWindow {
        pub hwnd: isize,
    }

    impl ClipboardWindow {
        pub fn spawn(shared: Arc<Shared>) -> Result<Self, PlatformError> {
            let (tx, rx) = mpsc::channel::<Result<isize, String>>();
            std::thread::Builder::new()
                .name("relay-clipboard".into())
                .spawn(move || run_window(shared, tx))
                .map_err(err)?;
            let hwnd = rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(err)?
                .map_err(PlatformError::Platform)?;
            Ok(Self { hwnd })
        }
    }

    impl Drop for ClipboardWindow {
        fn drop(&mut self) {
            // SAFETY: WM_CLOSE é postado para a janela da nossa thread; ela se destrói sozinha.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    fn run_window(shared: Arc<Shared>, tx: mpsc::Sender<Result<isize, String>>) {
        // SAFETY: tudo abaixo roda na thread dona da janela; o ponteiro de `Arc<Shared>` guardado em
        // GWLP_USERDATA é recuperado (e liberado) quando a janela é destruída, nesta mesma thread.
        unsafe {
            let class = w!("RelayClipboardWindow");
            let hinstance = GetModuleHandleW(None).map(Into::into).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc); // 0 se a classe já existe: segue.
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Relay clipboard"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let raw = Arc::into_raw(shared);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
            let _ = formats();
            if let Err(e) = AddClipboardFormatListener(hwnd) {
                let _ = tx.send(Err(e.to_string()));
                let _ = DestroyWindow(hwnd);
                return;
            }
            let _ = tx.send(Ok(hwnd.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// # Safety
    /// Só o sistema chama (janela criada por `run_window`).
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: ver a doc da função; todas as chamadas são Win32 síncronas na thread da janela.
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared;
            if ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            // SAFETY: `ptr` veio de `Arc::into_raw` e só é liberado em WM_DESTROY, abaixo.
            let shared = &*ptr;
            match msg {
                WM_CLIPBOARDUPDATE => {
                    if !is_remote_marked() {
                        let cb = lock(&shared.callback).clone();
                        if let Some(cb) = cb {
                            cb();
                        }
                    }
                    LRESULT(0)
                }
                WM_RENDERFORMAT => {
                    if wparam.0 as u32 == CF_HDROP {
                        let entries = lock(&shared.pending).clone();
                        render_hdrop(shared, &entries);
                    }
                    LRESULT(0)
                }
                WM_RENDERALLFORMATS => {
                    // O dono vai sair (logoff/encerramento): não baixa arquivo nenhum, o que
                    // travaria a saída. O CF_HDROP atrasado é abandonado.
                    lock(&shared.pending).clear();
                    LRESULT(0)
                }
                WM_DESTROYCLIPBOARD => {
                    lock(&shared.pending).clear();
                    LRESULT(0)
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    let _ = RemoveClipboardFormatListener(hwnd);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    // SAFETY: devolve a referência tomada em `run_window`.
                    drop(Arc::from_raw(ptr));
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }

    fn is_remote_marked() -> bool {
        let marker = formats().remote;
        // SAFETY: consulta sem abrir o clipboard.
        let present: Vec<u32> = if unsafe { IsClipboardFormatAvailable(marker) }.is_ok() {
            vec![marker]
        } else {
            Vec::new()
        };
        !should_notify_change(&present, marker)
    }

    /// Chamada dentro de WM_RENDERFORMAT/WM_RENDERALLFORMATS (clipboard já aberto).
    fn render_hdrop(shared: &Shared, entries: &[FileEntry]) {
        let provider = lock(&shared.provider).clone();
        let Some(provider) = provider else { return };
        // Bloqueia até o download terminar; sem caminhos, não grava (a colagem falha).
        match provider(entries) {
            Ok(paths) if !paths.is_empty() => {
                let bytes = build_dropfiles(&paths);
                let _ = set_bytes(CF_HDROP, &bytes);
            }
            _ => {}
        }
    }

    fn open_clipboard(owner: isize) -> Result<(), PlatformError> {
        let mut last = None;
        for _ in 0..20 {
            // SAFETY: HWND de uma janela nossa (ou 0 = tarefa atual).
            let hwnd = (owner != 0).then_some(HWND(owner as *mut _));
            match unsafe { OpenClipboard(hwnd) } {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        Err(err(last.map(|e| e.to_string()).unwrap_or_default()))
    }

    /// Guarda que fecha o clipboard ao sair do escopo.
    struct Open;
    impl Open {
        fn new(owner: isize) -> Result<Self, PlatformError> {
            open_clipboard(owner).map(|()| Open)
        }
    }
    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: só existe com o clipboard aberto por esta thread.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// Grava `data` como HGLOBAL no formato `format` (clipboard aberto).
    fn set_bytes(format: u32, data: &[u8]) -> Result<(), PlatformError> {
        // SAFETY: bloco alocado com o tamanho exato; após SetClipboardData bem-sucedido o sistema
        // é o dono do HGLOBAL, senão liberamos aqui.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)).map_err(err)?;
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return Err(err("GlobalLock falhou"));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            let _ = GlobalUnlock(h);
            if let Err(e) = SetClipboardData(format, Some(HANDLE(h.0))) {
                let _ = GlobalFree(Some(h));
                return Err(err(e));
            }
        }
        Ok(())
    }

    /// Copia os bytes do formato `format` (clipboard aberto).
    fn get_bytes(format: u32) -> Option<Vec<u8>> {
        // SAFETY: o handle pertence ao clipboard; só lemos `GlobalSize` bytes sob GlobalLock.
        unsafe {
            let h = GetClipboardData(format).ok()?;
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            let p = GlobalLock(hg) as *const u8;
            if p.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(p, size).to_vec();
            let _ = GlobalUnlock(hg);
            Some(v)
        }
    }

    fn available(format: u32) -> bool {
        // SAFETY: consulta sem abrir o clipboard.
        unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    fn utf16_to_string(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    fn trim_nul(mut v: Vec<u8>) -> Vec<u8> {
        while v.last() == Some(&0) {
            v.pop();
        }
        v
    }

    pub fn read(shared: &Shared) -> Result<Option<ClipboardContent>, PlatformError> {
        let f = formats();
        let mut avail = Vec::new();
        if available(CF_HDROP) {
            avail.push(ClipboardFormat::FileList);
        }
        if available(CF_UNICODETEXT) {
            avail.push(ClipboardFormat::Text);
        }
        if available(f.html) {
            avail.push(ClipboardFormat::Html);
        }
        if available(f.rtf) {
            avail.push(ClipboardFormat::Rtf);
        }
        if available(f.png) || available(CF_DIBV5) || available(CF_DIB) {
            avail.push(ClipboardFormat::Png);
        }
        let Some(pick) = pick_format(&avail) else {
            return Ok(None);
        };
        let _open = Open::new(0)?;
        let content = match pick {
            ClipboardFormat::Text => {
                get_bytes(CF_UNICODETEXT).map(|b| ClipboardContent::Text(utf16_to_string(&b)))
            }
            ClipboardFormat::Html => get_bytes(f.html)
                .and_then(|b| parse_cf_html(&b))
                .map(ClipboardContent::Html),
            ClipboardFormat::Rtf => get_bytes(f.rtf).map(|b| ClipboardContent::Rtf(trim_nul(b))),
            ClipboardFormat::Png => get_bytes(f.png)
                .or_else(|| {
                    get_bytes(CF_DIBV5)
                        .or_else(|| get_bytes(CF_DIB))
                        .and_then(|d| dib_to_png(&d).ok())
                })
                .map(ClipboardContent::Png),
            ClipboardFormat::FileList => get_bytes(CF_HDROP).map(|b| {
                let paths = parse_dropfiles(&b);
                let entries = paths
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let meta = std::fs::metadata(p).ok();
                        FileEntry {
                            id: i as u32,
                            name: p
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
                            size: meta.as_ref().filter(|m| m.is_file()).map_or(0, |m| m.len()),
                            is_dir: meta.is_some_and(|m| m.is_dir()),
                        }
                    })
                    .collect();
                *lock(&shared.local_paths) = paths;
                ClipboardContent::FileList(entries)
            }),
        };
        Ok(content)
    }

    pub fn write_remote(
        shared: &Shared,
        owner: isize,
        content: ClipboardContent,
    ) -> Result<(), PlatformError> {
        let f = formats();
        let _open = Open::new(owner)?;
        // SAFETY: clipboard aberto por `_open` nesta thread.
        unsafe { EmptyClipboard() }.map_err(err)?;
        match content {
            ClipboardContent::Text(t) => {
                let mut b: Vec<u8> = t.encode_utf16().flat_map(u16::to_le_bytes).collect();
                b.extend_from_slice(&[0, 0]);
                set_bytes(CF_UNICODETEXT, &b)?;
            }
            ClipboardContent::Html(h) => {
                let mut b = build_cf_html(&h).into_bytes();
                b.push(0);
                set_bytes(f.html, &b)?;
            }
            ClipboardContent::Rtf(mut r) => {
                r.push(0);
                set_bytes(f.rtf, &r)?;
            }
            ClipboardContent::Png(p) => {
                set_bytes(f.png, &p)?;
                if let Ok(dib) = png_to_dibv5(&p) {
                    set_bytes(CF_DIBV5, &dib)?;
                }
            }
            ClipboardContent::FileList(entries) => {
                // Renderização atrasada: nada é baixado até o Explorer pedir CF_HDROP.
                *lock(&shared.pending) = entries;
                // SAFETY: handle nulo = renderização atrasada, válido para SetClipboardData.
                unsafe { SetClipboardData(CF_HDROP, None) }.map_err(err)?;
            }
        }
        set_bytes(f.remote, &[1])
    }
}
