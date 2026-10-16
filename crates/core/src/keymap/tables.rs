//! dono: C5a
//! Tabelas keycode do macOS ↔ scan code do Windows (set 1, 0xE0xx = estendida) e teclas de mídia.

use crate::types::MediaKey;

/// Pares (kVK_* do macOS, scan code set 1 do Windows). Todas as entradas são bijetivas.
///
/// Teclas que só existem no Windows têm origem no Mac: F13→PrintScreen, F14→ScrollLock,
/// F15→Pause (0xE145, marcador: no fio real é a sequência E1 1D 45), Help→Insert e a tecla de
/// menu (ContextMenu). `kVK_Function` (0x3F) não é mapeada.
const TABLE: &[(u16, u16)] = &[
    // Letras ANSI
    (0x00, 0x1E), // A
    (0x01, 0x1F), // S
    (0x02, 0x20), // D
    (0x03, 0x21), // F
    (0x04, 0x23), // H
    (0x05, 0x22), // G
    (0x06, 0x2C), // Z
    (0x07, 0x2D), // X
    (0x08, 0x2E), // C
    (0x09, 0x2F), // V
    (0x0B, 0x30), // B
    (0x0C, 0x10), // Q
    (0x0D, 0x11), // W
    (0x0E, 0x12), // E
    (0x0F, 0x13), // R
    (0x10, 0x15), // Y
    (0x11, 0x14), // T
    (0x1F, 0x18), // O
    (0x20, 0x16), // U
    (0x22, 0x17), // I
    (0x23, 0x19), // P
    (0x25, 0x26), // L
    (0x26, 0x24), // J
    (0x28, 0x25), // K
    (0x2D, 0x31), // N
    (0x2E, 0x32), // M
    // Dígitos
    (0x12, 0x02), // 1
    (0x13, 0x03), // 2
    (0x14, 0x04), // 3
    (0x15, 0x05), // 4
    (0x17, 0x06), // 5
    (0x16, 0x07), // 6
    (0x1A, 0x08), // 7
    (0x1C, 0x09), // 8
    (0x19, 0x0A), // 9
    (0x1D, 0x0B), // 0
    // Pontuação ANSI
    (0x1B, 0x0C), // Minus
    (0x18, 0x0D), // Equal
    (0x21, 0x1A), // LeftBracket
    (0x1E, 0x1B), // RightBracket
    (0x29, 0x27), // Semicolon
    (0x27, 0x28), // Quote
    (0x2A, 0x2B), // Backslash
    (0x2B, 0x33), // Comma
    (0x2F, 0x34), // Period
    (0x2C, 0x35), // Slash
    (0x32, 0x29), // Grave
    // ISO e JIS
    (0x0A, 0x56), // ISO_Section
    (0x5D, 0x7D), // JIS_Yen
    (0x5E, 0x73), // JIS_Underscore (tecla /? do ABNT2)
    (0x5F, 0x7E), // JIS_KeypadComma (vírgula do keypad ABNT2)
    (0x66, 0x7B), // JIS_Eisu (Muhenkan)
    (0x68, 0x70), // JIS_Kana (Katakana/Hiragana)
    // Controle
    (0x24, 0x1C), // Return
    (0x30, 0x0F), // Tab
    (0x31, 0x39), // Space
    (0x33, 0x0E), // Delete (backspace)
    (0x35, 0x01), // Escape
    (0x39, 0x3A), // CapsLock
    // Modificadores L/R
    (0x37, 0xE05B), // Command
    (0x36, 0xE05C), // RightCommand
    (0x38, 0x2A),   // Shift
    (0x3C, 0x36),   // RightShift
    (0x3A, 0x38),   // Option
    (0x3D, 0xE038), // RightOption
    (0x3B, 0x1D),   // Control
    (0x3E, 0xE01D), // RightControl
    // Teclas de função
    (0x7A, 0x3B),   // F1
    (0x78, 0x3C),   // F2
    (0x63, 0x3D),   // F3
    (0x76, 0x3E),   // F4
    (0x60, 0x3F),   // F5
    (0x61, 0x40),   // F6
    (0x62, 0x41),   // F7
    (0x64, 0x42),   // F8
    (0x65, 0x43),   // F9
    (0x6D, 0x44),   // F10
    (0x67, 0x57),   // F11
    (0x6F, 0x58),   // F12
    (0x69, 0xE037), // F13 → PrintScreen
    (0x6B, 0x46),   // F14 → ScrollLock
    (0x71, 0xE145), // F15 → Pause
    (0x6A, 0x67),   // F16
    (0x40, 0x68),   // F17
    (0x4F, 0x69),   // F18
    (0x50, 0x6A),   // F19
    (0x5A, 0x6B),   // F20
    // Navegação
    (0x72, 0xE052), // Help → Insert
    (0x75, 0xE053), // ForwardDelete
    (0x73, 0xE047), // Home
    (0x77, 0xE04F), // End
    (0x74, 0xE049), // PageUp
    (0x79, 0xE051), // PageDown
    (0x7B, 0xE04B), // Left
    (0x7C, 0xE04D), // Right
    (0x7D, 0xE050), // Down
    (0x7E, 0xE048), // Up
    (0x6E, 0xE05D), // ContextMenu
    // Keypad
    (0x52, 0x52),   // 0
    (0x53, 0x4F),   // 1
    (0x54, 0x50),   // 2
    (0x55, 0x51),   // 3
    (0x56, 0x4B),   // 4
    (0x57, 0x4C),   // 5
    (0x58, 0x4D),   // 6
    (0x59, 0x47),   // 7
    (0x5B, 0x48),   // 8
    (0x5C, 0x49),   // 9
    (0x41, 0x53),   // Decimal
    (0x43, 0x37),   // Multiply
    (0x45, 0x4E),   // Plus
    (0x47, 0x45),   // Clear → NumLock
    (0x4B, 0xE035), // Divide
    (0x4C, 0xE01C), // Enter
    (0x4E, 0x4A),   // Minus
    (0x51, 0x59),   // Equals
];

/// Keycode virtual do macOS (kVK_*) → scan code do Windows.
pub fn mac_to_scan(mac_keycode: u16) -> Option<u16> {
    TABLE
        .iter()
        .find(|(m, _)| *m == mac_keycode)
        .map(|(_, s)| *s)
}

/// Inversa de `mac_to_scan`.
pub fn scan_to_mac(scan: u16) -> Option<u16> {
    TABLE.iter().find(|(_, s)| *s == scan).map(|(m, _)| *m)
}

const NX_KEYTYPE_SOUND_UP: u32 = 0;
const NX_KEYTYPE_SOUND_DOWN: u32 = 1;
const NX_KEYTYPE_MUTE: u32 = 7;
const NX_KEYTYPE_PLAY: u32 = 16;
const NX_KEYTYPE_NEXT: u32 = 17;
const NX_KEYTYPE_PREVIOUS: u32 = 18;

/// NX_KEYTYPE_* → `MediaKey`. Brilho devolve `None` (D21).
pub fn media_from_nx(nx_keytype: u32) -> Option<MediaKey> {
    match nx_keytype {
        NX_KEYTYPE_SOUND_UP => Some(MediaKey::VolumeUp),
        NX_KEYTYPE_SOUND_DOWN => Some(MediaKey::VolumeDown),
        NX_KEYTYPE_MUTE => Some(MediaKey::Mute),
        NX_KEYTYPE_PLAY => Some(MediaKey::PlayPause),
        NX_KEYTYPE_NEXT => Some(MediaKey::Next),
        NX_KEYTYPE_PREVIOUS => Some(MediaKey::Previous),
        _ => None,
    }
}

/// `MediaKey` → virtual-key do Windows (VK_VOLUME_UP etc.).
pub fn media_to_vk(key: MediaKey) -> u16 {
    match key {
        MediaKey::VolumeUp => 0xAF,   // VK_VOLUME_UP
        MediaKey::VolumeDown => 0xAE, // VK_VOLUME_DOWN
        MediaKey::Mute => 0xAD,       // VK_VOLUME_MUTE
        MediaKey::PlayPause => 0xB3,  // VK_MEDIA_PLAY_PAUSE
        MediaKey::Next => 0xB0,       // VK_MEDIA_NEXT_TRACK
        MediaKey::Previous => 0xB1,   // VK_MEDIA_PREV_TRACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asserts_especificos() {
        let casos = [
            (0x00, 0x1E),
            (0x24, 0x1C),
            (0x7C, 0xE04D),
            (0x37, 0xE05B),
            (0x36, 0xE05C),
            (0x3A, 0x38),
            (0x3B, 0x1D),
            (0x0A, 0x56),
            (0x5E, 0x73),
            (0x5F, 0x7E),
            (0x41, 0x53),
            (0x75, 0xE053),
            (0x69, 0xE037),
            (0x6B, 0x46),
            (0x72, 0xE052),
        ];
        for (mac, scan) in casos {
            assert_eq!(mac_to_scan(mac), Some(scan), "mac {mac:#x}");
        }
        assert!(mac_to_scan(0x71).is_some()); // F15 → Pause
        assert!(mac_to_scan(0x6E).is_some()); // menu
        assert_eq!(mac_to_scan(0x3F), None); // kVK_Function
    }

    #[test]
    fn tamanho_minimo() {
        assert!(TABLE.len() >= 100);
    }

    #[test]
    fn round_trip_bijetivo() {
        for &(mac, scan) in TABLE {
            assert_eq!(mac_to_scan(mac), Some(scan));
            assert_eq!(scan_to_mac(scan), Some(mac), "scan {scan:#x}");
        }
        assert_eq!(scan_to_mac(0xFFFF), None);
    }

    #[test]
    fn sem_duplicatas() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!(a.0, b.0, "mac duplicado {:#x}", a.0);
                assert_ne!(a.1, b.1, "scan duplicado {:#x}", a.1);
            }
        }
    }

    #[test]
    fn midia() {
        assert_eq!(media_from_nx(0), Some(MediaKey::VolumeUp));
        assert_eq!(media_from_nx(1), Some(MediaKey::VolumeDown));
        assert_eq!(media_from_nx(7), Some(MediaKey::Mute));
        assert_eq!(media_from_nx(16), Some(MediaKey::PlayPause));
        assert_eq!(media_from_nx(17), Some(MediaKey::Next));
        assert_eq!(media_from_nx(18), Some(MediaKey::Previous));
        // brilho (2, 3) e teclado iluminado (21..23) não são enviados (D21)
        for nx in [2, 3, 21, 22, 23, 99] {
            assert_eq!(media_from_nx(nx), None);
        }
        assert_eq!(media_to_vk(MediaKey::VolumeUp), 0xAF);
        assert_eq!(media_to_vk(MediaKey::VolumeDown), 0xAE);
        assert_eq!(media_to_vk(MediaKey::Mute), 0xAD);
        assert_eq!(media_to_vk(MediaKey::PlayPause), 0xB3);
        assert_eq!(media_to_vk(MediaKey::Next), 0xB0);
        assert_eq!(media_to_vk(MediaKey::Previous), 0xB1);
    }
}
