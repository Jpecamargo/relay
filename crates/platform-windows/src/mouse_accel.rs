//! dono: W1
//! Desliga e restaura a "Aprimorar precisão do ponteiro" (D27).

/// `SPI_GETMOUSE`: [limiar1, limiar2, aceleração]. Aceleração 0 = precisão do ponteiro desligada.
pub type MouseParams = [i32; 3];

/// Parâmetros com a aceleração zerada, preservando os limiares.
pub fn without_acceleration(saved: MouseParams) -> MouseParams {
    [saved[0], saved[1], 0]
}

/// Salva SPI_GETMOUSE, zera a aceleração e restaura no `Drop`. Sem efeito fora do Windows.
/// Não persiste no perfil do usuário (sem SPIF_UPDATEINIFILE).
#[derive(Debug, Default)]
pub struct PointerPrecisionGuard {
    saved: Option<MouseParams>,
}

impl PointerPrecisionGuard {
    /// Desliga a aceleração; se não conseguir ler/gravar, devolve um guard inerte.
    pub fn engage() -> Self {
        #[cfg(windows)]
        if let Some(saved) = win::get() {
            if saved[2] == 0 || win::set(without_acceleration(saved)) {
                return Self { saved: Some(saved) };
            }
        }
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.saved.is_some()
    }
}

impl Drop for PointerPrecisionGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(saved) = self.saved.take() {
            win::set(saved);
        }
    }
}

#[cfg(windows)]
mod win {
    use super::MouseParams;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETMOUSE, SPI_SETMOUSE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };

    pub fn get() -> Option<MouseParams> {
        let mut p: MouseParams = [0; 3];
        // SAFETY: SPI_GETMOUSE escreve 3 inteiros no buffer apontado.
        unsafe {
            SystemParametersInfoW(
                SPI_GETMOUSE,
                0,
                Some(p.as_mut_ptr().cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .ok()
        .map(|()| p)
    }

    pub fn set(mut p: MouseParams) -> bool {
        // SAFETY: SPI_SETMOUSE lê 3 inteiros do buffer apontado.
        unsafe {
            SystemParametersInfoW(
                SPI_SETMOUSE,
                0,
                Some(p.as_mut_ptr().cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroes_only_acceleration() {
        assert_eq!(without_acceleration([6, 10, 1]), [6, 10, 0]);
        assert_eq!(without_acceleration([0, 0, 2]), [0, 0, 0]);
    }

    #[test]
    fn default_guard_is_inert() {
        assert!(!PointerPrecisionGuard::default().is_active());
    }
}
