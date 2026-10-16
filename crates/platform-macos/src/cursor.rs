//! dono: M2
//! Controle de cursor (esconder, prender, warp) via CoreGraphics.

use std::ffi::c_void;

use core_graphics::geometry::CGPoint;
use core_graphics::sys::CGEventRef;
use relay_core::platform::{CursorControl, PlatformError};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGWarpMouseCursorPosition(p: CGPoint) -> i32;
    fn CGSetLocalEventsSuppressionInterval(seconds: f64) -> i32;
    fn CGEventCreate(source: *mut c_void) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
}

fn check(op: &str, err: i32) -> Result<(), PlatformError> {
    if err == 0 {
        Ok(())
    } else {
        Err(PlatformError::Platform(format!(
            "{op} falhou (CGError {err})"
        )))
    }
}

/// Cursor local do Mac. `hide`/`show` são idempotentes (o CG conta chamadas aninhadas, então
/// só chamamos quando o estado muda).
#[derive(Debug, Default)]
pub struct MacCursor {
    hidden: bool,
}

impl CursorControl for MacCursor {
    fn hide(&mut self) -> Result<(), PlatformError> {
        if self.hidden {
            return Ok(());
        }
        // SAFETY: chamadas CG sem ponteiros.
        check("CGDisplayHideCursor", unsafe {
            CGDisplayHideCursor(CGMainDisplayID())
        })?;
        self.hidden = true;
        Ok(())
    }

    fn show(&mut self) -> Result<(), PlatformError> {
        if !self.hidden {
            return Ok(());
        }
        // SAFETY: idem.
        check("CGDisplayShowCursor", unsafe {
            CGDisplayShowCursor(CGMainDisplayID())
        })?;
        self.hidden = false;
        Ok(())
    }

    fn associate(&mut self, associated: bool) -> Result<(), PlatformError> {
        // SAFETY: idem.
        check("CGAssociateMouseAndMouseCursorPosition", unsafe {
            CGAssociateMouseAndMouseCursorPosition(associated)
        })
    }

    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError> {
        // Sem isto o CG ignora os eventos do mouse por ~250 ms depois do warp.
        // SAFETY: idem.
        unsafe { CGSetLocalEventsSuppressionInterval(0.0) };
        check("CGWarpMouseCursorPosition", unsafe {
            CGWarpMouseCursorPosition(CGPoint::new(f64::from(x), f64::from(y)))
        })
    }

    fn position(&self) -> Result<(i32, i32), PlatformError> {
        // SAFETY: CGEventCreate(NULL) devolve um evento com a posição atual; liberado abaixo.
        let ev = unsafe { CGEventCreate(std::ptr::null_mut()) };
        if ev.is_null() {
            return Err(PlatformError::Platform(
                "CGEventCreate devolveu nulo".into(),
            ));
        }
        // SAFETY: `ev` válido; CFRelease devolve a posse.
        let p = unsafe { CGEventGetLocation(ev) };
        unsafe { core_foundation::base::CFRelease(ev as *const c_void) };
        Ok((p.x.round() as i32, p.y.round() as i32))
    }
}

impl Drop for MacCursor {
    fn drop(&mut self) {
        // Nunca deixar o cursor escondido para trás.
        let _ = self.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_nao_da_panic_e_hide_show_sao_idempotentes() {
        let mut c = MacCursor::default();
        assert!(c.position().is_ok());
        // show sem hide é no-op
        assert!(c.show().is_ok());
    }

    #[test]
    fn erro_do_cg_vira_platform_error() {
        assert!(check("x", 0).is_ok());
        assert!(matches!(check("x", 1000), Err(PlatformError::Platform(_))));
    }
}
