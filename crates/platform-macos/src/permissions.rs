//! dono: M2
//! Permissões (Acessibilidade, Monitoramento de Entrada) e atalho para os Ajustes. Stub do F2.

use relay_core::types::{PermissionKind, PermissionState, PermissionsStatus};

pub fn check_permissions() -> PermissionsStatus {
    PermissionsStatus {
        accessibility: PermissionState::Unknown,
        input_monitoring: PermissionState::Unknown,
    }
}

/// Abre `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` ou
/// `?Privacy_ListenEvent`.
pub fn open_permission_settings(_kind: PermissionKind) {}

/// Chama IOHIDRequestAccess.
pub fn request_input_monitoring() -> bool {
    false
}
