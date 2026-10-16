//! dono: W1
//! DPI per-monitor v2 (`set_dpi_awareness`, chamado pelo agente no setup).

/// Ativa DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 (só tem efeito no Windows).
pub fn set_dpi_awareness() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        // SAFETY: sem pré-condições; falha (já definido pelo manifesto) é inofensiva.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }
}
