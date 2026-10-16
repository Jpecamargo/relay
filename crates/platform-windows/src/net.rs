//! dono: W3
//! MAC address da interface principal. Stub do F2.

use relay_core::platform::PlatformError;

pub fn primary_mac_address() -> Result<[u8; 6], PlatformError> {
    Err(PlatformError::NotImplemented)
}
