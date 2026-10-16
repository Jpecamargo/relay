//! dono: M1
//! Captura via CGEventTap no nível HID (kCGHIDEventTap, head-insert, opção "default" para poder
//! engolir). A conversão `CGEvent → CapturedEvent` é função pura (`convert`); o tap real exige a
//! permissão de Monitoramento de Entrada e só é exercitado à mão (`examples/tap_probe.rs`).
//!
//! Notas de comportamento:
//! - O tap é criado por FFI direta: o `CGEventType` do crate `core-graphics` não tem os tipos
//!   NSSystemDefined (14), rotate (18) e magnify (30), e receber um valor fora do enum é UB.
//! - Secure Input é respeitado: com ele ativo o sistema simplesmente não entrega as teclas ao tap;
//!   não há (nem deve haver) tentativa de contornar.
//! - Todo evento é entregue ao `sink`, inclusive com `swallow` ligado: é o core (HotkeyMatcher) quem
//!   reconhece atalhos, então eles chegam a ele de qualquer jeito. `swallow` só decide se o evento
//!   segue para o sistema local. `flagsChanged` nunca é engolido, para o estado de modificadores
//!   do Mac não divergir do que o usuário tem pressionado.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortRef};
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::EventField;
use core_graphics::geometry::CGPoint;
use core_graphics::sys::CGEventRef;
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::{Retained, autoreleasepool};
use objc2::{ClassType, msg_send};
use objc2_app_kit::NSEvent;
use relay_core::platform::{CaptureError, InputCapture};
use relay_core::types::{CapturedEvent, KeyCode, MediaKey, MouseButton, ScrollPhase};

// Tipos de evento (CGEventType / NSEventType). Valores crus: ver `CGEventTypes.h`.
/// `kCGEventSourceUserData`: o injetor (`inject.rs`) marca com `RELAY_USER_DATA` o que posta, e o
/// tap ignora esses eventos (não os captura de volta no sentido inverso).
pub const FIELD_SOURCE_USER_DATA: u32 = 42;
pub const RELAY_USER_DATA: i64 = 0x0052_454C_4159;

const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const RIGHT_MOUSE_DOWN: u32 = 3;
const RIGHT_MOUSE_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_MOUSE_DRAGGED: u32 = 6;
const RIGHT_MOUSE_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SYSTEM_DEFINED: u32 = 14;
const ROTATE: u32 = 18;
const SCROLL_WHEEL: u32 = 22;
const OTHER_MOUSE_DOWN: u32 = 25;
const OTHER_MOUSE_UP: u32 = 26;
const OTHER_MOUSE_DRAGGED: u32 = 27;
const MAGNIFY: u32 = 30;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

const SUBTYPE_AUX_CONTROL_BUTTONS: i16 = 8;

const FIELD_SCROLL_PHASE: u32 = 99;
const FIELD_MOMENTUM_PHASE: u32 = 123;

const EVENT_TYPES: [u32; 18] = [
    LEFT_MOUSE_DOWN,
    LEFT_MOUSE_UP,
    RIGHT_MOUSE_DOWN,
    RIGHT_MOUSE_UP,
    MOUSE_MOVED,
    LEFT_MOUSE_DRAGGED,
    RIGHT_MOUSE_DRAGGED,
    KEY_DOWN,
    KEY_UP,
    FLAGS_CHANGED,
    SYSTEM_DEFINED,
    ROTATE,
    SCROLL_WHEEL,
    OTHER_MOUSE_DOWN,
    OTHER_MOUSE_UP,
    OTHER_MOUSE_DRAGGED,
    MAGNIFY,
    0, // preenchimento do array; ignorado por `event_mask`
];

/// Máscara de eventos de interesse para `CGEventTapCreate`.
pub fn event_mask() -> u64 {
    EVENT_TYPES
        .iter()
        .filter(|&&t| t != 0)
        .fold(0u64, |m, &t| m | (1u64 << t))
}

/// O que o callback devolve ao sistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// O evento segue para o sistema local.
    Keep,
    /// O callback devolve NULL: o evento não chega ao sistema local.
    Drop,
}

/// Decide se o evento é engolido. Função pura. O evento já foi (ou será) entregue ao core em
/// qualquer caso, então atalhos continuam visíveis para o `HotkeyMatcher`.
pub fn disposition(event_type: u32, swallow: bool) -> Disposition {
    let swallowable = matches!(
        event_type,
        LEFT_MOUSE_DOWN
            | LEFT_MOUSE_UP
            | RIGHT_MOUSE_DOWN
            | RIGHT_MOUSE_UP
            | MOUSE_MOVED
            | LEFT_MOUSE_DRAGGED
            | RIGHT_MOUSE_DRAGGED
            | KEY_DOWN
            | KEY_UP
            | SYSTEM_DEFINED
            | ROTATE
            | SCROLL_WHEEL
            | OTHER_MOUSE_DOWN
            | OTHER_MOUSE_UP
            | OTHER_MOUSE_DRAGGED
            | MAGNIFY
    );
    if swallow && swallowable {
        Disposition::Drop
    } else {
        Disposition::Keep
    }
}

/// Trata os avisos de tap desativado (timeout ou input do usuário): reativa e conta.
/// `reenable` é injetável para teste. Devolve `true` se o tipo era de desativação.
pub fn handle_tap_disabled(
    event_type: u32,
    reenable: &mut dyn FnMut(),
    reactivations: &AtomicU64,
) -> bool {
    if event_type == TAP_DISABLED_BY_TIMEOUT || event_type == TAP_DISABLED_BY_USER_INPUT {
        reenable();
        reactivations.fetch_add(1, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Máscara device-dependent (ou, para caps/fn, a independente) do modificador de um keycode de
/// `flagsChanged`. `None` se o keycode não é de modificador.
const fn modifier_mask(keycode: u16) -> Option<u64> {
    Some(match keycode {
        59 => 0x0000_0001, // Control esq.
        62 => 0x0000_2000, // Control dir.
        56 => 0x0000_0002, // Shift esq.
        60 => 0x0000_0004, // Shift dir.
        55 => 0x0000_0008, // Command esq.
        54 => 0x0000_0010, // Command dir.
        58 => 0x0000_0020, // Option esq.
        61 => 0x0000_0040, // Option dir.
        57 => 0x0001_0000, // Caps Lock
        63 => 0x0080_0000, // Fn
        _ => return None,
    })
}

/// Sinal do scroll: o valor sai como o CGEvent entrega, e ele já traz a direção que o usuário
/// configurou (rolagem natural incluída). Positivo em `dy_px` = rolar para cima, o mesmo sinal
/// de WHEEL_DELTA no Windows; nada é invertido. `dx_px` é invertido: no CG positivo é rolar para a
/// esquerda, e a convenção do InputEvent::Scroll é dx>0 = direita (HWHEEL).
///
/// Converte um `CGEvent` dos tipos que o CoreGraphics descreve por campos. Função pura e sem
/// alocação. `None` para tipos tratados à parte (mídia/gestos) ou irrelevantes.
pub fn convert(event_type: u32, event: CGEventRef) -> Option<CapturedEvent> {
    let ev = Ev(event);
    match event_type {
        KEY_DOWN | KEY_UP => Some(CapturedEvent::Key {
            code: KeyCode::Mac(keycode(&ev)),
            down: event_type == KEY_DOWN,
            flags: ev.flags(),
            autorepeat: event_type == KEY_DOWN
                && ev.int(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
        }),
        FLAGS_CHANGED => {
            let code = keycode(&ev);
            let flags = ev.flags();
            let mask = modifier_mask(code)?;
            Some(CapturedEvent::Key {
                code: KeyCode::Mac(code),
                down: flags & mask != 0,
                flags,
                autorepeat: false,
            })
        }
        MOUSE_MOVED | LEFT_MOUSE_DRAGGED | RIGHT_MOUSE_DRAGGED | OTHER_MOUSE_DRAGGED => {
            let p = ev.location();
            Some(CapturedEvent::MouseMove {
                dx: ev.int(EventField::MOUSE_EVENT_DELTA_X) as i32,
                dy: ev.int(EventField::MOUSE_EVENT_DELTA_Y) as i32,
                pos: (p.x, p.y),
            })
        }
        LEFT_MOUSE_DOWN | LEFT_MOUSE_UP => Some(CapturedEvent::Button {
            button: MouseButton::Left,
            down: event_type == LEFT_MOUSE_DOWN,
        }),
        RIGHT_MOUSE_DOWN | RIGHT_MOUSE_UP => Some(CapturedEvent::Button {
            button: MouseButton::Right,
            down: event_type == RIGHT_MOUSE_DOWN,
        }),
        OTHER_MOUSE_DOWN | OTHER_MOUSE_UP => {
            let button = match ev.int(EventField::MOUSE_EVENT_BUTTON_NUMBER) {
                2 => MouseButton::Middle,
                3 => MouseButton::X1,
                4 => MouseButton::X2,
                _ => return None,
            };
            Some(CapturedEvent::Button {
                button,
                down: event_type == OTHER_MOUSE_DOWN,
            })
        }
        SCROLL_WHEEL => {
            let continuous = ev.int(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0;
            let (f1, f2) = if continuous {
                (
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
                )
            } else {
                (
                    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1,
                    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2,
                )
            };
            Some(CapturedEvent::Scroll {
                dx_px: -ev.dbl(f2),
                dy_px: ev.dbl(f1),
                phase: scroll_phase(ev.int(FIELD_SCROLL_PHASE), ev.int(FIELD_MOMENTUM_PHASE)),
                continuous,
            })
        }
        _ => None,
    }
}

fn keycode(ev: &Ev) -> u16 {
    ev.int(EventField::KEYBOARD_EVENT_KEYCODE) as u16
}

/// kCGScrollPhase*: Began=1, Changed=2, Ended=4, Cancelled=8, MayBegin=128;
/// kCGMomentumScrollPhase*: Begin=1, Continue=2, End=3. Momentum tem precedência.
pub fn scroll_phase(scroll: i64, momentum: i64) -> ScrollPhase {
    if momentum != 0 {
        return ScrollPhase::Momentum;
    }
    match scroll {
        1 | 128 => ScrollPhase::Began,
        2 => ScrollPhase::Changed,
        4 | 8 => ScrollPhase::Ended,
        _ => ScrollPhase::None,
    }
}

/// Converte o `data1` de um NSSystemDefined subtipo 8 (teclas de mídia). Brilho (D21) e demais
/// teclas auxiliares viram `None`. Estado da tecla: 0xA = pressionada, 0xB = solta.
pub fn media_from_data1(data1: i64) -> Option<CapturedEvent> {
    let code = ((data1 as u64 & 0xFFFF_0000) >> 16) as u32;
    let state = ((data1 as u64 & 0xFF00) >> 8) as u32;
    let key = match code {
        0 => MediaKey::VolumeUp,
        1 => MediaKey::VolumeDown,
        7 => MediaKey::Mute,
        16 => MediaKey::PlayPause,
        17 | 19 => MediaKey::Next,
        18 | 20 => MediaKey::Previous,
        _ => return None,
    };
    let down = match state {
        0xA => true,
        0xB => false,
        _ => return None,
    };
    Some(CapturedEvent::Media { key, down })
}

/// Leitura de campos de um `CGEventRef` vivo (não é dono do evento).
struct Ev(CGEventRef);

impl Ev {
    fn int(&self, field: u32) -> i64 {
        // SAFETY: o ponteiro é de um evento vivo durante a chamada.
        unsafe { CGEventGetIntegerValueField(self.0, field) }
    }
    fn dbl(&self, field: u32) -> f64 {
        unsafe { CGEventGetDoubleValueField(self.0, field) }
    }
    fn flags(&self) -> u64 {
        unsafe { CGEventGetFlags(self.0) }
    }
    fn location(&self) -> CGPoint {
        unsafe { CGEventGetLocation(self.0) }
    }
}

#[repr(C)]
struct CGEventOpaque {
    _p: [u8; 0],
}

// SAFETY: `CGEventRef` é `struct __CGEvent *`; só usado como ponteiro.
unsafe impl RefEncode for CGEventOpaque {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}

/// Mídia e gestos exigem o NSEvent correspondente (o CGEvent não expõe esses campos).
/// Aloca (objeto ObjC), mas só para eventos raros; teclas, mouse e scroll não passam por aqui.
fn convert_ns(event_type: u32, raw: CGEventRef) -> Option<CapturedEvent> {
    autoreleasepool(|_| {
        // SAFETY: `raw` é o evento vivo entregue ao callback.
        let ns: Option<Retained<NSEvent>> =
            unsafe { msg_send![NSEvent::class(), eventWithCGEvent: raw as *const CGEventOpaque] };
        let ns = ns?;
        match event_type {
            SYSTEM_DEFINED if ns.subtype().0 == SUBTYPE_AUX_CONTROL_BUTTONS => {
                media_from_data1(ns.data1() as i64)
            }
            MAGNIFY => Some(CapturedEvent::Magnify {
                delta: ns.magnification(),
            }),
            ROTATE => Some(CapturedEvent::Rotate {
                delta: f64::from(ns.rotation()),
            }),
            _ => None,
        }
    })
}

/// Caminho de decisão do callback, sem FFI: converte, entrega ao `push` (sempre, mesmo engolindo,
/// para o core enxergar atalhos) e diz o que fazer com o evento.
pub fn dispatch(
    event_type: u32,
    converted: Option<CapturedEvent>,
    swallow: bool,
    push: &mut dyn FnMut(CapturedEvent),
) -> Disposition {
    if let Some(ev) = converted {
        push(ev);
    }
    disposition(event_type, swallow)
}

type TapCallback = unsafe extern "C" fn(*const c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetDoubleValueField(event: CGEventRef, field: u32) -> f64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
}

unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;

struct TapState {
    sink: rtrb::Producer<CapturedEvent>,
    swallow: Arc<AtomicBool>,
    reactivations: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    port: CFMachPortRef,
}

unsafe extern "C" fn tap_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: `user_info` é o `TapState` da thread do tap, acessado só por ela.
    let state = unsafe { &mut *(user_info as *mut TapState) };
    let port = state.port;
    let mut reenable = || unsafe { CGEventTapEnable(port, true) };
    if handle_tap_disabled(event_type, &mut reenable, &state.reactivations) {
        return event;
    }
    if Ev(event).int(FIELD_SOURCE_USER_DATA) == RELAY_USER_DATA {
        return event;
    }
    let converted = match event_type {
        SYSTEM_DEFINED | ROTATE | MAGNIFY => convert_ns(event_type, event),
        _ => convert(event_type, event),
    };
    let TapState {
        sink,
        swallow,
        dropped,
        ..
    } = state;
    let mut push = |ev| {
        if sink.push(ev).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    };
    match dispatch(
        event_type,
        converted,
        swallow.load(Ordering::Relaxed),
        &mut push,
    ) {
        Disposition::Keep => event,
        Disposition::Drop => std::ptr::null_mut(),
    }
}

/// Captura do Mac. `start` sobe uma thread (QoS user-interactive) com o CFRunLoop do tap.
#[derive(Default)]
pub struct MacCapture {
    swallow: Arc<AtomicBool>,
    reactivations: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    running: Option<Running>,
}

struct Running {
    run_loop: CFRunLoop,
    thread: JoinHandle<()>,
}

impl std::fmt::Debug for MacCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacCapture")
            .field("running", &self.running.is_some())
            .field("swallow", &self.swallow.load(Ordering::Relaxed))
            .finish()
    }
}

impl MacCapture {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quantas vezes o tap foi reativado após timeout/input do usuário.
    pub fn reactivations(&self) -> u64 {
        self.reactivations.load(Ordering::Relaxed)
    }

    /// Eventos descartados porque o anel estava cheio.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl InputCapture for MacCapture {
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError> {
        if self.running.is_some() {
            return Err(CaptureError::AlreadyRunning);
        }
        let swallow = self.swallow.clone();
        let reactivations = self.reactivations.clone();
        let dropped = self.dropped.clone();
        let (tx, rx) = mpsc::channel::<Result<CFRunLoop, CaptureError>>();
        let thread = std::thread::Builder::new()
            .name("relay-event-tap".into())
            .spawn(move || {
                // SAFETY: chamada simples de libSystem sobre a thread corrente.
                unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
                let mut state = Box::new(TapState {
                    sink,
                    swallow,
                    reactivations,
                    dropped,
                    port: std::ptr::null_mut(),
                });
                let user_info = &mut *state as *mut TapState as *mut c_void;
                // HID (0), head-insert (0), opção default (0): permite engolir.
                // SAFETY: `state` vive até o fim da thread, depois do run loop parar.
                let port =
                    unsafe { CGEventTapCreate(0, 0, 0, event_mask(), tap_callback, user_info) };
                if port.is_null() {
                    let _ = tx.send(Err(CaptureError::PermissionDenied));
                    return;
                }
                state.port = port;
                // SAFETY: `port` é um CFMachPort recém-criado (create rule).
                let mach_port = unsafe { CFMachPort::wrap_under_create_rule(port) };
                let Ok(source) = mach_port.create_runloop_source(0) else {
                    let _ = tx.send(Err(CaptureError::Platform(
                        "falha ao criar a fonte do run loop".into(),
                    )));
                    return;
                };
                let run_loop = CFRunLoop::get_current();
                run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
                unsafe { CGEventTapEnable(port, true) };
                let _ = tx.send(Ok(run_loop));
                CFRunLoop::run_current();
                // Fim do run loop: o tap é invalidado ao soltar `mach_port`.
                drop(source);
                drop(mach_port);
                drop(state);
            })
            .map_err(|e| CaptureError::Platform(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(run_loop)) => {
                self.running = Some(Running { run_loop, thread });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(CaptureError::Platform("thread do tap terminou".into()))
            }
        }
    }

    fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.run_loop.stop();
            let _ = r.thread.join();
        }
    }

    fn set_swallow(&self, swallow: bool) {
        self.swallow.store(swallow, Ordering::Relaxed);
    }

    fn is_swallowing(&self) -> bool {
        self.swallow.load(Ordering::Relaxed)
    }
}

impl Drop for MacCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::base::CFRelease;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    // Contador de alocações por thread (testes rodam em paralelo).
    struct Counting;
    thread_local! { static ALLOCS: Cell<u64> = const { Cell::new(0) }; }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(p, l, n) }
        }
    }
    #[global_allocator]
    static A: Counting = Counting;

    fn allocs() -> u64 {
        ALLOCS.with(Cell::get)
    }

    unsafe extern "C" {
        fn CGEventCreateKeyboardEvent(src: *mut c_void, key: u16, down: bool) -> CGEventRef;
        fn CGEventCreateMouseEvent(
            src: *mut c_void,
            ty: u32,
            pos: CGPoint,
            button: u32,
        ) -> CGEventRef;
        fn CGEventCreateScrollWheelEvent2(
            src: *mut c_void,
            units: u32,
            count: u32,
            w1: i32,
            w2: i32,
            w3: i32,
        ) -> CGEventRef;
        fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, v: i64);
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
    }

    /// Evento sintético; solta o CFRetain no fim.
    struct Synth(CGEventRef);
    impl Synth {
        fn new(raw: CGEventRef) -> Self {
            assert!(!raw.is_null());
            Self(raw)
        }
    }
    impl Drop for Synth {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0 as *const c_void) }
        }
    }

    fn key(code: u16, down: bool) -> Synth {
        Synth::new(unsafe { CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, down) })
    }

    /// Converte e confere que a conversão em si não alocou.
    fn convert_no_alloc(ty: u32, ev: &Synth) -> Option<CapturedEvent> {
        let before = allocs();
        let out = convert(ty, ev.0);
        assert_eq!(allocs(), before, "convert alocou");
        out
    }

    #[test]
    fn tecla_a_down() {
        let ev = key(0, true);
        let out = convert_no_alloc(KEY_DOWN, &ev).unwrap();
        match out {
            CapturedEvent::Key {
                code,
                down,
                autorepeat,
                ..
            } => {
                assert_eq!(code, KeyCode::Mac(0));
                assert!(down);
                assert!(!autorepeat);
            }
            other => panic!("esperava Key, veio {other:?}"),
        }
    }

    #[test]
    fn tecla_up_e_autorepeat() {
        let ev = key(0, true);
        unsafe { CGEventSetIntegerValueField(ev.0, EventField::KEYBOARD_EVENT_AUTOREPEAT, 1) };
        assert!(matches!(
            convert_no_alloc(KEY_DOWN, &ev),
            Some(CapturedEvent::Key {
                autorepeat: true,
                down: true,
                ..
            })
        ));
        let up = key(0, false);
        assert!(matches!(
            convert_no_alloc(KEY_UP, &up),
            Some(CapturedEvent::Key {
                autorepeat: false,
                down: false,
                ..
            })
        ));
    }

    #[test]
    fn flags_changed_usa_bit_do_lado() {
        let ev = key(56, true); // Shift esquerdo
        unsafe { CGEventSetFlags(ev.0, 0x0002_0002) };
        assert!(matches!(
            convert_no_alloc(FLAGS_CHANGED, &ev),
            Some(CapturedEvent::Key {
                code: KeyCode::Mac(56),
                down: true,
                ..
            })
        ));
        // Shift direito segue pressionado, esquerdo solto: não é "down" para keycode 56.
        unsafe { CGEventSetFlags(ev.0, 0x0002_0004) };
        assert!(matches!(
            convert_no_alloc(FLAGS_CHANGED, &ev),
            Some(CapturedEvent::Key {
                code: KeyCode::Mac(56),
                down: false,
                ..
            })
        ));
        let nao_mod = key(0, true);
        assert_eq!(convert_no_alloc(FLAGS_CHANGED, &nao_mod), None);
    }

    #[test]
    fn evento_marcado_pelo_relay_e_reconhecido() {
        let ev = Synth::new(unsafe {
            CGEventCreateMouseEvent(std::ptr::null_mut(), MOUSE_MOVED, CGPoint::new(1.0, 2.0), 0)
        });
        assert_ne!(Ev(ev.0).int(FIELD_SOURCE_USER_DATA), RELAY_USER_DATA);
        unsafe { CGEventSetIntegerValueField(ev.0, FIELD_SOURCE_USER_DATA, RELAY_USER_DATA) };
        assert_eq!(Ev(ev.0).int(FIELD_SOURCE_USER_DATA), RELAY_USER_DATA);
    }

    #[test]
    fn mouse_delta_dos_campos() {
        let ev = Synth::new(unsafe {
            CGEventCreateMouseEvent(
                std::ptr::null_mut(),
                MOUSE_MOVED,
                CGPoint::new(10.0, 20.0),
                0,
            )
        });
        unsafe {
            CGEventSetIntegerValueField(ev.0, EventField::MOUSE_EVENT_DELTA_X, 7);
            CGEventSetIntegerValueField(ev.0, EventField::MOUSE_EVENT_DELTA_Y, -3);
        }
        assert_eq!(
            convert_no_alloc(MOUSE_MOVED, &ev),
            Some(CapturedEvent::MouseMove {
                dx: 7,
                dy: -3,
                pos: (10.0, 20.0)
            })
        );
    }

    #[test]
    fn botoes() {
        let mk = |ty| {
            Synth::new(unsafe {
                CGEventCreateMouseEvent(std::ptr::null_mut(), ty, CGPoint::new(0.0, 0.0), 0)
            })
        };
        let l = mk(LEFT_MOUSE_DOWN);
        assert_eq!(
            convert_no_alloc(LEFT_MOUSE_DOWN, &l),
            Some(CapturedEvent::Button {
                button: MouseButton::Left,
                down: true
            })
        );
        let r = mk(RIGHT_MOUSE_UP);
        assert_eq!(
            convert_no_alloc(RIGHT_MOUSE_UP, &r),
            Some(CapturedEvent::Button {
                button: MouseButton::Right,
                down: false
            })
        );
        let o = mk(OTHER_MOUSE_DOWN);
        for (n, esperado) in [
            (2, Some(MouseButton::Middle)),
            (3, Some(MouseButton::X1)),
            (4, Some(MouseButton::X2)),
            (7, None),
        ] {
            unsafe { CGEventSetIntegerValueField(o.0, EventField::MOUSE_EVENT_BUTTON_NUMBER, n) };
            assert_eq!(
                convert_no_alloc(OTHER_MOUSE_DOWN, &o),
                esperado.map(|button| CapturedEvent::Button { button, down: true })
            );
        }
    }

    #[test]
    fn scroll_continuo_em_pixels() {
        let ev = Synth::new(unsafe {
            CGEventCreateScrollWheelEvent2(
                std::ptr::null_mut(),
                0, // pixel
                2,
                -12,
                5,
                0,
            )
        });
        unsafe { CGEventSetIntegerValueField(ev.0, FIELD_SCROLL_PHASE, 2) };
        match convert_no_alloc(SCROLL_WHEEL, &ev).unwrap() {
            CapturedEvent::Scroll {
                dx_px,
                dy_px,
                phase,
                continuous,
            } => {
                assert!(continuous);
                assert_eq!(dy_px, -12.0);
                assert_eq!(dx_px, -5.0);
                assert_eq!(phase, ScrollPhase::Changed);
            }
            other => panic!("esperava Scroll, veio {other:?}"),
        }
    }

    #[test]
    fn scroll_horizontal_positivo_e_direita() {
        // CG: wheel2 positivo = esquerda; a conversão inverte para dx>0 = direita.
        let ev = Synth::new(unsafe {
            CGEventCreateScrollWheelEvent2(std::ptr::null_mut(), 1, 2, 0, 4, 0)
        });
        match convert_no_alloc(SCROLL_WHEEL, &ev).unwrap() {
            CapturedEvent::Scroll { dx_px, .. } => assert_eq!(dx_px, -4.0),
            other => panic!("esperava Scroll, veio {other:?}"),
        }
    }

    #[test]
    fn scroll_de_roda_em_linhas() {
        let ev = Synth::new(unsafe {
            CGEventCreateScrollWheelEvent2(std::ptr::null_mut(), 1, 1, 3, 0, 0)
        });
        match convert_no_alloc(SCROLL_WHEEL, &ev).unwrap() {
            CapturedEvent::Scroll {
                dy_px, continuous, ..
            } => {
                assert!(!continuous);
                // Positivo = para cima (sinal de WHEEL_DELTA), sem inversão.
                assert_eq!(dy_px, 3.0);
            }
            other => panic!("esperava Scroll, veio {other:?}"),
        }
    }

    #[test]
    fn fases_de_scroll() {
        assert_eq!(scroll_phase(1, 0), ScrollPhase::Began);
        assert_eq!(scroll_phase(2, 0), ScrollPhase::Changed);
        assert_eq!(scroll_phase(4, 0), ScrollPhase::Ended);
        assert_eq!(scroll_phase(0, 0), ScrollPhase::None);
        assert_eq!(scroll_phase(0, 2), ScrollPhase::Momentum);
    }

    #[test]
    fn midia_data1() {
        // volume up pressionado: código 0, estado 0xA
        let d = |code: i64, state: i64| (code << 16) | (state << 8);
        assert_eq!(
            media_from_data1(d(0, 0xA)),
            Some(CapturedEvent::Media {
                key: MediaKey::VolumeUp,
                down: true
            })
        );
        assert_eq!(
            media_from_data1(d(16, 0xB)),
            Some(CapturedEvent::Media {
                key: MediaKey::PlayPause,
                down: false
            })
        );
        assert_eq!(
            media_from_data1(d(2, 0xA)),
            None,
            "brilho nunca é enviado (D21)"
        );
        assert_eq!(media_from_data1(d(0, 0x5)), None);
    }

    #[test]
    fn engolir_so_tipos_engoliveis() {
        assert_eq!(disposition(KEY_DOWN, true), Disposition::Drop);
        assert_eq!(disposition(MOUSE_MOVED, true), Disposition::Drop);
        assert_eq!(disposition(SYSTEM_DEFINED, true), Disposition::Drop);
        assert_eq!(disposition(FLAGS_CHANGED, true), Disposition::Keep);
        assert_eq!(disposition(KEY_DOWN, false), Disposition::Keep);
    }

    #[test]
    fn engolindo_o_core_ainda_recebe_o_evento() {
        let ev = key(0, true);
        let converted = convert(KEY_DOWN, ev.0);
        let mut entregues = Vec::new();
        let d = dispatch(KEY_DOWN, converted, true, &mut |e| entregues.push(e));
        assert_eq!(d, Disposition::Drop);
        assert_eq!(entregues.len(), 1);
    }

    #[test]
    fn tap_desativado_reativa_e_conta() {
        let contador = AtomicU64::new(0);
        let mut reativou = 0;
        for ty in [TAP_DISABLED_BY_TIMEOUT, TAP_DISABLED_BY_USER_INPUT] {
            assert!(handle_tap_disabled(ty, &mut || reativou += 1, &contador));
        }
        assert!(!handle_tap_disabled(
            KEY_DOWN,
            &mut || reativou += 1,
            &contador
        ));
        assert_eq!(reativou, 2);
        assert_eq!(contador.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn mascara_inclui_tipos_e_exclui_desativacao() {
        let m = event_mask();
        for t in [
            KEY_DOWN,
            SCROLL_WHEEL,
            MAGNIFY,
            ROTATE,
            SYSTEM_DEFINED,
            MOUSE_MOVED,
        ] {
            assert!(m & (1 << t) != 0);
        }
        assert_eq!(m & (1 << 0), 0);
    }

    #[test]
    fn sem_permissao_start_nao_entra_em_panico() {
        // Com permissão o tap sobe de verdade; sem, deve devolver PermissionDenied.
        let (tx, _rx) = rtrb::RingBuffer::<CapturedEvent>::new(8);
        let mut cap = MacCapture::new();
        match cap.start(tx) {
            Ok(()) => cap.stop(),
            Err(e) => assert!(matches!(e, CaptureError::PermissionDenied), "{e:?}"),
        }
    }
}
