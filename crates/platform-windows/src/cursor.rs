//! dono: W2
//! Controle do cursor do Windows (preso em um centro enquanto o Mac controla): ClipCursor de 1 px
//! no centro + SetCursorPos de volta (feito pelo hook), cursor escondido.

use relay_core::platform::{CursorControl, PlatformError};

#[derive(Debug, Default)]
pub struct WindowsCursor {
    hidden: bool,
    locked: bool,
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        ClipCursor, CreateCursor, GetCursorPos, SPI_SETCURSORS, SYSTEM_CURSOR_ID,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursorPos, SetSystemCursor, SystemParametersInfoW,
    };

    use super::*;
    use crate::hooks::{set_lock, sync_last};

    /// Cursores padrão (OCR_*) trocados por um transparente enquanto escondido.
    const SYSTEM_CURSORS: [u32; 13] = [
        32512, 32513, 32514, 32515, 32516, 32642, 32643, 32644, 32645, 32646, 32648, 32649, 32650,
    ];

    fn err(e: windows::core::Error) -> PlatformError {
        PlatformError::Platform(e.to_string())
    }

    pub fn position() -> Result<(i32, i32), PlatformError> {
        let mut p = POINT::default();
        // SAFETY: ponteiro para POINT local.
        unsafe { GetCursorPos(&mut p) }.map_err(err)?;
        Ok((p.x, p.y))
    }

    pub fn warp(x: i32, y: i32) -> Result<(), PlatformError> {
        // SAFETY: chamada Win32 sem ponteiros.
        unsafe { SetCursorPos(x, y) }.map_err(err)
    }

    pub fn lock() -> Result<(), PlatformError> {
        let (x, y) = position()?;
        let rect = RECT {
            left: x,
            top: y,
            right: x + 1,
            bottom: y + 1,
        };
        // SAFETY: ponteiro para RECT local, copiado pelo sistema.
        unsafe { ClipCursor(Some(&rect)) }.map_err(err)?;
        set_lock(Some((x, y)));
        Ok(())
    }

    pub fn unlock() -> Result<(), PlatformError> {
        set_lock(None);
        // SAFETY: `None` remove o clip.
        unsafe { ClipCursor(None) }.map_err(err)?;
        if let Ok((x, y)) = position() {
            sync_last(x, y);
        }
        Ok(())
    }

    pub fn hide() -> Result<(), PlatformError> {
        // Máscara AND toda 1 + XOR 0 = transparente (cursor 32x32, 1 bpp).
        let and = [0xFFu8; 128];
        let xor = [0u8; 128];
        for id in SYSTEM_CURSORS {
            // SAFETY: máscaras de 32x32 1bpp (128 bytes) vivem durante a chamada; o
            // SetSystemCursor assume a posse do cursor criado.
            unsafe {
                let cursor =
                    CreateCursor(None, 0, 0, 32, 32, and.as_ptr().cast(), xor.as_ptr().cast())
                        .map_err(err)?;
                SetSystemCursor(cursor, SYSTEM_CURSOR_ID(id)).map_err(err)?;
            }
        }
        Ok(())
    }

    pub fn show() -> Result<(), PlatformError> {
        // SAFETY: recarrega os cursores do usuário a partir do registro.
        unsafe {
            SystemParametersInfoW(
                SPI_SETCURSORS,
                0,
                None,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .map_err(err)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn position() -> Result<(i32, i32), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
    pub fn warp(_x: i32, _y: i32) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
    pub fn lock() -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
    pub fn unlock() -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
    pub fn hide() -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
    pub fn show() -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}

impl CursorControl for WindowsCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        if !self.hidden {
            imp::hide()?;
            self.hidden = true;
        }
        Ok(())
    }

    fn show(&mut self) -> Result<(), PlatformError> {
        if self.hidden {
            imp::show()?;
            self.hidden = false;
        }
        Ok(())
    }

    /// `false` prende o cursor num centro (clip de 1 px) para medir deltas; `true` solta.
    fn associate(&mut self, associated: bool) -> Result<(), PlatformError> {
        if associated == !self.locked {
            return Ok(());
        }
        if associated {
            imp::unlock()?;
        } else {
            imp::lock()?;
        }
        self.locked = !associated;
        Ok(())
    }

    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        imp::warp(x, y)
    }

    fn position(&self) -> Result<(i32, i32), PlatformError> {
        imp::position()
    }
}

impl Drop for WindowsCursor {
    fn drop(&mut self) {
        let _ = self.show();
        let _ = self.associate(true);
    }
}
