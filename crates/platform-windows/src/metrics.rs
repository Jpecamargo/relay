//! dono: W6
//! Métricas do sistema (PDH, GlobalMemoryStatusEx, NVML). Stub do F2.

use relay_core::types::SystemMetrics;

pub fn collect_metrics() -> SystemMetrics {
    SystemMetrics::default()
}
