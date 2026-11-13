//! dono: M3
//! Leitura de telas com notificação de mudança (CoreGraphics).

use std::ffi::c_void;
use std::sync::{Arc, Mutex, Once};

use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberGetTypeID, CFNumberRef};
use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
use core_foundation::uuid::{CFUUIDCreateString, CFUUIDRef};
use core_graphics::display::{CGDirectDisplayID, CGDisplay};
use relay_core::layout::display_identity;
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
    fn CGDisplayVendorNumber(display: CGDirectDisplayID) -> u32;
    fn CGDisplayModelNumber(display: CGDirectDisplayID) -> u32;
    fn CGDisplaySerialNumber(display: CGDirectDisplayID) -> u32;
    fn CGDisplayRegisterReconfigurationCallback(
        callback: unsafe extern "C" fn(CGDirectDisplayID, u32, *const c_void),
        user_info: *const c_void,
    ) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const std::ffi::c_char) -> CFDictionaryRef;
    fn IOServiceGetMatchingServices(main_port: u32, matching: CFDictionaryRef, it: *mut u32)
    -> i32;
    fn IOIteratorNext(it: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CFStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CFTypeRef;
}

/// Atributos do monitor que o framebuffer publica no IORegistry (Apple Silicon):
/// `(fabricante, modelo, série numérica, série alfanumérica)`.
type ProductAttributes = (u32, u32, u32, String);

/// Valor de `key` em `dict` se for do tipo `type_id`; ponteiro emprestado (get rule).
unsafe fn dict_value(dict: CFDictionaryRef, key: &str, type_id: usize) -> Option<*const c_void> {
    let key = CFString::new(key);
    // SAFETY: `dict` é um CFDictionary válido durante a chamada.
    let v = unsafe { CFDictionaryGetValue(dict, key.as_concrete_TypeRef() as *const c_void) };
    (!v.is_null() && unsafe { CFGetTypeID(v) } == type_id).then_some(v)
}

unsafe fn dict_u32(dict: CFDictionaryRef, key: &str) -> Option<u32> {
    let v = unsafe { dict_value(dict, key, CFNumberGetTypeID()) }?;
    // SAFETY: tipo conferido; get rule.
    let n = unsafe { CFNumber::wrap_under_get_rule(v as CFNumberRef) };
    n.to_i64().map(|n| n as u32)
}

/// `ProductAttributes` de cada `IOMobileFramebuffer` que tem série alfanumérica. Vazio em Mac
/// Intel (outra classe) ou se o IOKit falhar.
fn framebuffer_products() -> Vec<ProductAttributes> {
    let mut out = Vec::new();
    // SAFETY: nome terminado em NUL; o dicionário é consumido por IOServiceGetMatchingServices.
    let matching = unsafe { IOServiceMatching(c"IOMobileFramebuffer".as_ptr()) };
    if matching.is_null() {
        return out;
    }
    let mut it = 0u32;
    if unsafe { IOServiceGetMatchingServices(0, matching, &mut it) } != 0 {
        return out;
    }
    let attrs_key = CFString::new("DisplayAttributes");
    loop {
        // SAFETY: `it` é um iterador válido até o IOObjectRelease abaixo.
        let entry = unsafe { IOIteratorNext(it) };
        if entry == 0 {
            break;
        }
        // SAFETY: create rule; liberado com CFRelease.
        let attrs = unsafe {
            IORegistryEntryCreateCFProperty(
                entry,
                attrs_key.as_concrete_TypeRef(),
                std::ptr::null(),
                0,
            )
        };
        unsafe { IOObjectRelease(entry) };
        if attrs.is_null() {
            continue;
        }
        if unsafe { CFGetTypeID(attrs) } == unsafe { CFDictionaryGetTypeID() } {
            // SAFETY: `attrs` é um CFDictionary vivo até o CFRelease.
            let product = unsafe {
                dict_value(
                    attrs as CFDictionaryRef,
                    "ProductAttributes",
                    CFDictionaryGetTypeID(),
                )
            };
            if let Some(p) =
                product.and_then(|p| unsafe { product_attributes(p as CFDictionaryRef) })
            {
                out.push(p);
            }
        }
        unsafe { CFRelease(attrs) };
    }
    unsafe { IOObjectRelease(it) };
    out
}

unsafe fn product_attributes(p: CFDictionaryRef) -> Option<ProductAttributes> {
    let serial = unsafe { dict_value(p, "AlphanumericSerialNumber", CFStringGetTypeID()) }?;
    // SAFETY: tipo conferido; get rule.
    let serial = unsafe { CFString::wrap_under_get_rule(serial as CFStringRef) }.to_string();
    Some((
        unsafe { dict_u32(p, "LegacyManufacturerID") }?,
        unsafe { dict_u32(p, "ProductID") }?,
        unsafe { dict_u32(p, "SerialNumber") }.unwrap_or(0),
        serial,
    ))
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

    /// Telas externas ativas casadas com o framebuffer pelo fabricante, modelo e série numérica
    /// (os três vêm do mesmo EDID), com a série alfanumérica dele.
    fn identities(&self) -> Vec<(String, String)> {
        let Ok(ids) = active_ids() else {
            return Vec::new();
        };
        let products = framebuffer_products();
        ids.into_iter()
            .filter(|&id| !CGDisplay::new(id).is_builtin())
            .filter_map(|id| {
                // SAFETY: consultas puras sobre um id ativo.
                let key = unsafe {
                    (
                        CGDisplayVendorNumber(id),
                        CGDisplayModelNumber(id),
                        CGDisplaySerialNumber(id),
                    )
                };
                let (vendor, _, _, serial) =
                    products.iter().find(|(v, m, s, _)| (*v, *m, *s) == key)?;
                let identity = display_identity(*vendor as u16, serial)?;
                Some((
                    display_uuid(id).unwrap_or_else(|| format!("display-{id}")),
                    identity,
                ))
            })
            .collect()
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

    #[test]
    #[ignore = "depende de monitor externo com EDID; rodar com --ignored neste Mac"]
    fn identities_of_real_displays() {
        let ids = MacDisplays::default().identities();
        eprintln!("{ids:?}");
        assert!(ids.iter().all(|(_, i)| i.contains(':')));
    }
}
