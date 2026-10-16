//! dono: M3
//! Leitura do Dock e dos Hot Corners (com.apple.dock).
//! `DockInfo` e `HotCorners` vivem em `relay_core::types` (consumidos por `layout::dock_warnings`).

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use relay_core::types::{DockInfo, DockOrientation, HotCorners};

const DOCK_APP_ID: &str = "com.apple.dock";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFPreferencesCopyAppValue(
        key: CFStringRef,
        application_id: CFStringRef,
    ) -> core_foundation::base::CFTypeRef;
}

/// Valores crus de `com.apple.dock`; `None` = chave ausente (o padrão do sistema vale).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RawDock {
    pub autohide: Option<bool>,
    pub orientation: Option<String>,
    /// `wvous-tl/tr/bl/br-corner`: 0 e 1 = sem ação; >= 2 = ação configurada.
    pub corners: [Option<i64>; 4],
}

/// Converte os valores crus em `DockInfo`/`HotCorners`. Pura, testável sem o sistema.
pub(crate) fn parse_dock_info(raw: &RawDock) -> (DockInfo, HotCorners) {
    let orientation = match raw.orientation.as_deref() {
        Some("left") => DockOrientation::Left,
        Some("right") => DockOrientation::Right,
        _ => DockOrientation::Bottom,
    };
    let active = |i: usize| raw.corners[i].is_some_and(|v| v >= 2);
    (
        DockInfo {
            autohide: raw.autohide.unwrap_or(false),
            orientation,
        },
        HotCorners {
            top_left: active(0),
            top_right: active(1),
            bottom_left: active(2),
            bottom_right: active(3),
        },
    )
}

fn read_pref(key: &str) -> Option<CFType> {
    let key = CFString::new(key);
    let app = CFString::new(DOCK_APP_ID);
    // SAFETY: argumentos são CFStrings vivas; o retorno segue a create rule (ou NULL).
    let raw =
        unsafe { CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), app.as_concrete_TypeRef()) };
    if raw.is_null() {
        None
    } else {
        // SAFETY: não nulo e com +1 de retain (Copy).
        Some(unsafe { CFType::wrap_under_create_rule(raw) })
    }
}

fn read_raw() -> RawDock {
    let corner = |key: &str| {
        read_pref(key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
    };
    RawDock {
        autohide: read_pref("autohide")
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from),
        orientation: read_pref("orientation")
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string()),
        corners: [
            corner("wvous-tl-corner"),
            corner("wvous-tr-corner"),
            corner("wvous-bl-corner"),
            corner("wvous-br-corner"),
        ],
    }
}

/// Lê autohide, posição do Dock e Hot Corners ativos. Chave ausente = padrão do sistema.
pub fn read_dock_info() -> (DockInfo, HotCorners) {
    parse_dock_info(&read_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_keys_missing() {
        let (dock, corners) = parse_dock_info(&RawDock::default());
        assert!(!dock.autohide);
        assert_eq!(dock.orientation, DockOrientation::Bottom);
        assert_eq!(corners, HotCorners::default());
    }

    #[test]
    fn parses_example_values() {
        let raw = RawDock {
            autohide: Some(true),
            orientation: Some("left".into()),
            corners: [Some(2), Some(1), Some(0), Some(14)],
        };
        let (dock, corners) = parse_dock_info(&raw);
        assert!(dock.autohide);
        assert_eq!(dock.orientation, DockOrientation::Left);
        assert_eq!(
            corners,
            HotCorners {
                top_left: true,
                top_right: false,
                bottom_left: false,
                bottom_right: true
            }
        );
        let raw = RawDock {
            orientation: Some("right".into()),
            ..Default::default()
        };
        assert_eq!(parse_dock_info(&raw).0.orientation, DockOrientation::Right);
    }

    #[test]
    fn reads_system_without_panic() {
        let _ = read_dock_info();
    }
}
