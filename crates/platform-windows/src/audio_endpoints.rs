//! dono: W4a
//! Endpoints de áudio (enumeração, volume, mudo, troca de saída padrão) via Core Audio.
//! A lógica pura (`find_vbcable`, `find_virtual_output`, `clamp_scalar`) não tem cfg e é testada
//! no Mac; as chamadas COM ficam em `win`, só no target Windows.

use relay_core::platform::PlatformError;
use relay_core::types::{AudioDevice, AudioDevices};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Render,
    Capture,
}

/// Endpoints do VB-CABLE (D2/D14): `input` é o render "CABLE Input" (onde o agente toca o mic do
/// Mac); `output` é o capture "CABLE Output" (o que o Discord usa como microfone).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VbCable {
    pub input: Option<AudioDevice>,
    pub output: Option<AudioDevice>,
}

/// Acha o VB-CABLE por nome amigável (ex.: "CABLE Input (VB-Audio Virtual Cable)"). Não confunde
/// com o cabo A+B ("CABLE-A Input", "CABLE In 16ch"), cujo prefixo é outro.
pub fn find_vbcable(endpoints: &AudioDevices) -> VbCable {
    let by_prefix = |list: &[AudioDevice], prefix: &str| {
        list.iter()
            .find(|d| d.name.trim().to_lowercase().starts_with(prefix))
            .cloned()
    };
    VbCable {
        input: by_prefix(&endpoints.outputs, "cable input"),
        output: by_prefix(&endpoints.inputs, "cable output"),
    }
}

/// Acha a saída virtual configurada pelo usuário (D14, modo `system`): nome exato sem diferenciar
/// caixa; senão, o primeiro que contém o texto. Nome vazio não casa com nada.
pub fn find_virtual_output(devices: &[AudioDevice], name: &str) -> Option<AudioDevice> {
    let want = name.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    devices
        .iter()
        .find(|d| d.name.trim().to_lowercase() == want)
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.name.to_lowercase().contains(&want))
        })
        .cloned()
}

/// Limita o volume a 0..=1 (NaN vira 0).
pub fn clamp_scalar(level: f32) -> f32 {
    if level.is_nan() {
        0.0
    } else {
        level.clamp(0.0, 1.0)
    }
}

/// Endpoints ativos do tipo pedido; `is_default` marca o padrão de console.
pub fn list_endpoints(kind: EndpointKind) -> Result<Vec<AudioDevice>, PlatformError> {
    #[cfg(windows)]
    {
        win::list_endpoints(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        Err(PlatformError::Unsupported)
    }
}

/// Render e capture juntos, no formato de `list_audio_devices`.
pub fn list_all() -> Result<AudioDevices, PlatformError> {
    Ok(AudioDevices {
        outputs: list_endpoints(EndpointKind::Render)?,
        inputs: list_endpoints(EndpointKind::Capture)?,
    })
}

/// Volume (0..1) do endpoint padrão do tipo.
pub fn get_volume(kind: EndpointKind) -> Result<f32, PlatformError> {
    #[cfg(windows)]
    {
        win::get_volume(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        Err(PlatformError::Unsupported)
    }
}

/// Define o volume (0..1, limitado) do endpoint padrão do tipo.
pub fn set_volume(kind: EndpointKind, level: f32) -> Result<(), PlatformError> {
    let level = clamp_scalar(level);
    #[cfg(windows)]
    {
        win::set_volume(kind, level)
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, level);
        Err(PlatformError::Unsupported)
    }
}

pub fn get_mute(kind: EndpointKind) -> Result<bool, PlatformError> {
    #[cfg(windows)]
    {
        win::get_mute(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        Err(PlatformError::Unsupported)
    }
}

pub fn set_mute(kind: EndpointKind, muted: bool) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        win::set_mute(kind, muted)
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, muted);
        Err(PlatformError::Unsupported)
    }
}

/// Id do endpoint padrão de console do tipo.
pub fn default_endpoint_id(kind: EndpointKind) -> Result<String, PlatformError> {
    #[cfg(windows)]
    {
        win::default_endpoint_id(kind)
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        Err(PlatformError::Unsupported)
    }
}

/// Torna `id` o padrão para eConsole, eMultimedia e eCommunications (via IPolicyConfig).
pub fn set_default_endpoint(id: &str) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        win::set_default_endpoint(id)
    }
    #[cfg(not(windows))]
    {
        let _ = id;
        Err(PlatformError::Unsupported)
    }
}

/// Guarda a saída padrão original e a restaura em `restore()` e no `Drop`.
#[derive(Debug, Default)]
pub struct DefaultOutputGuard {
    original: Option<String>,
}

impl DefaultOutputGuard {
    /// Registra a saída padrão atual como a original.
    pub fn new() -> Result<Self, PlatformError> {
        Ok(Self {
            original: Some(default_endpoint_id(EndpointKind::Render)?),
        })
    }

    /// Troca a saída padrão para `id`; a original é guardada na primeira troca.
    pub fn switch_to(&mut self, id: &str) -> Result<(), PlatformError> {
        if self.original.is_none() {
            self.original = Some(default_endpoint_id(EndpointKind::Render)?);
        }
        set_default_endpoint(id)
    }

    /// Volta à saída original. Idempotente: só limpa o estado se a restauração der certo.
    pub fn restore(&mut self) -> Result<(), PlatformError> {
        if let Some(id) = self.original.clone() {
            set_default_endpoint(&id)?;
            self.original = None;
        }
        Ok(())
    }
}

impl Drop for DefaultOutputGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(windows)]
mod win {
    use super::{AudioDevice, EndpointKind, PlatformError};
    use std::cell::Cell;
    use std::ffi::c_void;
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
        eCapture, eCommunications, eConsole, eMultimedia, eRender,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        STGM_READ,
    };
    use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, PWSTR};

    /// IPolicyConfig (Vista+), interface COM não documentada do Windows, declarada à mão.
    /// IID (PolicyConfig.h, ReactOS/EarTrumpet): f8679f50-850a-41cf-9c72-430f290290c8.
    /// A ordem dos campos é a da vtable real; só `SetDefaultEndpoint` é chamado, os demais
    /// existem para manter os offsets e usam ponteiros opacos.
    #[repr(transparent)]
    #[derive(Clone)]
    struct IPolicyConfig(IUnknown);

    #[repr(C)]
    #[allow(non_snake_case)]
    struct IPolicyConfig_Vtbl {
        base: IUnknown_Vtbl,
        GetMixFormat: unsafe extern "system" fn(*mut c_void, PCWSTR, *mut *mut c_void) -> HRESULT,
        GetDeviceFormat:
            unsafe extern "system" fn(*mut c_void, PCWSTR, i32, *mut *mut c_void) -> HRESULT,
        ResetDeviceFormat: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
        SetDeviceFormat:
            unsafe extern "system" fn(*mut c_void, PCWSTR, *mut c_void, *mut c_void) -> HRESULT,
        GetProcessingPeriod:
            unsafe extern "system" fn(*mut c_void, PCWSTR, i32, *mut i64, *mut i64) -> HRESULT,
        SetProcessingPeriod: unsafe extern "system" fn(*mut c_void, PCWSTR, *mut i64) -> HRESULT,
        GetShareMode: unsafe extern "system" fn(*mut c_void, PCWSTR, *mut c_void) -> HRESULT,
        SetShareMode: unsafe extern "system" fn(*mut c_void, PCWSTR, *mut c_void) -> HRESULT,
        GetPropertyValue: unsafe extern "system" fn(
            *mut c_void,
            PCWSTR,
            i32,
            *const c_void,
            *mut c_void,
        ) -> HRESULT,
        SetPropertyValue: unsafe extern "system" fn(
            *mut c_void,
            PCWSTR,
            i32,
            *const c_void,
            *mut c_void,
        ) -> HRESULT,
        SetDefaultEndpoint: unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT,
        SetEndpointVisibility: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    }

    unsafe impl Interface for IPolicyConfig {
        type Vtable = IPolicyConfig_Vtbl;
        const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    }

    impl IPolicyConfig {
        unsafe fn set_default_endpoint(&self, device: PCWSTR, role: ERole) -> HRESULT {
            unsafe {
                (Interface::vtable(self).SetDefaultEndpoint)(Interface::as_raw(self), device, role)
            }
        }
    }

    /// CLSID de CPolicyConfigClient.
    const CLSID_POLICY_CONFIG_CLIENT: GUID =
        GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    fn err(what: &str, e: windows::core::Error) -> PlatformError {
        PlatformError::Platform(format!("{what}: {e}"))
    }

    fn init_com() {
        thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
        DONE.with(|d| {
            if !d.get() {
                // S_FALSE (já iniciado) e RPC_E_CHANGED_MODE (outro modelo) são aceitáveis aqui.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                d.set(true);
            }
        });
    }

    fn flow(kind: EndpointKind) -> EDataFlow {
        match kind {
            EndpointKind::Render => eRender,
            EndpointKind::Capture => eCapture,
        }
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, PlatformError> {
        init_com();
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| err("MMDeviceEnumerator", e))
    }

    fn take_pwstr(p: PWSTR) -> String {
        let s = unsafe { p.to_string() }.unwrap_or_default();
        unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
        s
    }

    fn device_id(d: &IMMDevice) -> Result<String, PlatformError> {
        unsafe { d.GetId() }
            .map(take_pwstr)
            .map_err(|e| err("GetId", e))
    }

    fn friendly_name(d: &IMMDevice) -> Result<String, PlatformError> {
        let store =
            unsafe { d.OpenPropertyStore(STGM_READ) }.map_err(|e| err("OpenPropertyStore", e))?;
        let value = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
            .map_err(|e| err("GetValue(FriendlyName)", e))?;
        unsafe { PropVariantToStringAlloc(&value) }
            .map(take_pwstr)
            .map_err(|e| err("PropVariantToStringAlloc", e))
    }

    fn default_device(kind: EndpointKind) -> Result<IMMDevice, PlatformError> {
        unsafe { enumerator()?.GetDefaultAudioEndpoint(flow(kind), eConsole) }
            .map_err(|e| err("GetDefaultAudioEndpoint", e))
    }

    pub fn default_endpoint_id(kind: EndpointKind) -> Result<String, PlatformError> {
        device_id(&default_device(kind)?)
    }

    pub fn list_endpoints(kind: EndpointKind) -> Result<Vec<AudioDevice>, PlatformError> {
        let en = enumerator()?;
        let default_id = default_endpoint_id(kind).ok();
        let coll = unsafe { en.EnumAudioEndpoints(flow(kind), DEVICE_STATE_ACTIVE) }
            .map_err(|e| err("EnumAudioEndpoints", e))?;
        let count = unsafe { coll.GetCount() }.map_err(|e| err("GetCount", e))?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let dev = unsafe { coll.Item(i) }.map_err(|e| err("Item", e))?;
            let id = device_id(&dev)?;
            let name = friendly_name(&dev).unwrap_or_else(|_| id.clone());
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(AudioDevice {
                id,
                name,
                is_default,
            });
        }
        Ok(out)
    }

    fn volume(kind: EndpointKind) -> Result<IAudioEndpointVolume, PlatformError> {
        unsafe { default_device(kind)?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }
            .map_err(|e| err("Activate(IAudioEndpointVolume)", e))
    }

    pub fn get_volume(kind: EndpointKind) -> Result<f32, PlatformError> {
        unsafe { volume(kind)?.GetMasterVolumeLevelScalar() }
            .map_err(|e| err("GetMasterVolumeLevelScalar", e))
    }

    pub fn set_volume(kind: EndpointKind, level: f32) -> Result<(), PlatformError> {
        unsafe { volume(kind)?.SetMasterVolumeLevelScalar(level, std::ptr::null()) }
            .map_err(|e| err("SetMasterVolumeLevelScalar", e))
    }

    pub fn get_mute(kind: EndpointKind) -> Result<bool, PlatformError> {
        unsafe { volume(kind)?.GetMute() }
            .map(|b| b.as_bool())
            .map_err(|e| err("GetMute", e))
    }

    pub fn set_mute(kind: EndpointKind, muted: bool) -> Result<(), PlatformError> {
        unsafe { volume(kind)?.SetMute(muted, std::ptr::null()) }.map_err(|e| err("SetMute", e))
    }

    pub fn set_default_endpoint(id: &str) -> Result<(), PlatformError> {
        init_com();
        let policy: IPolicyConfig =
            unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
                .map_err(|e| err("CPolicyConfigClient", e))?;
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        for role in [eConsole, eMultimedia, eCommunications] {
            unsafe { policy.set_default_endpoint(PCWSTR(wide.as_ptr()), role) }
                .ok()
                .map_err(|e| err("SetDefaultEndpoint", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> AudioDevice {
        AudioDevice {
            id: format!("id-{name}"),
            name: name.into(),
            is_default: false,
        }
    }

    #[test]
    fn finds_vbcable_by_name() {
        let eps = AudioDevices {
            outputs: vec![
                dev("Alto-falantes (Realtek)"),
                dev("CABLE-A Input (VB-Audio Cable A)"),
                dev("CABLE Input (VB-Audio Virtual Cable)"),
            ],
            inputs: vec![
                dev("Microfone (Realtek)"),
                dev("CABLE Output (VB-Audio Virtual Cable)"),
            ],
        };
        let c = find_vbcable(&eps);
        assert_eq!(
            c.input.unwrap().name,
            "CABLE Input (VB-Audio Virtual Cable)"
        );
        assert_eq!(
            c.output.unwrap().name,
            "CABLE Output (VB-Audio Virtual Cable)"
        );
    }

    #[test]
    fn vbcable_missing_is_none() {
        let eps = AudioDevices {
            outputs: vec![dev("CABLE-A Input (VB-Audio Cable A)")],
            inputs: vec![],
        };
        assert_eq!(find_vbcable(&eps), VbCable::default());
    }

    #[test]
    fn virtual_output_exact_then_contains() {
        let list = vec![
            dev("CABLE-A Input (VB-Audio Cable A)"),
            dev("CABLE Input (VB-Audio Virtual Cable)"),
        ];
        let exact = find_virtual_output(&list, "cable input (vb-audio virtual cable)").unwrap();
        assert_eq!(exact.id, "id-CABLE Input (VB-Audio Virtual Cable)");
        let part = find_virtual_output(&list, "Cable A").unwrap();
        assert_eq!(part.id, "id-CABLE-A Input (VB-Audio Cable A)");
        assert!(find_virtual_output(&list, "").is_none());
        assert!(find_virtual_output(&list, "inexistente").is_none());
    }

    #[test]
    fn clamp_limits_range() {
        assert_eq!(clamp_scalar(-1.0), 0.0);
        assert_eq!(clamp_scalar(2.0), 1.0);
        assert_eq!(clamp_scalar(f32::NAN), 0.0);
        assert_eq!(clamp_scalar(0.4), 0.4);
    }

    #[test]
    fn guard_default_restore_is_noop() {
        let mut g = DefaultOutputGuard::default();
        assert!(g.restore().is_ok());
    }
}
