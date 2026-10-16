//! dono: M3
//! Eventos de sistema: sleep/wake (NSWorkspace) e mudança de rede (SCNetworkReachability).
//!
//! As notificações do NSWorkspace são entregues pelo run loop principal (o app Tauri já o roda).
//! A rede usa uma fila de dispatch própria e não depende de run loop.

use std::ffi::c_void;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSObject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    WillSleep,
    DidWake,
    NetworkChanged,
}

type Callback = Arc<dyn Fn(SystemEvent) + Send + Sync>;

struct ObserverIvars {
    callback: Callback,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "RelaySystemEventsObserver"]
    #[ivars = ObserverIvars]
    struct Observer;

    impl Observer {
        #[unsafe(method(willSleep:))]
        fn will_sleep(&self, _note: &NSNotification) {
            (self.ivars().callback)(SystemEvent::WillSleep);
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _note: &NSNotification) {
            (self.ivars().callback)(SystemEvent::DidWake);
        }
    }
);

impl Observer {
    fn new(callback: Callback) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { callback });
        unsafe { msg_send![super(this), init] }
    }
}

// --- SCNetworkReachability + libdispatch (FFI direto; libSystem já está linkada) ---

type Reachability = *const c_void;
type DispatchQueue = *mut c_void;

#[repr(C)]
struct ReachabilityContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[repr(C)]
struct SockaddrIn {
    len: u8,
    family: u8,
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkReachabilityCreateWithAddress(
        allocator: *const c_void,
        address: *const SockaddrIn,
    ) -> Reachability;
    fn SCNetworkReachabilitySetCallback(
        target: Reachability,
        callout: Option<unsafe extern "C" fn(Reachability, u32, *mut c_void)>,
        context: *mut ReachabilityContext,
    ) -> u8;
    fn SCNetworkReachabilitySetDispatchQueue(target: Reachability, queue: DispatchQueue) -> u8;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const u8, attr: *const c_void) -> DispatchQueue;
    fn dispatch_sync_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_release(object: *mut c_void);
    fn CFRelease(cf: *const c_void);
}

const NO_FLAGS_YET: u64 = u64::MAX;

struct NetState {
    callback: Callback,
    /// Últimos flags vistos (`NO_FLAGS_YET` antes do callback inicial, que é descartado).
    last: std::sync::atomic::AtomicU64,
    _pad: AtomicU32,
}

unsafe extern "C" fn on_reachability(_t: Reachability, flags: u32, info: *mut c_void) {
    // SAFETY: `info` é o Box<NetState> vivo até `NetMonitor::drop` (que drena a fila antes).
    let state = unsafe { &*(info as *const NetState) };
    let prev = state.last.swap(flags as u64, Ordering::SeqCst);
    if prev != NO_FLAGS_YET && prev != flags as u64 {
        (state.callback)(SystemEvent::NetworkChanged);
    }
}

unsafe extern "C" fn noop(_: *mut c_void) {}

struct NetMonitor {
    target: Reachability,
    queue: DispatchQueue,
    state: *mut NetState,
}

// SAFETY: os ponteiros só são usados em start/drop; o callback roda na fila de dispatch e o
// estado compartilhado é atômico + Arc<dyn Send + Sync>.
unsafe impl Send for NetMonitor {}

impl NetMonitor {
    fn start(callback: Callback) -> Option<Self> {
        let addr = SockaddrIn {
            len: std::mem::size_of::<SockaddrIn>() as u8,
            family: 2, // AF_INET
            port: 0,
            addr: 0,
            zero: [0; 8],
        };
        let state = Box::into_raw(Box::new(NetState {
            callback,
            last: std::sync::atomic::AtomicU64::new(NO_FLAGS_YET),
            _pad: AtomicU32::new(0),
        }));
        // SAFETY: chamadas FFI com ponteiros válidos; em falha desfaz o que foi criado.
        unsafe {
            let target = SCNetworkReachabilityCreateWithAddress(std::ptr::null(), &addr);
            let queue =
                dispatch_queue_create(c"relay.system-events.net".as_ptr().cast(), std::ptr::null());
            let mut ctx = ReachabilityContext {
                version: 0,
                info: state.cast(),
                retain: std::ptr::null(),
                release: std::ptr::null(),
                copy_description: std::ptr::null(),
            };
            let ok = !target.is_null()
                && !queue.is_null()
                && SCNetworkReachabilitySetCallback(target, Some(on_reachability), &mut ctx) != 0
                && SCNetworkReachabilitySetDispatchQueue(target, queue) != 0;
            let monitor = NetMonitor {
                target,
                queue,
                state,
            };
            if ok {
                Some(monitor)
            } else {
                drop(monitor);
                None
            }
        }
    }
}

impl Drop for NetMonitor {
    fn drop(&mut self) {
        // SAFETY: desliga a entrega, drena callbacks em voo e só então libera tudo.
        unsafe {
            if !self.target.is_null() {
                SCNetworkReachabilitySetDispatchQueue(self.target, std::ptr::null_mut());
            }
            if !self.queue.is_null() {
                dispatch_sync_f(self.queue, std::ptr::null_mut(), noop);
                dispatch_release(self.queue);
            }
            if !self.target.is_null() {
                CFRelease(self.target);
            }
            drop(Box::from_raw(self.state));
        }
    }
}

#[derive(Default)]
pub struct SystemEvents {
    observer: Option<Retained<Observer>>,
    net: Option<NetMonitor>,
}

impl fmt::Debug for SystemEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemEvents")
            .field("running", &self.observer.is_some())
            .finish()
    }
}

// SAFETY: o observer é só um handle para remoção no NSNotificationCenter (thread-safe) e
// `NetMonitor` já é Send; nenhum método do observer é chamado a partir daqui.
unsafe impl Send for SystemEvents {}

impl SystemEvents {
    /// Começa a entregar eventos (substitui uma assinatura anterior).
    pub fn start(&mut self, callback: Box<dyn Fn(SystemEvent) + Send + Sync>) {
        self.stop();
        let callback: Callback = Arc::from(callback);
        let observer = Observer::new(callback.clone());
        // SAFETY: seletores existem em `Observer` com a assinatura (NSNotification *).
        unsafe {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            let target: &AnyObject = &observer;
            center.addObserver_selector_name_object(
                target,
                sel!(willSleep:),
                Some(NSWorkspaceWillSleepNotification),
                None,
            );
            center.addObserver_selector_name_object(
                target,
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
        }
        self.observer = Some(observer);
        self.net = NetMonitor::start(callback);
    }

    pub fn stop(&mut self) {
        self.net = None;
        if let Some(observer) = self.observer.take() {
            let target: &AnyObject = &observer;
            // SAFETY: remove o observer registrado em `start`.
            unsafe {
                NSWorkspace::sharedWorkspace()
                    .notificationCenter()
                    .removeObserver(target);
            }
        }
    }
}

impl Drop for SystemEvents {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_stop_without_panic() {
        let mut ev = SystemEvents::default();
        ev.start(Box::new(|_| {}));
        ev.start(Box::new(|_| {})); // reinício substitui
        ev.stop();
        ev.stop();
    }

    // Sleep/wake e troca de rede reais não são reproduzíveis em teste automatizado.
    #[test]
    #[ignore = "manual: dormir/acordar o Mac ou trocar de Wi-Fi com um run loop ativo"]
    fn delivers_real_events() {}
}
