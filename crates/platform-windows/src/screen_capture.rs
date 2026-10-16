//! dono: W6
//! Captura de tela de um monitor em PNG (Windows.Graphics.Capture).

use relay_core::platform::PlatformError;

/// Captura um frame do monitor `display_id` (id de dispositivo, ex.: `\\.\DISPLAY1`) como PNG.
pub fn capture_monitor_png(display_id: &str) -> Result<Vec<u8>, PlatformError> {
    #[cfg(windows)]
    {
        imp::capture(display_id)
    }
    #[cfg(not(windows))]
    {
        let _ = display_id;
        Err(PlatformError::Unsupported)
    }
}

/// Converte pixels BGRA (linhas contíguas, `width * 4` bytes cada) em PNG RGBA 8 bits.
/// O alfa é forçado a 255: a captura de monitor é opaca.
pub fn bgra_to_png(bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>, PlatformError> {
    let row = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| PlatformError::Platform("dimensões inválidas".into()))?;
    let expected = row.checked_mul(height as usize);
    if width == 0 || height == 0 || expected != Some(bgra.len()) {
        return Err(PlatformError::Platform(format!(
            "buffer BGRA incompatível com {width}x{height}"
        )));
    }

    // O alfa vem como 0 em parte dos drivers; a captura de monitor é opaca.
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }

    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc
        .write_header()
        .map_err(|e| PlatformError::Platform(format!("PNG: {e}")))?;
    writer
        .write_image_data(&rgba)
        .map_err(|e| PlatformError::Platform(format!("PNG: {e}")))?;
    writer
        .finish()
        .map_err(|e| PlatformError::Platform(format!("PNG: {e}")))?;
    Ok(out)
}

#[cfg(windows)]
mod imp {
    use super::bgra_to_png;
    use relay_core::platform::PlatformError;
    use std::time::{Duration, Instant};
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HMODULE, LPARAM, RECT};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{
        DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
        MONITORINFOEXW,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::core::{BOOL, Interface};

    const FRAME_TIMEOUT: Duration = Duration::from_secs(3);

    fn err(what: &str, e: impl std::fmt::Display) -> PlatformError {
        PlatformError::Platform(format!("{what}: {e}"))
    }

    fn wide_to_string(w: &[u16]) -> String {
        let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..n])
    }

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` aponta para o Vec vivo em `find_monitor`.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        BOOL(1)
    }

    /// Acha o HMONITOR cujo nome de dispositivo (`\\.\DISPLAYn`) ou id de monitor
    /// (`DISPLAY\...`) é `display_id`.
    fn find_monitor(display_id: &str) -> Result<HMONITOR, PlatformError> {
        let mut monitors: Vec<HMONITOR> = Vec::new();
        // SAFETY: o callback só escreve no Vec, que vive durante a chamada.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(collect_monitor),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }
        for monitor in monitors {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: `cbSize` preenchido; MONITORINFOEXW começa com MONITORINFO.
            if !unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) }.as_bool() {
                continue;
            }
            let device = wide_to_string(&info.szDevice);
            if device.eq_ignore_ascii_case(display_id) {
                return Ok(monitor);
            }
            let mut dd = DISPLAY_DEVICEW {
                cb: size_of::<DISPLAY_DEVICEW>() as u32,
                ..Default::default()
            };
            // SAFETY: `szDevice` é terminado em nulo; `dd.cb` preenchido.
            if unsafe {
                EnumDisplayDevicesW(windows::core::PCWSTR(info.szDevice.as_ptr()), 0, &mut dd, 0)
            }
            .as_bool()
                && (wide_to_string(&dd.DeviceID).eq_ignore_ascii_case(display_id)
                    || wide_to_string(&dd.DeviceName).eq_ignore_ascii_case(display_id))
            {
                return Ok(monitor);
            }
        }
        Err(PlatformError::Platform(format!(
            "monitor não encontrado: {display_id}"
        )))
    }

    fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext), PlatformError> {
        let mut device = None;
        let mut context = None;
        // SAFETY: ponteiros de saída válidos.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| err("D3D11CreateDevice", e))?;
        device
            .zip(context)
            .ok_or_else(|| PlatformError::Platform("D3D11CreateDevice sem dispositivo".into()))
    }

    pub fn capture(display_id: &str) -> Result<Vec<u8>, PlatformError> {
        // SAFETY: inicialização idempotente; falha (já inicializado em outro modo) é ignorada.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if !windows::Graphics::Capture::GraphicsCaptureSession::IsSupported()
            .map_err(|e| err("GraphicsCaptureSession::IsSupported", e))?
        {
            return Err(PlatformError::Unsupported);
        }
        let monitor = find_monitor(display_id)?;
        let (device, context) = create_device()?;

        let dxgi: IDXGIDevice = device.cast().map_err(|e| err("IDXGIDevice", e))?;
        // SAFETY: `dxgi` vivo durante a chamada.
        let winrt_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .and_then(|d| d.cast())
            .map_err(|e| err("CreateDirect3D11DeviceFromDXGIDevice", e))?;

        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| err("IGraphicsCaptureItemInterop", e))?;
        // SAFETY: `monitor` é um HMONITOR válido obtido acima.
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(monitor) }.map_err(|e| err("CreateForMonitor", e))?;
        let size = item.Size().map_err(|e| err("item.Size", e))?;

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )
        .map_err(|e| err("CreateFreeThreaded", e))?;
        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| err("CreateCaptureSession", e))?;
        session.StartCapture().map_err(|e| err("StartCapture", e))?;

        let deadline = Instant::now() + FRAME_TIMEOUT;
        let frame = loop {
            if let Ok(f) = pool.TryGetNextFrame() {
                break f;
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::Platform("timeout aguardando frame".into()));
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        let result = read_frame(&device, &context, &frame);
        let _ = session.Close();
        let _ = pool.Close();
        let (bgra, w, h) = result?;
        bgra_to_png(&bgra, w, h)
    }

    fn read_frame(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
    ) -> Result<(Vec<u8>, u32, u32), PlatformError> {
        let surface = frame.Surface().map_err(|e| err("frame.Surface", e))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|e| err("IDirect3DDxgiInterfaceAccess", e))?;
        // SAFETY: a interface devolvida ganha refcount próprio.
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
            .map_err(|e| err("GetInterface(ID3D11Texture2D)", e))?;

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `desc` válido para escrita.
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.MiscFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;

        let mut staging = None;
        // SAFETY: `desc` descreve uma textura de staging legível pela CPU.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
            .map_err(|e| err("CreateTexture2D(staging)", e))?;
        let staging =
            staging.ok_or_else(|| PlatformError::Platform("staging não criado".into()))?;

        let (w, h) = (desc.Width, desc.Height);
        let row = w as usize * 4;
        let mut out = vec![0u8; row * h as usize];
        // SAFETY: Map/Unmap pareados; leitura limitada a `h` linhas de `RowPitch` bytes.
        unsafe {
            context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| err("Map", e))?;
            let pitch = mapped.RowPitch as usize;
            let src = mapped.pData as *const u8;
            for y in 0..h as usize {
                let line = std::slice::from_raw_parts(src.add(y * pitch), row);
                out[y * row..(y + 1) * row].copy_from_slice(line);
            }
            context.Unmap(&staging, 0);
        }
        Ok((out, w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(data: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(data))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    #[test]
    fn bgra_to_png_troca_canais_e_forca_alfa() {
        // 2x2: azul, verde / vermelho, branco (BGRA), alfa 0 na entrada.
        let bgra = [
            255, 0, 0, 0, 0, 255, 0, 0, //
            0, 0, 255, 0, 255, 255, 255, 0,
        ];
        let (w, h, rgba) = decode(&bgra_to_png(&bgra, 2, 2).unwrap());
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rgba,
            [
                0, 0, 255, 255, 0, 255, 0, 255, //
                255, 0, 0, 255, 255, 255, 255, 255
            ]
        );
    }

    #[test]
    fn bgra_to_png_imagem_grande_faz_roundtrip() {
        let (w, h) = (300u32, 200u32);
        let bgra: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let (dw, dh, rgba) = decode(&bgra_to_png(&bgra, w, h).unwrap());
        assert_eq!((dw, dh), (w, h));
        assert_eq!(&rgba[..4], &[bgra[2], bgra[1], bgra[0], 255]);
        assert_eq!(rgba.len(), bgra.len());
    }

    #[test]
    fn bgra_to_png_rejeita_buffer_incompativel() {
        assert!(bgra_to_png(&[0; 7], 2, 1).is_err());
        assert!(bgra_to_png(&[], 0, 0).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn fora_do_windows_nao_suportado() {
        assert!(matches!(
            capture_monitor_png("\\\\.\\DISPLAY1"),
            Err(PlatformError::Unsupported)
        ));
    }
}
