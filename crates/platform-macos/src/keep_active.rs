//! dono: MAC1
//! "Manter Mac ativo" (mesma solução do Bustelo, só a parte de atividade): depois de um minuto
//! sem input, um `flagsChanged` com os modificadores já pressionados zera o tempo ocioso do
//! sistema — é o que o Teams e afins leem para ficar "Ausente". Nenhuma tecla é pressionada e o
//! cursor não se move. Não impede o Mac de dormir (isso é "awake", fica com outro app).

use std::ffi::c_void;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use core_foundation::base::CFType;
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use objc2_app_kit::NSWorkspace;

use crate::capture::{FIELD_SOURCE_USER_DATA, RELAY_USER_DATA};

/// Ocioso por este tempo, o sistema recebe o toque; bem abaixo dos 5 min do Teams.
pub const IDLE_THRESHOLD: Duration = Duration::from_secs(60);
/// Consultar a cada poucos segundos mantém o ocioso perto do limite.
pub const TICK: Duration = Duration::from_secs(5);

/// O que o sistema mostra no momento em que um toque é considerado.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub idle: Duration,
    pub can_post: bool,
    pub on_console: bool,
    pub screen_locked: bool,
    pub display_asleep: bool,
    pub screen_saver: bool,
    pub mouse_down: bool,
}

/// Só toca quando o usuário está ocioso e nada foi deixado assim de propósito: tela bloqueada,
/// monitor dormindo e protetor de tela contam como ausência; clique/arrasto em curso, nunca.
pub fn should_nudge(s: &Snapshot, threshold: Duration) -> bool {
    s.can_post
        && s.on_console
        && !s.screen_locked
        && !s.display_asleep
        && !s.screen_saver
        && !s.mouse_down
        && s.idle >= threshold
}

// --- FFI --------------------------------------------------------------------------------------

use core_graphics::sys::CGEventRef;

const COMBINED_SESSION_STATE: i32 = 0;
const HID_SYSTEM_STATE: i32 = 1;
const ANY_INPUT_EVENT: u32 = !0;
const FLAGS_CHANGED: u32 = 12;
const HID_EVENT_TAP: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, ty: u32) -> f64;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreate(source: *mut c_void) -> CGEventRef;
    fn CGEventSetType(event: CGEventRef, ty: u32);
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGPreflightPostEventAccess() -> bool;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

fn session_flag(dict: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<bool> {
    dict.find(CFString::from_static_string(key))?
        .downcast::<CFBoolean>()
        .map(bool::from)
}

fn screen_saver_running() -> bool {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .any(|app| {
            app.bundleIdentifier()
                .is_some_and(|id| id.to_string() == "com.apple.ScreenSaver.Engine")
        })
}

fn snapshot() -> Snapshot {
    // SAFETY: as funções abaixo não recebem ponteiros (ou recebem só o dicionário que possuímos).
    unsafe {
        let raw = CGSessionCopyCurrentDictionary();
        let (on_console, screen_locked) = if raw.is_null() {
            (false, false)
        } else {
            let dict = CFDictionary::<CFString, CFType>::wrap_under_create_rule(raw);
            (
                session_flag(&dict, "kCGSSessionOnConsoleKey").unwrap_or(false),
                session_flag(&dict, "CGSSessionScreenIsLocked").unwrap_or(false),
            )
        };
        Snapshot {
            idle: Duration::from_secs_f64(
                CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
                    .max(0.0),
            ),
            can_post: CGPreflightPostEventAccess(),
            on_console,
            screen_locked,
            display_asleep: CGDisplayIsAsleep(CGMainDisplayID()) != 0,
            screen_saver: screen_saver_running(),
            mouse_down: (0..3).any(|b| CGEventSourceButtonState(COMBINED_SESSION_STATE, b)),
        }
    }
}

/// Modificadores iguais aos já pressionados: nada é digitado, mas o window server conta como input.
fn nudge() {
    // SAFETY: fonte e evento criados aqui e liberados no fim; CGEventPost não toma posse.
    unsafe {
        let source = CGEventSourceCreate(HID_SYSTEM_STATE);
        let event = CGEventCreate(source);
        if !event.is_null() {
            CGEventSetType(event, FLAGS_CHANGED);
            CGEventSetFlags(event, CGEventSourceFlagsState(COMBINED_SESSION_STATE));
            // Marca como do Relay para o tap não repassar o toque ao outro computador.
            CGEventSetIntegerValueField(event, FIELD_SOURCE_USER_DATA, RELAY_USER_DATA);
            CGEventPost(HID_EVENT_TAP, event);
            CFRelease(event as *const c_void);
        }
        if !source.is_null() {
            CFRelease(source as *const c_void);
        }
    }
}

/// Ligado enquanto existir: a thread toca o sistema quando ele fica ocioso, até o drop.
pub struct KeepActive {
    stop_tx: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for KeepActive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeepActive").finish_non_exhaustive()
    }
}

impl KeepActive {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            loop {
                if should_nudge(&snapshot(), IDLE_THRESHOLD) {
                    nudge();
                }
                match rx.recv_timeout(TICK) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
        });
        Self {
            stop_tx: Some(tx),
            thread: Some(thread),
        }
    }
}

impl Drop for KeepActive {
    fn drop(&mut self) {
        self.stop_tx = None; // fecha o canal e acorda a thread
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> Snapshot {
        Snapshot {
            idle: IDLE_THRESHOLD,
            can_post: true,
            on_console: true,
            screen_locked: false,
            display_asleep: false,
            screen_saver: false,
            mouse_down: false,
        }
    }

    #[test]
    fn toca_so_depois_do_limite_ocioso() {
        assert!(should_nudge(&idle(), IDLE_THRESHOLD));
        let s = Snapshot {
            idle: IDLE_THRESHOLD - Duration::from_secs(1),
            ..idle()
        };
        assert!(!should_nudge(&s, IDLE_THRESHOLD));
    }

    #[test]
    fn respeita_ausencia_deliberada_e_clique_em_curso() {
        let casos = [
            Snapshot {
                can_post: false,
                ..idle()
            },
            Snapshot {
                on_console: false,
                ..idle()
            },
            Snapshot {
                screen_locked: true,
                ..idle()
            },
            Snapshot {
                display_asleep: true,
                ..idle()
            },
            Snapshot {
                screen_saver: true,
                ..idle()
            },
            Snapshot {
                mouse_down: true,
                ..idle()
            },
        ];
        for s in casos {
            assert!(!should_nudge(&s, IDLE_THRESHOLD), "{s:?}");
        }
    }

    #[test]
    fn liga_e_desliga_sem_travar() {
        drop(KeepActive::start());
    }
}
