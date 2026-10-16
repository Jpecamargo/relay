//! dono: MI1
//! Cursor escondível com o app em background (app de menu bar).
//!
//! `CGDisplayHideCursor` só esconde o cursor de quem está em primeiro plano. Barrier e Synergy
//! contornam ligando a propriedade de conexão `SetsCursorInBackground` na conexão do processo
//! com o WindowServer, por FFI privada (`CGSMainConnectionID` + `CGSSetConnectionProperty`).
//! Os símbolos são procurados com `dlsym`: se a Apple os remover, `enable` devolve `false` e o
//! cursor continua visível em background, sem falha nem panic.

use std::ffi::{CString, c_char, c_void};
use std::sync::OnceLock;

use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::{CFString, CFStringRef};

const PROPERTY: &str = "SetsCursorInBackground";

type MainConnection = unsafe extern "C" fn() -> i32;
type SetProperty = unsafe extern "C" fn(i32, i32, CFStringRef, CFTypeRef) -> i32;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_DEFAULT` no macOS: procura em todas as imagens já carregadas.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

fn lookup(name: &str) -> Option<*mut c_void> {
    let c = CString::new(name).ok()?;
    // SAFETY: `dlsym` com RTLD_DEFAULT e um C string válido; só lê a tabela de símbolos.
    let p = unsafe { dlsym(RTLD_DEFAULT, c.as_ptr()) };
    (!p.is_null()).then_some(p)
}

/// Liga `SetsCursorInBackground` uma vez por processo. `true` se a propriedade foi aceita;
/// `false` (silencioso) se algum símbolo faltou ou o WindowServer recusou.
pub fn enable() -> bool {
    static RESULT: OnceLock<bool> = OnceLock::new();
    *RESULT.get_or_init(|| try_enable().is_some())
}

fn try_enable() -> Option<()> {
    let main = lookup("CGSMainConnectionID").or_else(|| lookup("_CGSDefaultConnection"))?;
    let set = lookup("CGSSetConnectionProperty")?;
    // SAFETY: as assinaturas são as de Barrier/Synergy
    // (`int CGSMainConnectionID(void)`, `CGError CGSSetConnectionProperty(int, int, CFStringRef,
    // CFTypeRef)`); o transmute só acontece com o símbolo encontrado.
    let (main, set): (MainConnection, SetProperty) = unsafe {
        (
            std::mem::transmute::<*mut c_void, MainConnection>(main),
            std::mem::transmute::<*mut c_void, SetProperty>(set),
        )
    };
    // SAFETY: chamadas sem pré-condição além de o processo ter conexão com o WindowServer.
    let err = unsafe {
        let cid = main();
        let key = CFString::new(PROPERTY);
        let value = CFBoolean::true_value();
        set(
            cid,
            cid,
            key.as_concrete_TypeRef(),
            value.as_concrete_TypeRef() as CFTypeRef,
        )
    };
    (err == 0).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simbolo_inexistente_nao_e_encontrado() {
        assert!(lookup("RelayNaoExisteEsteSimbolo").is_none());
    }

    #[test]
    fn enable_nao_da_panic_e_e_estavel() {
        let first = enable();
        eprintln!("SetsCursorInBackground aceito: {first}");
        assert_eq!(enable(), first);
    }
}
