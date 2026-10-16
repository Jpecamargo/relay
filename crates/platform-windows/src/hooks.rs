//! dono: W2
//! Captura via hooks WH_KEYBOARD_LL e WH_MOUSE_LL (sentido inverso: o mouse/teclado do PC
//! controlam o Mac). Os callbacks só copiam o evento para o `rtrb` e retornam; eventos injetados
//! (LLKHF_INJECTED/LLMHF_INJECTED) são ignorados. A conversão está em `hooks_map`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use relay_core::platform::{CaptureError, InputCapture};
use relay_core::types::CapturedEvent;

/// Estado lido pelos callbacks (que são `extern "system"` e não carregam contexto). Só atômicos:
/// sem lock e sem alocação no caminho do hook.
static SWALLOW: AtomicBool = AtomicBool::new(false);
#[cfg_attr(not(windows), allow(dead_code))]
static LOCKED: AtomicBool = AtomicBool::new(false);
#[cfg_attr(not(windows), allow(dead_code))]
static LOCK_X: AtomicI32 = AtomicI32::new(0);
#[cfg_attr(not(windows), allow(dead_code))]
static LOCK_Y: AtomicI32 = AtomicI32::new(0);
#[cfg_attr(not(windows), allow(dead_code))]
static LAST_X: AtomicI32 = AtomicI32::new(0);
#[cfg_attr(not(windows), allow(dead_code))]
static LAST_Y: AtomicI32 = AtomicI32::new(0);

/// Liga/desliga o centro preso: com `Some`, o delta do mouse é medido contra ele (cursor.rs).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn set_lock(center: Option<(i32, i32)>) {
    if let Some((x, y)) = center {
        LOCK_X.store(x, Ordering::Relaxed);
        LOCK_Y.store(y, Ordering::Relaxed);
    }
    LOCKED.store(center.is_some(), Ordering::Release);
}

/// Posição de referência do próximo delta quando não há centro preso.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn sync_last(x: i32, y: i32) {
    LAST_X.store(x, Ordering::Relaxed);
    LAST_Y.store(y, Ordering::Relaxed);
}

/// Posição de referência do próximo delta (só os testes a leem).
#[cfg(test)]
pub(crate) fn last_pos() -> (i32, i32) {
    (
        LAST_X.load(Ordering::Relaxed),
        LAST_Y.load(Ordering::Relaxed),
    )
}

/// Movimento injetado (SendInput/SetCursorPos do próprio agente quando o Mac controla o PC):
/// não vira evento, mas move o cursor, então a referência do próximo delta físico acompanha —
/// senão o primeiro delta depois dele sai inflado. Com o centro preso a referência é o centro.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn note_injected_move(pt: (i32, i32)) {
    if !LOCKED.load(Ordering::Acquire) {
        sync_last(pt.0, pt.1);
    }
}

#[derive(Debug, Default)]
pub struct WindowsHooks {
    #[cfg(windows)]
    worker: Option<imp::Worker>,
}

impl InputCapture for WindowsHooks {
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        #[cfg(windows)]
        {
            if self.worker.is_some() {
                return Err(CaptureError::AlreadyRunning);
            }
            self.worker = Some(imp::spawn(sink)?);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = sink;
            Err(CaptureError::NotImplemented)
        }
    }

    fn stop(&mut self) {
        #[cfg(windows)]
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
        SWALLOW.store(false, Ordering::Release);
    }

    fn set_swallow(&self, swallow: bool) {
        SWALLOW.store(swallow, Ordering::Release);
    }

    fn is_swallowing(&self) -> bool {
        SWALLOW.load(Ordering::Acquire)
    }
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, KillTimer,
        MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetCursorPos, SetTimer, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT, WM_TIMER,
    };

    use super::*;
    use crate::hooks_map::{
        WM_MOUSEMOVE, keyboard_injected, map_keyboard, map_mouse, mouse_injected,
    };

    /// Intervalo do watchdog: reinstala os hooks que o Windows possa ter removido (ex.: callback
    /// que estourou LowLevelHooksTimeout).
    const WATCHDOG_MS: u32 = 3000;

    thread_local! {
        /// Só a thread dos hooks (que roda os callbacks) toca nisto.
        static SINK: RefCell<Option<rtrb::Producer<CapturedEvent>>> = const { RefCell::new(None) };
    }

    pub struct Worker {
        thread_id: u32,
        handle: JoinHandle<()>,
    }

    impl std::fmt::Debug for Worker {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Worker")
                .field("thread_id", &self.thread_id)
                .finish()
        }
    }

    impl Worker {
        pub fn stop(self) {
            // SAFETY: id de uma thread nossa; se ela já saiu o Post só falha.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = self.handle.join();
        }
    }

    fn push(event: CapturedEvent) {
        // Fila cheia: descarta o evento (nunca bloqueia o hook).
        SINK.with(|sink| {
            if let Some(producer) = sink.borrow_mut().as_mut() {
                let _ = producer.push(event);
            }
        });
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: com HC_ACTION, lparam aponta para um KBDLLHOOKSTRUCT válido.
            let s = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if !keyboard_injected(s.flags.0) {
                if let Some(event) = map_keyboard(wparam.0 as u32, s.vkCode, s.scanCode, s.flags.0)
                {
                    push(event);
                }
                if SWALLOW.load(Ordering::Acquire) {
                    return LRESULT(1);
                }
            }
        }
        // SAFETY: repassa os argumentos recebidos.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: com HC_ACTION, lparam aponta para um MSLLHOOKSTRUCT válido.
            let s = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if mouse_injected(s.flags) {
                note_injected_move((s.pt.x, s.pt.y));
            } else {
                let pt = (s.pt.x, s.pt.y);
                let locked = LOCKED.load(Ordering::Acquire);
                let center = (
                    LOCK_X.load(Ordering::Relaxed),
                    LOCK_Y.load(Ordering::Relaxed),
                );
                let last = (
                    LAST_X.load(Ordering::Relaxed),
                    LAST_Y.load(Ordering::Relaxed),
                );
                sync_last(pt.0, pt.1);
                let msg = wparam.0 as u32;
                let origin = if locked { center } else { last };
                if let Some(event) = map_mouse(msg, pt, s.mouseData, origin) {
                    let still = matches!(event, CapturedEvent::MouseMove { dx: 0, dy: 0, .. });
                    if !still {
                        push(event);
                    }
                }
                if locked && msg == WM_MOUSEMOVE {
                    // Volta ao centro; o SetCursorPos gera um evento injetado, ignorado acima.
                    // SAFETY: chamada Win32 sem ponteiros.
                    unsafe {
                        let _ = SetCursorPos(center.0, center.1);
                    }
                }
                if SWALLOW.load(Ordering::Acquire) {
                    return LRESULT(1);
                }
            }
        }
        // SAFETY: repassa os argumentos recebidos.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn install() -> Result<(HHOOK, HHOOK), CaptureError> {
        let err = |e: windows::core::Error| CaptureError::Platform(e.to_string());
        // SAFETY: hooks LL não injetam DLL; `hmod` só precisa ser um módulo válido do processo.
        unsafe {
            let module = GetModuleHandleW(None).map_err(err)?;
            let hmod = Some(HINSTANCE(module.0));
            let kb =
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0).map_err(err)?;
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0) {
                Ok(ms) => Ok((kb, ms)),
                Err(e) => {
                    let _ = UnhookWindowsHookEx(kb);
                    Err(err(e))
                }
            }
        }
    }

    fn uninstall((kb, ms): (HHOOK, HHOOK)) {
        // SAFETY: handles criados por `install`.
        unsafe {
            let _ = UnhookWindowsHookEx(kb);
            let _ = UnhookWindowsHookEx(ms);
        }
    }

    pub fn spawn(sink: rtrb::Producer<CapturedEvent>) -> Result<Worker, CaptureError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, CaptureError>>();
        let handle = std::thread::Builder::new()
            .name("relay-hooks".into())
            .spawn(move || run(sink, ready_tx))
            .map_err(|e| CaptureError::Platform(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Worker { thread_id, handle }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err(CaptureError::Platform("thread dos hooks encerrou".into())),
        }
    }

    fn run(sink: rtrb::Producer<CapturedEvent>, ready: mpsc::Sender<Result<u32, CaptureError>>) {
        SINK.with(|s| *s.borrow_mut() = Some(sink));
        let mut hooks = match install() {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        // SAFETY: chamadas Win32 de loop de mensagens na thread que instalou os hooks.
        unsafe {
            let mut p = windows::Win32::Foundation::POINT::default();
            if GetCursorPos(&mut p).is_ok() {
                sync_last(p.x, p.y);
            }
            let timer = SetTimer(None, 0, WATCHDOG_MS, None);
            let _ = ready.send(Ok(GetCurrentThreadId()));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if msg.message == WM_TIMER {
                    // Instala o novo par antes de remover o antigo: não perde eventos.
                    if let Ok(fresh) = install() {
                        uninstall(hooks);
                        hooks = fresh;
                    }
                }
            }
            if timer != 0 {
                let _ = KillTimer(None, timer);
            }
        }
        uninstall(hooks);
        SINK.with(|s| *s.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movimento_injetado_atualiza_a_referencia_do_delta() {
        set_lock(None);
        sync_last(10, 10);
        note_injected_move((500, 300));
        assert_eq!(last_pos(), (500, 300));
        // Com o centro preso a referência é o centro: o movimento injetado não a altera.
        set_lock(Some((50, 50)));
        note_injected_move((700, 700));
        assert_eq!(last_pos(), (500, 300));
        set_lock(None);
    }
}
