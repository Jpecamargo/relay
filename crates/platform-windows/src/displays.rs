//! dono: W3
//! Leitura de telas com mm via EDID (registro), fallback por DPI, e mudança via WM_DISPLAYCHANGE.

use relay_core::platform::{ChangeCallback, DisplaySource, PlatformError};
use relay_core::types::DisplayInfo;

/// DPI de referência do Windows (escala 100%).
const BASE_DPI: f32 = 96.0;

/// Tamanho físico estimado a partir de pixels e DPI efetivo (fallback quando não há EDID).
pub fn size_mm_from_dpi(size_px: (u32, u32), dpi: (u32, u32)) -> Option<(f32, f32)> {
    if dpi.0 == 0 || dpi.1 == 0 {
        return None;
    }
    Some((
        size_px.0 as f32 / dpi.0 as f32 * 25.4,
        size_px.1 as f32 / dpi.1 as f32 * 25.4,
    ))
}

/// Escala de UI (1.0 = 100%) a partir do DPI efetivo.
pub fn scale_from_dpi(dpi: u32) -> f32 {
    if dpi == 0 { 1.0 } else { dpi as f32 / BASE_DPI }
}

#[derive(Debug, Default)]
pub struct WindowsDisplays {}

impl DisplaySource for WindowsDisplays {
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        #[cfg(windows)]
        {
            imp::list()
        }
        #[cfg(not(windows))]
        Err(PlatformError::Unsupported)
    }

    fn set_change_callback(&mut self, callback: ChangeCallback) {
        #[cfg(windows)]
        imp::set_change_callback(callback);
        #[cfg(not(windows))]
        drop(callback);
    }
}

#[cfg(windows)]
mod imp {
    use super::{BASE_DPI, scale_from_dpi, size_mm_from_dpi};
    use crate::edid::parse_edid_size_mm;
    use relay_core::platform::{ChangeCallback, PlatformError};
    use relay_core::types::{DisplayInfo, Machine};
    use std::sync::{Arc, Mutex, Once};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
        MONITORINFOEXW,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG,
        RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WM_DISPLAYCHANGE, WNDCLASSW,
        WS_OVERLAPPED,
    };
    use windows::core::{BOOL, PCWSTR, w};

    /// `MONITORINFOF_PRIMARY` (a crate não o expõe).
    const MONITORINFOF_PRIMARY: u32 = 1;

    fn wide_to_string(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` é o `&mut Vec<HMONITOR>` passado por `list` e vivo durante a enumeração.
        let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        BOOL(1)
    }

    pub fn list() -> Result<Vec<DisplayInfo>, PlatformError> {
        let mut monitors: Vec<HMONITOR> = Vec::new();
        // SAFETY: o ponteiro para `monitors` só é usado dentro desta chamada.
        let ok = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut _ as isize),
            )
        };
        if !ok.as_bool() {
            return Err(PlatformError::Platform("EnumDisplayMonitors falhou".into()));
        }
        Ok(monitors.into_iter().filter_map(describe).collect())
    }

    /// `DisplayInfo.id` = DeviceID do monitor (`MONITOR\<modelo>\{guid}\<n>`, estável entre reboots);
    /// se o monitor não for resolvido, cai para o `szDevice` do adaptador (`\\.\DISPLAYn`).
    fn describe(monitor: HMONITOR) -> Option<DisplayInfo> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` tem o `cbSize` da variante EX, que o ponteiro base aceita.
        unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) }
            .as_bool()
            .then_some(())?;
        let rc = info.monitorInfo.rcMonitor;
        let size_px = (
            (rc.right - rc.left).max(0) as u32,
            (rc.bottom - rc.top).max(0) as u32,
        );
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        // SAFETY: ponteiros para locais válidos.
        let dpi_ok =
            unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_ok();
        if !dpi_ok {
            (dpi_x, dpi_y) = (BASE_DPI as u32, BASE_DPI as u32);
        }

        let device = monitor_device(&info.szDevice);
        let size_mm = device
            .as_ref()
            .and_then(|(device_id, _)| edid_for_device_id(device_id))
            .and_then(|edid| parse_edid_size_mm(&edid))
            .or_else(|| size_mm_from_dpi(size_px, (dpi_x, dpi_y)));
        let (id, name) = device.unwrap_or_else(|| {
            let adapter = wide_to_string(&info.szDevice);
            (adapter.clone(), adapter)
        });
        Some(DisplayInfo {
            id,
            name,
            machine: Machine::Windows,
            origin_px: (rc.left, rc.top),
            size_px,
            scale: scale_from_dpi(dpi_x),
            size_mm,
            is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            is_builtin: false,
        })
    }

    /// `(DeviceID do monitor, nome amigável)` do primeiro monitor ligado ao adaptador `szDevice`.
    fn monitor_device(sz_device: &[u16; 32]) -> Option<(String, String)> {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: `sz_device` é terminada em NUL (veio de GetMonitorInfoW); `dd` tem `cb` correto.
        unsafe { EnumDisplayDevicesW(PCWSTR(sz_device.as_ptr()), 0, &mut dd, 0) }
            .as_bool()
            .then_some(())?;
        Some((
            wide_to_string(&dd.DeviceID),
            wide_to_string(&dd.DeviceString),
        ))
    }

    /// Lê o EDID de `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\<modelo>\<instância>\Device
    /// Parameters\EDID`. `device_id` tem a forma `MONITOR\<modelo>\{guid}\<n>`; a instância certa é
    /// a cujo valor `Driver` é `{guid}\<n>` (se só houver uma, ela vale).
    fn edid_for_device_id(device_id: &str) -> Option<Vec<u8>> {
        let mut parts = device_id.splitn(3, '\\');
        let (_, model, driver) = (parts.next()?, parts.next()?, parts.next()?);
        let model_path = format!(r"SYSTEM\CurrentControlSet\Enum\DISPLAY\{model}");
        let model_key = open_key(HKEY_LOCAL_MACHINE, &model_path)?;
        let instances = enum_subkeys(model_key);
        let mut fallback = None;
        let mut found = None;
        for instance in &instances {
            let Some(key) = open_key(model_key, instance) else {
                continue;
            };
            let edid = open_key(key, "Device Parameters").and_then(|p| {
                let v = query_binary(p, "EDID");
                close(p);
                v
            });
            let same_driver =
                query_string(key, "Driver").is_some_and(|d| d.eq_ignore_ascii_case(driver));
            close(key);
            if let Some(edid) = edid {
                if same_driver {
                    found = Some(edid);
                    break;
                }
                fallback.get_or_insert(edid);
            }
        }
        close(model_key);
        found.or(if instances.len() == 1 { fallback } else { None })
    }

    fn close(key: HKEY) {
        // SAFETY: chave aberta por `open_key`.
        let _ = unsafe { RegCloseKey(key) };
    }

    fn open_key(parent: HKEY, path: &str) -> Option<HKEY> {
        let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut key = HKEY::default();
        // SAFETY: `path` é terminado em NUL e `key` é local.
        unsafe { RegOpenKeyExW(parent, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) }
            .is_ok()
            .then_some(key)
    }

    fn enum_subkeys(key: HKEY) -> Vec<String> {
        let mut out = Vec::new();
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: `name` e `len` locais e coerentes.
            let rc = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    Some(windows::core::PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if rc.is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        out
    }

    fn query_binary(key: HKEY, name: &str) -> Option<Vec<u8>> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut size = 0u32;
        // SAFETY: primeira chamada só pergunta o tamanho; a segunda usa buffer desse tamanho.
        unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            )
            .ok()
            .ok()?;
            let mut buf = vec![0u8; size as usize];
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut size),
            )
            .ok()
            .ok()?;
            buf.truncate(size as usize);
            Some(buf)
        }
    }

    fn query_string(key: HKEY, name: &str) -> Option<String> {
        let bytes = query_binary(key, name)?;
        let wide: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Some(wide_to_string(&wide))
    }

    // --- mudança de configuração (janela oculta + WM_DISPLAYCHANGE) ---

    static CALLBACK: Mutex<Option<Arc<ChangeCallback>>> = Mutex::new(None);
    static WINDOW: Once = Once::new();

    pub fn set_change_callback(callback: ChangeCallback) {
        if let Ok(mut g) = CALLBACK.lock() {
            *g = Some(Arc::new(callback));
        }
        WINDOW.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("relay-display-watch".into())
                .spawn(message_loop);
        });
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_DISPLAYCHANGE {
            let cb = CALLBACK.lock().ok().and_then(|g| g.clone());
            if let Some(cb) = cb {
                cb();
            }
            return LRESULT(0);
        }
        // SAFETY: repassa os mesmos argumentos recebidos.
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    /// Janela top-level oculta (message-only não recebe broadcasts como WM_DISPLAYCHANGE).
    fn message_loop() {
        // SAFETY: registro de classe e janela com dados estáticos; o loop vive até o fim do processo.
        unsafe {
            let Ok(instance) = GetModuleHandleW(None) else {
                return;
            };
            let class = w!("RelayDisplayWatch");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                return;
            }
            let created = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Relay display watch"),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            );
            if created.is_err() {
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_por_dpi() {
        // 1920x1080 a 96 dpi ≈ 508 x 285.75 mm.
        let (w, h) = size_mm_from_dpi((1920, 1080), (96, 96)).unwrap();
        assert!((w - 508.0).abs() < 0.01 && (h - 285.75).abs() < 0.01);
        assert_eq!(size_mm_from_dpi((1920, 1080), (0, 96)), None);
    }

    #[test]
    fn escala_do_dpi() {
        assert_eq!(scale_from_dpi(96), 1.0);
        assert_eq!(scale_from_dpi(144), 1.5);
        assert_eq!(scale_from_dpi(0), 1.0);
    }
}
