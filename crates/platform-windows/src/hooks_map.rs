//! dono: W2
//! Funções puras KBDLLHOOKSTRUCT/MSLLHOOKSTRUCT → `CapturedEvent`, testadas no Mac. Recebem os
//! campos crus (inteiros) das structs, para não depender do crate `windows`.

use relay_core::types::{CapturedEvent, KeyCode, MediaKey, MouseButton, ScrollPhase};

pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const WM_XBUTTONUP: u32 = 0x020C;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;

/// `KBDLLHOOKSTRUCT.flags`.
pub const LLKHF_EXTENDED: u32 = 0x01;
pub const LLKHF_INJECTED: u32 = 0x10;
/// `MSLLHOOKSTRUCT.flags`.
pub const LLMHF_INJECTED: u32 = 0x01;

const VK_PAUSE: u32 = 0x13;
const VK_NUMLOCK: u32 = 0x90;
const SCAN_EXTENDED: u16 = 0xE000;
/// Pause: sequência E1 1D 45, representada como 0xE145 (convenção de C5a/W1).
const SCAN_PAUSE: u16 = 0xE145;
const SCAN_NUMLOCK: u16 = 0x45;
const SCAN_LSHIFT: u32 = 0x2A;
/// Uma "volta" da roda do mouse (`WHEEL_DELTA`).
const WHEEL_DELTA: f64 = 120.0;

/// True se o evento veio de `SendInput` (próprio ou de outro programa) e deve ser ignorado.
pub const fn keyboard_injected(flags: u32) -> bool {
    flags & LLKHF_INJECTED != 0
}

pub const fn mouse_injected(flags: u32) -> bool {
    flags & LLMHF_INJECTED != 0
}

fn media_from_vk(vk: u32) -> Option<MediaKey> {
    Some(match vk {
        0xAD => MediaKey::Mute,
        0xAE => MediaKey::VolumeDown,
        0xAF => MediaKey::VolumeUp,
        0xB0 => MediaKey::Next,
        0xB1 => MediaKey::Previous,
        0xB3 => MediaKey::PlayPause,
        _ => return None,
    })
}

/// KBDLLHOOKSTRUCT (`vk`, `scan`, `flags`) + mensagem → evento. `None` para mensagens que não são
/// tecla e para o shift falso (0xE02A) que o Windows insere em volta de teclas estendidas.
pub fn map_keyboard(msg: u32, vk: u32, scan: u32, flags: u32) -> Option<CapturedEvent> {
    let down = match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return None,
    };
    if let Some(key) = media_from_vk(vk) {
        return Some(CapturedEvent::Media { key, down });
    }
    let extended = flags & LLKHF_EXTENDED != 0;
    if extended && scan == SCAN_LSHIFT {
        return None;
    }
    let code = match vk {
        VK_PAUSE => SCAN_PAUSE,
        // O hook marca NumLock como estendido; o scan real é 0x45 puro.
        VK_NUMLOCK => SCAN_NUMLOCK,
        _ => (scan as u16 & 0xFF) | if extended { SCAN_EXTENDED } else { 0 },
    };
    Some(CapturedEvent::Key {
        code: KeyCode::Scan(code),
        down,
        flags: 0,
        autorepeat: false,
    })
}

/// MSLLHOOKSTRUCT (`pt`, `mouse_data`) + mensagem → evento. `origin` é a referência do delta:
/// a posição anterior, ou o centro preso enquanto o Mac controla.
pub fn map_mouse(
    msg: u32,
    pt: (i32, i32),
    mouse_data: u32,
    origin: (i32, i32),
) -> Option<CapturedEvent> {
    let button = |button, down| Some(CapturedEvent::Button { button, down });
    let xbutton = || match mouse_data >> 16 {
        1 => Some(MouseButton::X1),
        2 => Some(MouseButton::X2),
        _ => None,
    };
    // Delta da roda: HIWORD de `mouseData`, com sinal. Positivo = para cima/direita.
    let wheel = f64::from((mouse_data >> 16) as u16 as i16) / WHEEL_DELTA;
    match msg {
        WM_MOUSEMOVE => Some(CapturedEvent::MouseMove {
            dx: pt.0.saturating_sub(origin.0),
            dy: pt.1.saturating_sub(origin.1),
            pos: (f64::from(pt.0), f64::from(pt.1)),
        }),
        WM_LBUTTONDOWN => button(MouseButton::Left, true),
        WM_LBUTTONUP => button(MouseButton::Left, false),
        WM_RBUTTONDOWN => button(MouseButton::Right, true),
        WM_RBUTTONUP => button(MouseButton::Right, false),
        WM_MBUTTONDOWN => button(MouseButton::Middle, true),
        WM_MBUTTONUP => button(MouseButton::Middle, false),
        WM_XBUTTONDOWN => button(xbutton()?, true),
        WM_XBUTTONUP => button(xbutton()?, false),
        WM_MOUSEWHEEL => Some(CapturedEvent::Scroll {
            dx_px: 0.0,
            dy_px: wheel,
            phase: ScrollPhase::None,
            continuous: false,
        }),
        WM_MOUSEHWHEEL => Some(CapturedEvent::Scroll {
            dx_px: wheel,
            dy_px: 0.0,
            phase: ScrollPhase::None,
            continuous: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(code: u16, down: bool) -> Option<CapturedEvent> {
        Some(CapturedEvent::Key {
            code: KeyCode::Scan(code),
            down,
            flags: 0,
            autorepeat: false,
        })
    }

    #[test]
    fn tecla_comum_e_estendida() {
        assert_eq!(map_keyboard(WM_KEYDOWN, 0x41, 0x1E, 0), scan(0x1E, true));
        assert_eq!(map_keyboard(WM_KEYUP, 0x41, 0x1E, 0), scan(0x1E, false));
        // Seta para a esquerda: E0 4B.
        assert_eq!(
            map_keyboard(WM_SYSKEYDOWN, 0x25, 0x4B, LLKHF_EXTENDED),
            scan(0xE04B, true)
        );
    }

    #[test]
    fn pause_numlock_e_shift_falso() {
        assert_eq!(map_keyboard(WM_KEYDOWN, 0x13, 0x45, 0), scan(0xE145, true));
        assert_eq!(
            map_keyboard(WM_KEYDOWN, 0x90, 0x45, LLKHF_EXTENDED),
            scan(0x45, true)
        );
        assert_eq!(map_keyboard(WM_KEYDOWN, 0xA0, 0x2A, LLKHF_EXTENDED), None);
        assert_eq!(map_keyboard(WM_KEYDOWN, 0xA0, 0x2A, 0), scan(0x2A, true));
    }

    #[test]
    fn midia_vira_media() {
        assert_eq!(
            map_keyboard(WM_KEYDOWN, 0xAF, 0x30, LLKHF_EXTENDED),
            Some(CapturedEvent::Media {
                key: MediaKey::VolumeUp,
                down: true
            })
        );
        assert_eq!(
            map_keyboard(WM_KEYUP, 0xB3, 0x22, LLKHF_EXTENDED),
            Some(CapturedEvent::Media {
                key: MediaKey::PlayPause,
                down: false
            })
        );
    }

    #[test]
    fn mensagem_de_teclado_desconhecida() {
        assert_eq!(map_keyboard(0x0102, 0x41, 0x1E, 0), None);
    }

    #[test]
    fn flags_de_injecao() {
        assert!(keyboard_injected(LLKHF_INJECTED | LLKHF_EXTENDED));
        assert!(!keyboard_injected(LLKHF_EXTENDED));
        assert!(mouse_injected(LLMHF_INJECTED));
        assert!(!mouse_injected(0));
    }

    #[test]
    fn movimento_usa_delta_da_origem() {
        assert_eq!(
            map_mouse(WM_MOUSEMOVE, (105, 97), 0, (100, 100)),
            Some(CapturedEvent::MouseMove {
                dx: 5,
                dy: -3,
                pos: (105.0, 97.0)
            })
        );
    }

    #[test]
    fn botoes() {
        let b = |button, down| Some(CapturedEvent::Button { button, down });
        assert_eq!(
            map_mouse(WM_LBUTTONDOWN, (0, 0), 0, (0, 0)),
            b(MouseButton::Left, true)
        );
        assert_eq!(
            map_mouse(WM_RBUTTONUP, (0, 0), 0, (0, 0)),
            b(MouseButton::Right, false)
        );
        assert_eq!(
            map_mouse(WM_MBUTTONDOWN, (0, 0), 0, (0, 0)),
            b(MouseButton::Middle, true)
        );
        assert_eq!(
            map_mouse(WM_XBUTTONDOWN, (0, 0), 1 << 16, (0, 0)),
            b(MouseButton::X1, true)
        );
        assert_eq!(
            map_mouse(WM_XBUTTONUP, (0, 0), 2 << 16, (0, 0)),
            b(MouseButton::X2, false)
        );
        assert_eq!(map_mouse(WM_XBUTTONDOWN, (0, 0), 3 << 16, (0, 0)), None);
    }

    #[test]
    fn roda_vertical_e_horizontal_com_sinal() {
        let up = (120u16 as u32) << 16;
        let down = ((-240i16) as u16 as u32) << 16;
        assert_eq!(
            map_mouse(WM_MOUSEWHEEL, (0, 0), up, (0, 0)),
            Some(CapturedEvent::Scroll {
                dx_px: 0.0,
                dy_px: 1.0,
                phase: ScrollPhase::None,
                continuous: false
            })
        );
        assert_eq!(
            map_mouse(WM_MOUSEWHEEL, (0, 0), down, (0, 0)),
            Some(CapturedEvent::Scroll {
                dx_px: 0.0,
                dy_px: -2.0,
                phase: ScrollPhase::None,
                continuous: false
            })
        );
        assert_eq!(
            map_mouse(WM_MOUSEHWHEEL, (0, 0), up, (0, 0)),
            Some(CapturedEvent::Scroll {
                dx_px: 1.0,
                dy_px: 0.0,
                phase: ScrollPhase::None,
                continuous: false
            })
        );
    }

    #[test]
    fn mensagem_de_mouse_desconhecida() {
        assert_eq!(map_mouse(0x0203, (0, 0), 0, (0, 0)), None);
    }
}
