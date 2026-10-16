//! dono: M3
//! Leitura de telas com notificação de mudança (CoreGraphics).

use std::ffi::c_void;
use std::sync::{Arc, Mutex, Once};

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation::uuid::{CFUUIDCreateString, CFUUIDRef};
use core_graphics::display::{CGDirectDisplayID, CGDisplay};
use relay_core::platform::{ChangeCallback, DisplaySource, PlatformError};
use relay_core::types::{DisplayInfo, Machine};

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

const BEGIN_CONFIGURATION_FLAG: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    fn CGDisplayScreenSize(display: CGDirectDisplayID) -> CGSize;
    fn CGDisplayIsMain(display: CGDirectDisplayID) -> u32;
    fn CGDisplayCreateUUIDFromDisplayID(display: CGDirectDisplayID) -> CFUUIDRef;
    fn CGDisplayRegisterReconfigurationCallback(
        callback: unsafe extern "C" fn(CGDirectDisplayID, u32, *const c_void),
        user_info: *const c_void,
    ) -> i32;
}

/// `pixelWidth / width` do modo atual; 1.0 se algum valor for 0.
pub(crate) fn backing_scale(pixel_width: u64, width: u64) -> f32 {
    if pixel_width == 0 || width == 0 {
        1.0
    } else {
        pixel_width as f32 / width as f32
    }
}

fn display_uuid(id: CGDirectDisplayID) -> Option<String> {
    // SAFETY: devolve um CFUUID +1 (create rule) ou NULL.
    let uuid = unsafe { CGDisplayCreateUUIDFromDisplayID(id) };
    if uuid.is_null() {
        return None;
    }
    // SAFETY: `uuid` é válido; a CFString devolvida segue a create rule.
    let s = unsafe { CFUUIDCreateString(std::ptr::null(), uuid) };
    // Libera o CFUUID embrulhando-o numa CFType.
    drop(unsafe { core_foundation::base::CFType::wrap_under_create_rule(uuid as _) });
    if s.is_null() {
        return None;
    }
    Some(unsafe { CFString::wrap_under_create_rule(s) }.to_string())
}

fn active_ids() -> Result<Vec<CGDirectDisplayID>, PlatformError> {
    let mut ids = [0u32; 32];
    let mut count = 0u32;
    // SAFETY: buffer com 32 posições, `max` = 32.
    let err = unsafe { CGGetActiveDisplayList(32, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Err(PlatformError::Platform(format!(
            "CGGetActiveDisplayList falhou ({err})"
        )));
    }
    Ok(ids[..count as usize].to_vec())
}

fn describe(id: CGDirectDisplayID) -> DisplayInfo {
    let display = CGDisplay::new(id);
    let bounds = display.bounds();
    let (pw, w) = display
        .display_mode()
        .map(|m| (m.pixel_width(), m.width()))
        .unwrap_or((0, 0));
    // SAFETY: funções puras de consulta sobre um id ativo.
    let (size, is_main) = unsafe { (CGDisplayScreenSize(id), CGDisplayIsMain(id) != 0) };
    let size_mm =
        (size.width > 0.0 && size.height > 0.0).then_some((size.width as f32, size.height as f32));
    let is_builtin = display.is_builtin();
    let name = if is_builtin {
        "Built-in Display".to_string()
    } else {
        format!("Display {id}")
    };
    DisplayInfo {
        id: display_uuid(id).unwrap_or_else(|| format!("display-{id}")),
        name,
        machine: Machine::Mac,
        origin_px: (bounds.origin.x as i32, bounds.origin.y as i32),
        size_px: (bounds.size.width as u32, bounds.size.height as u32),
        scale: backing_scale(pw, w),
        size_mm,
        is_primary: is_main,
        is_builtin,
    }
}

type SharedCallback = Arc<dyn Fn() + Send + Sync>;

static CALLBACK: Mutex<Option<SharedCallback>> = Mutex::new(None);
static REGISTER: Once = Once::new();

unsafe extern "C" fn on_reconfigure(_display: CGDirectDisplayID, flags: u32, _user: *const c_void) {
    // O sistema chama antes e depois da mudança; só o "depois" interessa.
    if flags & BEGIN_CONFIGURATION_FLAG != 0 {
        return;
    }
    let cb = CALLBACK.lock().ok().and_then(|g| g.clone());
    if let Some(cb) = cb {
        cb();
    }
}

#[derive(Debug, Default)]
pub struct MacDisplays {}

impl DisplaySource for MacDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(active_ids()?.into_iter().map(describe).collect())
    }

    /// O callback roda na thread que processa o run loop principal (exige run loop ativo).
    /// Há um callback global: registrar em outra instância substitui o anterior.
    fn set_change_callback(&mut self, callback: ChangeCallback) {
        if let Ok(mut g) = CALLBACK.lock() {
            *g = Some(Arc::from(callback));
        }
        REGISTER.call_once(|| {
            // SAFETY: função `extern "C"` 'static; não usa user_info.
            unsafe { CGDisplayRegisterReconfigurationCallback(on_reconfigure, std::ptr::null()) };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_from_mode() {
        assert_eq!(backing_scale(3024, 1512), 2.0);
        assert_eq!(backing_scale(1920, 1920), 1.0);
        assert_eq!(backing_scale(0, 100), 1.0);
        assert_eq!(backing_scale(100, 0), 1.0);
    }

    // Precisa de ao menos uma tela ativa (sessão gráfica); ignorado em CI sem display.
    #[test]
    #[ignore = "depende de hardware/sessão gráfica; rodar com --ignored neste Mac"]
    fn lists_real_displays() {
        let list = MacDisplays::default().list().unwrap();
        assert!(!list.is_empty());
        assert!(
            list.iter()
                .any(|d| d.size_mm.is_some_and(|(w, h)| w > 0.0 && h > 0.0))
        );
        assert!(list.iter().all(|d| !d.id.is_empty() && d.size_px.0 > 0));
        assert_eq!(list.iter().filter(|d| d.is_primary).count(), 1);
    }
}
