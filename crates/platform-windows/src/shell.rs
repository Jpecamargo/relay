//! dono: W3
//! `ShellExecute` de URL, caminho ou nome de app.

use relay_core::platform::PlatformError;

/// Valida o alvo e devolve-o sem espaços nas pontas. Aceita URL http(s), caminho e nome de app;
/// rejeita vazio e caracteres de controle (incluindo NUL, que truncaria a string do Win32).
pub fn validate_target(target: &str) -> Result<&str, PlatformError> {
    let t = target.trim();
    if t.is_empty() {
        return Err(PlatformError::Platform("alvo vazio".into()));
    }
    if t.chars().any(char::is_control) {
        return Err(PlatformError::Platform(
            "alvo com caractere de controle".into(),
        ));
    }
    Ok(t)
}

#[cfg(windows)]
pub fn open_target(target: &str) -> Result<(), PlatformError> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let target = HSTRING::from(validate_target(target)?);
    // SAFETY: strings terminadas em NUL que vivem até o fim da chamada.
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    // Sucesso é qualquer valor > 32 (contrato do ShellExecute).
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(PlatformError::Platform(format!(
            "ShellExecuteW falhou (código {})",
            result.0 as usize
        )))
    }
}

#[cfg(not(windows))]
pub fn open_target(target: &str) -> Result<(), PlatformError> {
    validate_target(target)?;
    Err(PlatformError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aceita_url_caminho_e_nome() {
        assert_eq!(
            validate_target("https://example.com/a?b=1").unwrap(),
            "https://example.com/a?b=1"
        );
        assert!(validate_target("http://localhost:3000").is_ok());
        assert!(validate_target(r"C:\Program Files\App\app.exe").is_ok());
        assert_eq!(validate_target("  notepad  ").unwrap(), "notepad");
    }

    #[test]
    fn rejeita_vazio_e_controle() {
        assert!(validate_target("").is_err());
        assert!(validate_target("   \t").is_err());
        assert!(validate_target("a\0b").is_err());
        assert!(validate_target("a\nb").is_err());
    }
}
