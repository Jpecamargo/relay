//! dono: W3
//! Observador do app em primeiro plano (SetWinEventHook).

use std::thread::JoinHandle;

/// Basename do exe de um caminho do Windows (aceita `\` e `/`); `None` se vazio.
pub fn exe_basename(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Chama o callback com o basename do exe sempre que o app em primeiro plano muda.
#[derive(Debug, Default)]
pub struct ForegroundWatcher {
    thread: Option<(u32, JoinHandle<()>)>,
}

impl ForegroundWatcher {
    /// Instala o hook numa thread própria (SetWinEventHook exige message loop) e já reporta o app
    /// atual. Chamar de novo substitui o callback anterior.
    pub fn start(&mut self, on_change: Box<dyn Fn(String) + Send + Sync>) {
        self.stop();
        #[cfg(windows)]
        {
            self.thread = imp::spawn(on_change);
        }
        #[cfg(not(windows))]
        drop(on_change);
    }

    pub fn stop(&mut self) {
        if let Some((_thread_id, handle)) = self.thread.take() {
            #[cfg(windows)]
            imp::quit(_thread_id);
            let _ = handle.join();
        }
    }
}

impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
mod imp {
    use super::exe_basename;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{
        GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, MSG, OBJID_WINDOW, PostThreadMessageW, TranslateMessage,
        WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
    };
    use windows::core::PWSTR;

    type Callback = Arc<dyn Fn(String) + Send + Sync>;

    // O hook não carrega contexto: só há um watcher ativo por processo.
    static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);

    fn exe_name_of(hwnd: HWND) -> Option<String> {
        let mut pid = 0u32;
        // SAFETY: `pid` é local e válido durante a chamada.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            return None;
        }
        // SAFETY: handle fechado no fim; o buffer tem `len` unidades e `len` é atualizado pela API.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(process);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
                .and_then(|p| exe_basename(&p))
        }
    }

    fn report(hwnd: HWND) {
        if hwnd.0.is_null() {
            return;
        }
        let Some(name) = exe_name_of(hwnd) else {
            return;
        };
        let cb = CALLBACK.lock().ok().and_then(|g| g.clone());
        if let Some(cb) = cb {
            cb(name);
        }
    }

    unsafe extern "system" fn on_event(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if id_object == OBJID_WINDOW.0 {
            report(hwnd);
        }
    }

    pub fn spawn(on_change: Box<dyn Fn(String) + Send + Sync>) -> Option<(u32, JoinHandle<()>)> {
        if let Ok(mut g) = CALLBACK.lock() {
            *g = Some(Arc::from(on_change));
        }
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("relay-foreground".into())
            .spawn(move || {
                // SAFETY: hook fora de contexto (callback roda nesta thread, via message loop
                // abaixo); é removido antes de a thread terminar.
                unsafe {
                    let hook = SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(on_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    );
                    let installed = !hook.0.is_null();
                    let _ = tx.send(installed.then(|| GetCurrentThreadId()));
                    if !installed {
                        return;
                    }
                    report(GetForegroundWindow());
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWinEvent(hook);
                }
            })
            .ok()?;
        match rx.recv().ok().flatten() {
            Some(id) => Some((id, handle)),
            None => {
                let _ = handle.join();
                None
            }
        }
    }

    pub fn quit(thread_id: u32) {
        // SAFETY: só posta uma mensagem para a thread do watcher.
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Ok(mut g) = CALLBACK.lock() {
            *g = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_do_exe() {
        assert_eq!(
            exe_basename(r"C:\Program Files\Google\Chrome\chrome.exe").as_deref(),
            Some("chrome.exe")
        );
        assert_eq!(exe_basename("C:/x/Code.exe").as_deref(), Some("Code.exe"));
        assert_eq!(exe_basename("notepad.exe").as_deref(), Some("notepad.exe"));
        assert_eq!(exe_basename(""), None);
        assert_eq!(exe_basename(r"C:\dir\"), None);
    }
}
