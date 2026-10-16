//! dono: W3
//! Configuração de repetição de tecla do Windows (`SPI_GETKEYBOARDDELAY` / `SPI_GETKEYBOARDSPEED`).

use relay_core::platform::{KeyRepeat, PlatformError};

/// `SPI_GETKEYBOARDDELAY` (0..=3) → 250..=1000 ms, em passos de 250 ms.
pub fn delay_ms_from_spi(delay: u32) -> u32 {
    (delay.min(3) + 1) * 250
}

/// `SPI_GETKEYBOARDSPEED` (0..=31) → 2,5..=30 Hz, linear.
pub fn rate_hz_from_spi(speed: u32) -> f32 {
    2.5 + speed.min(31) as f32 * (27.5 / 31.0)
}

#[cfg(windows)]
pub fn keyboard_repeat() -> Result<KeyRepeat, PlatformError> {
    use std::ffi::c_void;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        SystemParametersInfoW,
    };

    let query = |action| -> Result<u32, PlatformError> {
        let mut value = 0u32;
        // SAFETY: `value` vive até o fim da chamada e é do tamanho que esses dois SPI escrevem.
        unsafe {
            SystemParametersInfoW(
                action,
                0,
                Some(&mut value as *mut u32 as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .map_err(|e| PlatformError::Platform(format!("SystemParametersInfoW: {e}")))?;
        Ok(value)
    };
    Ok(KeyRepeat {
        delay_ms: delay_ms_from_spi(query(SPI_GETKEYBOARDDELAY)?),
        rate_hz: rate_hz_from_spi(query(SPI_GETKEYBOARDSPEED)?),
    })
}

#[cfg(not(windows))]
pub fn keyboard_repeat() -> Result<KeyRepeat, PlatformError> {
    Err(PlatformError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_cobre_250_a_1000() {
        let v: Vec<u32> = (0..=3).map(delay_ms_from_spi).collect();
        assert_eq!(v, [250, 500, 750, 1000]);
        assert_eq!(delay_ms_from_spi(9), 1000);
    }

    #[test]
    fn velocidade_cobre_2_5_a_30_hz() {
        assert_eq!(rate_hz_from_spi(0), 2.5);
        assert!((rate_hz_from_spi(31) - 30.0).abs() < 1e-4);
        assert!(rate_hz_from_spi(16) > rate_hz_from_spi(15));
        assert_eq!(rate_hz_from_spi(99), rate_hz_from_spi(31));
    }
}
