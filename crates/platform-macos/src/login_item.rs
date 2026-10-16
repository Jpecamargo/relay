//! dono: M3
//! Login item via SMAppService.mainApp.

use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use relay_core::platform::PlatformError;

#[derive(Debug, Default)]
pub struct LoginItem {}

/// `SMAppService.mainApp` só funciona dentro de um bundle assinado; fora dele devolve erro.
fn ensure_bundle() -> Result<(), PlatformError> {
    let in_bundle = NSBundle::mainBundle().bundleIdentifier().is_some();
    if in_bundle {
        Ok(())
    } else {
        Err(PlatformError::Platform(
            "login item exige execução dentro de um app bundle".into(),
        ))
    }
}

impl LoginItem {
    pub fn register(&self) -> Result<(), PlatformError> {
        ensure_bundle()?;
        // SAFETY: mainAppService não tem pré-condições; chamada a partir de qualquer thread.
        let service = unsafe { SMAppService::mainAppService() };
        unsafe { service.registerAndReturnError() }
            .map_err(|e| PlatformError::Platform(e.localizedDescription().to_string()))
    }

    pub fn unregister(&self) -> Result<(), PlatformError> {
        ensure_bundle()?;
        let service = unsafe { SMAppService::mainAppService() };
        unsafe { service.unregisterAndReturnError() }
            .map_err(|e| PlatformError::Platform(e.localizedDescription().to_string()))
    }

    /// `true` só quando o status é `Enabled` (`RequiresApproval` conta como não registrado).
    pub fn is_registered(&self) -> Result<bool, PlatformError> {
        ensure_bundle()?;
        let service = unsafe { SMAppService::mainAppService() };
        Ok(unsafe { service.status() } == SMAppServiceStatus::Enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_bundle_errors_without_panic() {
        let item = LoginItem::default();
        assert!(item.register().is_err());
        assert!(item.unregister().is_err());
        assert!(item.is_registered().is_err());
    }
}
