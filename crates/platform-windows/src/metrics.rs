//! dono: W6
//! Métricas do sistema: CPU via PDH, RAM via GlobalMemoryStatusEx, GPU via NVML (carga dinâmica).

use relay_core::types::SystemMetrics;

const MIB: u64 = 1024 * 1024;

#[cfg_attr(not(windows), allow(dead_code))]
/// Bytes → MiB, saturando em `u32`.
fn bytes_to_mb(bytes: u64) -> u32 {
    u32::try_from(bytes / MIB).unwrap_or(u32::MAX)
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Limita o percentual lido do contador ao intervalo [0, 100].
fn clamp_pct(v: f64) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 100.0) as f32
    } else {
        0.0
    }
}

pub fn collect_metrics() -> SystemMetrics {
    #[cfg(windows)]
    {
        imp::collect()
    }
    #[cfg(not(windows))]
    {
        SystemMetrics::default()
    }
}

#[cfg(windows)]
mod imp {
    use super::{bytes_to_mb, clamp_pct};
    use nvml_wrapper::Nvml;
    use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
    use relay_core::types::SystemMetrics;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;
    use windows::Win32::System::Performance::{
        PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW,
        PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
    };
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::core::w;

    const PDH_OK: u32 = 0;

    struct Pdh {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    // Os handles PDH são ponteiros opacos; o acesso é serializado pelo Mutex.
    unsafe impl Send for Pdh {}

    static PDH: OnceLock<Option<Mutex<Pdh>>> = OnceLock::new();
    static NVML: OnceLock<Option<Nvml>> = OnceLock::new();

    fn open_pdh() -> Option<Mutex<Pdh>> {
        let mut query = PDH_HQUERY::default();
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: ponteiros de saída válidos; strings literais terminadas em nulo.
        unsafe {
            if PdhOpenQueryW(None, 0, &mut query) != PDH_OK {
                return None;
            }
            if PdhAddEnglishCounterW(
                query,
                w!("\\Processor(_Total)\\% Processor Time"),
                0,
                &mut counter,
            ) != PDH_OK
            {
                return None;
            }
            // O contador de taxa precisa de duas amostras: a primeira só prepara a leitura.
            if PdhCollectQueryData(query) != PDH_OK {
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        Some(Mutex::new(Pdh { query, counter }))
    }

    fn cpu_pct() -> f32 {
        let Some(pdh) = PDH.get_or_init(open_pdh).as_ref() else {
            return 0.0;
        };
        let Ok(pdh) = pdh.lock() else {
            return 0.0;
        };
        // SAFETY: handles criados em `open_pdh`; `value` é preenchido pelo PDH.
        unsafe {
            if PdhCollectQueryData(pdh.query) != PDH_OK {
                return 0.0;
            }
            let mut value = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(pdh.counter, PDH_FMT_DOUBLE, None, &mut value) != PDH_OK
                || value.CStatus != PDH_OK
            {
                return 0.0;
            }
            clamp_pct(value.Anonymous.doubleValue)
        }
    }

    fn ram_mb() -> (u32, u32) {
        let mut st = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: `dwLength` preenchido conforme exigido.
        if unsafe { GlobalMemoryStatusEx(&mut st) }.is_err() {
            return (0, 0);
        }
        let total = st.ullTotalPhys;
        (
            bytes_to_mb(total.saturating_sub(st.ullAvailPhys)),
            bytes_to_mb(total),
        )
    }

    /// `(gpu_pct, vram_usada_mb, temperatura_c)` da primeira GPU NVIDIA; tudo `None` sem nvml.dll.
    fn gpu() -> (Option<f32>, Option<u32>, Option<f32>) {
        let Some(nvml) = NVML.get_or_init(|| Nvml::init().ok()).as_ref() else {
            return (None, None, None);
        };
        let Ok(dev) = nvml.device_by_index(0) else {
            return (None, None, None);
        };
        let pct = dev.utilization_rates().ok().map(|u| u.gpu as f32);
        let mem = dev.memory_info().ok().map(|m| bytes_to_mb(m.used));
        let temp = dev
            .temperature(TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32);
        (pct, mem, temp)
    }

    pub fn collect() -> SystemMetrics {
        let (ram_used_mb, ram_total_mb) = ram_mb();
        let (gpu_pct, gpu_mem_mb, gpu_temp_c) = gpu();
        SystemMetrics {
            cpu_pct: cpu_pct(),
            ram_used_mb,
            ram_total_mb,
            gpu_pct,
            gpu_mem_mb,
            gpu_temp_c,
            cpu_temp_c: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_mb_converte_e_satura() {
        assert_eq!(bytes_to_mb(0), 0);
        assert_eq!(bytes_to_mb(8 * 1024 * MIB), 8192);
        assert_eq!(bytes_to_mb(u64::MAX), u32::MAX);
    }

    #[test]
    fn clamp_pct_limita_e_trata_nan() {
        assert_eq!(clamp_pct(-3.0), 0.0);
        assert_eq!(clamp_pct(142.0), 100.0);
        assert_eq!(clamp_pct(f64::NAN), 0.0);
        assert_eq!(clamp_pct(37.5), 37.5);
    }

    #[cfg(not(windows))]
    #[test]
    fn fora_do_windows_devolve_neutro() {
        assert_eq!(collect_metrics(), SystemMetrics::default());
    }
}
