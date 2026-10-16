//! dono: W1
//! Montagem pura de eventos SendInput (normalização virtual, botões, scan codes, UTF-16),
//! compilada e testada em todos os targets. `inject.rs` converte `RawInput` em `INPUT`.

use relay_core::types::{InputEvent, MediaKey, MouseButton};

// MOUSEEVENTF_*
pub const MOUSEEVENTF_MOVE: u32 = 0x0001;
pub const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
pub const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
pub const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
pub const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
pub const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
pub const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
pub const MOUSEEVENTF_XDOWN: u32 = 0x0080;
pub const MOUSEEVENTF_XUP: u32 = 0x0100;
pub const MOUSEEVENTF_WHEEL: u32 = 0x0800;
pub const MOUSEEVENTF_HWHEEL: u32 = 0x1000;
pub const MOUSEEVENTF_VIRTUALDESK: u32 = 0x4000;
pub const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
pub const XBUTTON1: u32 = 0x0001;
pub const XBUTTON2: u32 = 0x0002;

// KEYEVENTF_*
pub const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
pub const KEYEVENTF_KEYUP: u32 = 0x0002;
pub const KEYEVENTF_UNICODE: u32 = 0x0004;
pub const KEYEVENTF_SCANCODE: u32 = 0x0008;

/// Scan interno da tecla Pause (C5a).
pub const SCAN_PAUSE: u16 = 0xE145;

// VK_MEDIA_* / VK_VOLUME_*
pub const VK_VOLUME_MUTE: u16 = 0xAD;
pub const VK_VOLUME_DOWN: u16 = 0xAE;
pub const VK_VOLUME_UP: u16 = 0xAF;
pub const VK_MEDIA_NEXT_TRACK: u16 = 0xB0;
pub const VK_MEDIA_PREV_TRACK: u16 = 0xB1;
pub const VK_MEDIA_PLAY_PAUSE: u16 = 0xB3;

/// Retângulo da área de trabalho virtual em pixels físicos (SM_*VIRTUALSCREEN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMouse {
    pub dx: i32,
    pub dy: i32,
    pub data: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub vk: u16,
    pub scan: u16,
    pub flags: u32,
}

/// Espelho neutro de `INPUT` (sem dependência do crate `windows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawInput {
    Mouse(RawMouse),
    Key(RawKey),
}

/// Entradas de um evento (no máximo 3: roda vertical e horizontal; Pause), no array da pilha.
#[derive(Debug, Clone, Copy)]
pub struct Batch {
    items: [RawInput; 3],
    len: usize,
}

impl Batch {
    const EMPTY: RawInput = RawInput::Key(RawKey {
        vk: 0,
        scan: 0,
        flags: 0,
    });

    fn one(a: RawInput) -> Self {
        Self {
            items: [a, Self::EMPTY, Self::EMPTY],
            len: 1,
        }
    }

    pub fn as_slice(&self) -> &[RawInput] {
        &self.items[..self.len]
    }
}

/// Posição em pixels físicos → 0..=65535 sobre a área virtual (cantos exatos; fora é limitado).
pub fn normalize_virtual(x: i32, y: i32, rect: VirtualRect) -> (i32, i32) {
    fn axis(v: i32, origin: i32, size: i32) -> i32 {
        let span = i64::from(size.max(2) - 1);
        let rel = i64::from(v - origin).clamp(0, span);
        ((rel * 65535 + span / 2) / span) as i32
    }
    (
        axis(x, rect.left, rect.width),
        axis(y, rect.top, rect.height),
    )
}

/// Flag de botão (e `mouseData` para X1/X2).
pub fn button_flags(button: MouseButton, down: bool) -> (u32, u32) {
    match (button, down) {
        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1),
        (MouseButton::X1, false) => (MOUSEEVENTF_XUP, XBUTTON1),
        (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2),
        (MouseButton::X2, false) => (MOUSEEVENTF_XUP, XBUTTON2),
    }
}

/// Scan set 1 (0xE0xx = estendida) → `wScan` + flags.
pub fn scan_key(scan: u16, down: bool) -> RawKey {
    let mut flags = KEYEVENTF_SCANCODE;
    if scan >> 8 == 0xE0 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    RawKey {
        vk: 0,
        scan: scan & 0xFF,
        flags,
    }
}

pub fn media_vk(key: MediaKey) -> u16 {
    match key {
        MediaKey::VolumeUp => VK_VOLUME_UP,
        MediaKey::VolumeDown => VK_VOLUME_DOWN,
        MediaKey::Mute => VK_VOLUME_MUTE,
        MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
        MediaKey::Next => VK_MEDIA_NEXT_TRACK,
        MediaKey::Previous => VK_MEDIA_PREV_TRACK,
    }
}

pub fn media_key(key: MediaKey, down: bool) -> RawKey {
    RawKey {
        vk: media_vk(key),
        scan: 0,
        flags: if down { 0 } else { KEYEVENTF_KEYUP },
    }
}

/// Para cada unidade UTF-16 (surrogates separados), um par down/up com KEYEVENTF_UNICODE.
pub fn text_inputs(text: &str) -> Vec<RawInput> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            out.push(RawInput::Key(RawKey {
                vk: 0,
                scan: unit,
                flags,
            }));
        }
    }
    out
}

/// Monta as entradas SendInput de um evento. Sem alocação.
pub fn event_inputs(event: &InputEvent, rect: VirtualRect) -> Batch {
    let mouse = |dx, dy, data, flags| {
        RawInput::Mouse(RawMouse {
            dx,
            dy,
            data,
            flags,
        })
    };
    match *event {
        InputEvent::MouseMove { dx, dy } => {
            Batch::one(mouse(i32::from(dx), i32::from(dy), 0, MOUSEEVENTF_MOVE))
        }
        InputEvent::MouseAbs { x, y } => {
            let (nx, ny) = normalize_virtual(x, y, rect);
            Batch::one(mouse(
                nx,
                ny,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            ))
        }
        InputEvent::Button { button, down } => {
            let (flags, data) = button_flags(button, down);
            Batch::one(mouse(0, 0, data, flags))
        }
        InputEvent::Scroll { dx, dy, .. } => {
            // mouseData é o delta com sinal em 1/120 de degrau, qualquer valor (hi-res).
            let wheel = |flags, d: i16| mouse(0, 0, i32::from(d) as u32, flags);
            let mut b = Batch {
                items: [Batch::EMPTY; 3],
                len: 0,
            };
            if dy != 0 {
                b.items[b.len] = wheel(MOUSEEVENTF_WHEEL, dy);
                b.len += 1;
            }
            if dx != 0 {
                b.items[b.len] = wheel(MOUSEEVENTF_HWHEEL, dx);
                b.len += 1;
            }
            b
        }
        InputEvent::Key { scan, down } if scan == SCAN_PAUSE => {
            // Pause é a sequência E1 1D 45 (sem tecla estendida E0); no up, 1D e 45 com KEYUP.
            let up = if down { 0 } else { KEYEVENTF_KEYUP };
            let k = |scan, flags| {
                RawInput::Key(RawKey {
                    vk: 0,
                    scan,
                    flags: KEYEVENTF_SCANCODE | flags,
                })
            };
            Batch {
                items: [k(0xE1, 0), k(0x1D, up), k(0x45, up)],
                len: 3,
            }
        }
        InputEvent::Key { scan, down } => Batch::one(RawInput::Key(scan_key(scan, down))),
        InputEvent::Media { key, down } => Batch::one(RawInput::Key(media_key(key, down))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_core::types::ScrollPhase;

    const R: VirtualRect = VirtualRect {
        left: -1920,
        top: 0,
        width: 4480,
        height: 1440,
    };

    #[test]
    fn normalize_corners() {
        assert_eq!(normalize_virtual(-1920, 0, R), (0, 0));
        assert_eq!(normalize_virtual(2559, 1439, R), (65535, 65535));
        assert_eq!(normalize_virtual(-1920, 1439, R), (0, 65535));
        assert_eq!(normalize_virtual(5000, -50, R), (65535, 0));
    }

    #[test]
    fn buttons() {
        assert_eq!(
            button_flags(MouseButton::Left, true),
            (MOUSEEVENTF_LEFTDOWN, 0)
        );
        assert_eq!(
            button_flags(MouseButton::Right, false),
            (MOUSEEVENTF_RIGHTUP, 0)
        );
        assert_eq!(
            button_flags(MouseButton::Middle, true),
            (MOUSEEVENTF_MIDDLEDOWN, 0)
        );
        assert_eq!(
            button_flags(MouseButton::X1, true),
            (MOUSEEVENTF_XDOWN, XBUTTON1)
        );
        assert_eq!(
            button_flags(MouseButton::X2, false),
            (MOUSEEVENTF_XUP, XBUTTON2)
        );
    }

    #[test]
    fn extended_scan() {
        let k = scan_key(0xE04D, true);
        assert_eq!(k.scan, 0x4D);
        assert_eq!(k.flags, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_SCANCODE);
        let k = scan_key(0x1E, false);
        assert_eq!(k.flags, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP);
    }

    #[test]
    fn pause_sequence() {
        let k = |scan, flags| {
            RawInput::Key(RawKey {
                vk: 0,
                scan,
                flags: KEYEVENTF_SCANCODE | flags,
            })
        };
        let d = event_inputs(
            &InputEvent::Key {
                scan: 0xE145,
                down: true,
            },
            R,
        );
        assert_eq!(d.as_slice(), &[k(0xE1, 0), k(0x1D, 0), k(0x45, 0)]);
        let u = event_inputs(
            &InputEvent::Key {
                scan: 0xE145,
                down: false,
            },
            R,
        );
        assert_eq!(
            u.as_slice(),
            &[
                k(0xE1, 0),
                k(0x1D, KEYEVENTF_KEYUP),
                k(0x45, KEYEVENTF_KEYUP)
            ]
        );
    }

    #[test]
    fn media_keys() {
        assert_eq!(media_vk(MediaKey::PlayPause), 0xB3);
        assert_eq!(media_vk(MediaKey::Mute), 0xAD);
        assert_eq!(media_key(MediaKey::Next, false).flags, KEYEVENTF_KEYUP);
    }

    #[test]
    fn text_surrogates() {
        let t = text_inputs("olá 😀");
        let units: Vec<u16> = "olá 😀".encode_utf16().collect();
        assert_eq!(units.len(), 6); // o l á ' ' + par de surrogates
        assert_eq!(t.len(), 12);
        for (i, u) in units.iter().enumerate() {
            let RawInput::Key(d) = t[2 * i] else { panic!() };
            let RawInput::Key(up) = t[2 * i + 1] else {
                panic!()
            };
            assert_eq!((d.scan, d.flags), (*u, KEYEVENTF_UNICODE));
            assert_eq!(
                (up.scan, up.flags),
                (*u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)
            );
        }
        assert_eq!(units[4], 0xD83D);
        assert_eq!(units[5], 0xDE00);
    }

    #[test]
    fn wheel_hires() {
        let b = event_inputs(
            &InputEvent::Scroll {
                dx: -37,
                dy: 7,
                phase: ScrollPhase::None,
            },
            R,
        );
        assert_eq!(
            b.as_slice(),
            &[
                RawInput::Mouse(RawMouse {
                    dx: 0,
                    dy: 0,
                    data: 7,
                    flags: MOUSEEVENTF_WHEEL
                }),
                RawInput::Mouse(RawMouse {
                    dx: 0,
                    dy: 0,
                    data: (-37i32) as u32,
                    flags: MOUSEEVENTF_HWHEEL
                }),
            ]
        );
        let b = event_inputs(
            &InputEvent::Scroll {
                dx: 0,
                dy: 0,
                phase: ScrollPhase::None,
            },
            R,
        );
        assert!(b.as_slice().is_empty());
    }

    #[test]
    fn abs_and_rel_move() {
        let b = event_inputs(&InputEvent::MouseAbs { x: -1920, y: 0 }, R);
        assert_eq!(
            b.as_slice(),
            &[RawInput::Mouse(RawMouse {
                dx: 0,
                dy: 0,
                data: 0,
                flags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK
            })]
        );
        let b = event_inputs(&InputEvent::MouseMove { dx: -3, dy: 4 }, R);
        assert_eq!(
            b.as_slice(),
            &[RawInput::Mouse(RawMouse {
                dx: -3,
                dy: 4,
                data: 0,
                flags: MOUSEEVENTF_MOVE
            })]
        );
    }
}
