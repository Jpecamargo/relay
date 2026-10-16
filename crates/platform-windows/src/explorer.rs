//! dono: W5
//! Destino de drop: pasta do Explorer sob o cursor (IShellWindows) e pasta Downloads (D20).

use std::path::PathBuf;

/// Converte a `LocationURL` de uma janela do Explorer (`file:///C:/a%20b`,
/// `file://servidor/share/x`) em caminho Windows. `None` para pastas virtuais (não-`file:`).
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.get(..5).filter(|p| p.eq_ignore_ascii_case("file:"))?;
    let rest = &url[rest.len()..];
    let rest = rest.strip_prefix("//")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let decoded = percent_decode(path)?;
    let path = decoded.replace('/', "\\");
    let full = if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        path
    } else {
        format!("\\\\{host}\\{path}")
    };
    (!full.is_empty()).then(|| PathBuf::from(full))
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Pasta aberta na janela do Explorer sob o cursor, se houver (`None` em outra janela, na área de
/// trabalho ou em pasta virtual). Chamável de qualquer thread (inicializa COM se preciso).
pub fn explorer_folder_at_cursor() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        win::folder_at_cursor()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Pasta Downloads do usuário (`FOLDERID_Downloads`).
pub fn downloads_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        win::downloads_dir()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod win {
    use std::path::PathBuf;

    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Shell::{
        FOLDERID_Downloads, IShellWindows, IWebBrowserApp, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
        ShellWindows,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GetAncestor, GetCursorPos, WindowFromPoint,
    };
    use windows::core::Interface;

    pub fn downloads_dir() -> Option<PathBuf> {
        // SAFETY: o PWSTR devolvido é liberado com CoTaskMemFree após a cópia.
        unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.as_ptr().cast()));
            s.map(PathBuf::from)
        }
    }

    pub fn folder_at_cursor() -> Option<PathBuf> {
        // SAFETY: chamadas COM/Win32 síncronas na thread atual; COM é inicializado aqui (S_FALSE
        // ou modo diferente já inicializado são aceitáveis: ignoramos o resultado).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut pt = POINT::default();
            GetCursorPos(&mut pt).ok()?;
            let top = GetAncestor(WindowFromPoint(pt), GA_ROOT);
            if top.0.is_null() {
                return None;
            }
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
            let count = windows.Count().ok()?;
            for i in 0..count {
                let Ok(disp) = windows.Item(&VARIANT::from(i)) else {
                    continue;
                };
                let Ok(app) = disp.cast::<IWebBrowserApp>() else {
                    continue;
                };
                let Ok(h) = app.HWND() else { continue };
                if HWND(h.0 as *mut _) != top {
                    continue;
                }
                let Ok(url) = app.LocationURL() else { continue };
                if let Some(path) = super::file_url_to_path(&url.to_string()) {
                    return Some(path);
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_to_path() {
        assert_eq!(
            file_url_to_path("file:///C:/Users/Jo%C3%A3o/My%20Docs"),
            Some(PathBuf::from("C:\\Users\\João\\My Docs"))
        );
        assert_eq!(
            file_url_to_path("file://srv/share/x"),
            Some(PathBuf::from("\\\\srv\\share\\x"))
        );
        assert_eq!(file_url_to_path("shell:::{guid}"), None);
        assert_eq!(file_url_to_path("file:///C:/bad%zz"), None);
        assert_eq!(file_url_to_path(""), None);
    }
}
