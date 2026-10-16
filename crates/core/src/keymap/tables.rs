//! dono: C5a
//! Tabelas keycode do macOS ↔ scan code do Windows (set 1, 0xE0xx = estendida) e teclas de mídia.
//! Stubs do F2: assinaturas.

use crate::types::MediaKey;

/// Keycode virtual do macOS (kVK_*) → scan code do Windows.
pub fn mac_to_scan(_mac_keycode: u16) -> Option<u16> {
    None
}

/// Inversa de `mac_to_scan`.
pub fn scan_to_mac(_scan: u16) -> Option<u16> {
    None
}

/// NX_KEYTYPE_* → `MediaKey`. Brilho devolve `None` (D21).
pub fn media_from_nx(_nx_keytype: u32) -> Option<MediaKey> {
    None
}

/// `MediaKey` → virtual-key do Windows (VK_VOLUME_UP etc.).
pub fn media_to_vk(_key: MediaKey) -> u16 {
    0
}
