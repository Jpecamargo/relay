//! dono: M3
//! Leitura do Dock e dos Hot Corners (com.apple.dock). Stub do F2.
//! `DockInfo` e `HotCorners` vivem em `relay_core::types` (consumidos por `layout::dock_warnings`).

use relay_core::types::{DockInfo, DockOrientation, HotCorners};

pub fn read_dock_info() -> (DockInfo, HotCorners) {
    (
        DockInfo {
            autohide: false,
            orientation: DockOrientation::Bottom,
        },
        HotCorners::default(),
    )
}
