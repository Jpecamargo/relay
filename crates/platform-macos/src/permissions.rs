//! dono: M2
//! Permissões (Acessibilidade, Monitoramento de Entrada) e atalho para os Ajustes.

use relay_core::types::{PermissionKind, PermissionState, PermissionsStatus};

/// `kIOHIDRequestTypeListenEvent`.
const IOHID_REQUEST_LISTEN_EVENT: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
    fn IOHIDRequestAccess(request_type: u32) -> bool;
}

/// URL do painel de Privacidade e Segurança certo para cada permissão.
pub fn settings_url(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PermissionKind::InputMonitoring => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
    }
}

/// `IOHIDAccessType`: 0 = granted, 1 = denied, 2 (ou qualquer outro) = unknown.
pub fn input_monitoring_state(raw: u32) -> PermissionState {
    match raw {
        0 => PermissionState::Granted,
        1 => PermissionState::Denied,
        _ => PermissionState::Unknown,
    }
}

pub fn check_permissions() -> PermissionsStatus {
    // SAFETY: chamadas sem ponteiros.
    let (ax, im) = unsafe {
        (
            AXIsProcessTrusted(),
            IOHIDCheckAccess(IOHID_REQUEST_LISTEN_EVENT),
        )
    };
    PermissionsStatus {
        accessibility: if ax {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        },
        input_monitoring: input_monitoring_state(im),
    }
}

/// Abre o painel de Ajustes da permissão (`Privacy_Accessibility` ou `Privacy_ListenEvent`).
pub fn open_permission_settings(kind: PermissionKind) {
    let _ = std::process::Command::new("open")
        .arg(settings_url(kind))
        .spawn();
}

/// Chama IOHIDRequestAccess (mostra o diálogo do sistema na primeira vez).
pub fn request_input_monitoring() -> bool {
    // SAFETY: chamada sem ponteiros.
    unsafe { IOHIDRequestAccess(IOHID_REQUEST_LISTEN_EVENT) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_exatas() {
        assert_eq!(
            settings_url(PermissionKind::Accessibility),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
        assert_eq!(
            settings_url(PermissionKind::InputMonitoring),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        );
    }

    #[test]
    fn mapeia_iohid_access() {
        assert_eq!(input_monitoring_state(0), PermissionState::Granted);
        assert_eq!(input_monitoring_state(1), PermissionState::Denied);
        assert_eq!(input_monitoring_state(2), PermissionState::Unknown);
        assert_eq!(input_monitoring_state(99), PermissionState::Unknown);
    }

    #[test]
    fn check_permissions_nao_da_panic() {
        let _ = check_permissions();
    }

    /// Abre os Ajustes e (na 1ª vez) mostra o diálogo: só à mão.
    #[test]
    #[ignore = "abre os Ajustes do Sistema e pode mostrar diálogo de permissão"]
    fn atalhos_reais() {
        open_permission_settings(PermissionKind::InputMonitoring);
        let _ = request_input_monitoring();
    }
}
