//! dono: W3
//! Ações de energia (lock, suspend, restart, shutdown).

use relay_core::platform::PlatformError;

#[cfg(windows)]
fn err(what: &str, e: windows::core::Error) -> PlatformError {
    PlatformError::Platform(format!("{what}: {e}"))
}

pub fn lock() -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Shutdown::LockWorkStation;
        // SAFETY: sem argumentos nem ponteiros.
        unsafe { LockWorkStation() }.map_err(|e| err("LockWorkStation", e))
    }
    #[cfg(not(windows))]
    Err(PlatformError::Unsupported)
}

pub fn suspend() -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::SetSuspendState;
        // Suspender (não hibernar), sem forçar, com eventos de despertar habilitados.
        // SAFETY: sem ponteiros.
        if unsafe { SetSuspendState(false, false, false) } {
            Ok(())
        } else {
            Err(err("SetSuspendState", windows::core::Error::from_thread()))
        }
    }
    #[cfg(not(windows))]
    Err(PlatformError::Unsupported)
}

pub fn restart() -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Shutdown::EWX_REBOOT;
        exit_windows(EWX_REBOOT)
    }
    #[cfg(not(windows))]
    Err(PlatformError::Unsupported)
}

pub fn shutdown() -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Shutdown::EWX_POWEROFF;
        exit_windows(EWX_POWEROFF)
    }
    #[cfg(not(windows))]
    Err(PlatformError::Unsupported)
}

/// Habilita `SE_SHUTDOWN_NAME` no token do processo e chama `ExitWindowsEx`.
#[cfg(windows)]
fn exit_windows(
    mode: windows::Win32::System::Shutdown::EXIT_WINDOWS_FLAGS,
) -> Result<(), PlatformError> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows::Win32::System::Shutdown::{
        EWX_FORCEIFHUNG, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SAFETY: handles e estruturas locais, válidos durante cada chamada; o token é fechado ao fim.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| err("OpenProcessToken", e))?;

        let mut luid = LUID::default();
        let result =
            LookupPrivilegeValueW(None, windows::Win32::Security::SE_SHUTDOWN_NAME, &mut luid)
                .map_err(|e| err("LookupPrivilegeValueW", e))
                .and_then(|()| {
                    let privileges = TOKEN_PRIVILEGES {
                        PrivilegeCount: 1,
                        Privileges: [LUID_AND_ATTRIBUTES {
                            Luid: luid,
                            Attributes: SE_PRIVILEGE_ENABLED,
                        }],
                    };
                    AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None)
                        .map_err(|e| err("AdjustTokenPrivileges", e))
                });
        let _ = CloseHandle(token);
        result?;

        ExitWindowsEx(
            mode | EWX_FORCEIFHUNG,
            SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
        )
        .map_err(|e| err("ExitWindowsEx", e))
    }
}
