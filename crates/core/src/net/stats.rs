//! Relógio monotônico e estimadores de rede (também usado pelas threads do `fast`, que não têm runtime assíncrono).

use std::sync::OnceLock;
use std::time::Instant;

use super::fast::{FastShared, FastStats};
use crate::types::LatencyStats;

/// Microssegundos desde a primeira chamada no processo. É o relógio de `ts_us` dos pacotes e dos
/// Pings; só diferenças do mesmo lado têm significado (os relógios das duas máquinas não casam).
pub fn now_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// Jitter no estilo RFC 3550: `J += (|D| - J) / 16`, com D a variação do trânsito entre amostras.
#[derive(Debug, Default, Clone, Copy)]
pub struct Jitter {
    prev: Option<f64>,
    value: f64,
}

impl Jitter {
    pub fn update(&mut self, transit_us: f64) {
        if let Some(prev) = self.prev {
            self.value += ((transit_us - prev).abs() - self.value) / 16.0;
        }
        self.prev = Some(transit_us);
    }

    pub fn value_us(&self) -> f64 {
        self.value
    }
}

/// RTT suavizado (EWMA 1/8) e jitter do RTT, a partir de Ping/Pong.
#[derive(Debug, Default)]
pub struct RttTracker {
    rtt_ms: Option<f64>,
    jitter: Jitter,
}

impl RttTracker {
    pub fn sample(&mut self, rtt_us: u64) {
        // 1 us de piso: em loopback o relógio pode devolver 0 e rtt_ms precisa ser > 0.
        let us = rtt_us.max(1) as f64;
        self.jitter.update(us);
        let ms = us / 1000.0;
        self.rtt_ms = Some(match self.rtt_ms {
            Some(prev) => prev + (ms - prev) / 8.0,
            None => ms,
        });
    }

    /// Com o canal rápido ativo, jitter e perda vêm dele, e o RTT também quando o par responde ao
    /// eco UDP: é o caminho do mouse. O Ping/Pong (TCP) fica como RTT de quem não tem eco.
    pub fn snapshot(&self, fast: Option<FastStats>) -> LatencyStats {
        let (jitter_us, loss_pct, rtt_ms) = match fast {
            Some(f) => (
                f.jitter_us,
                f.loss_pct,
                f.rtt_us.map(|r| r / 1000.0).or(self.rtt_ms),
            ),
            None => (self.jitter.value_us(), 0.0, self.rtt_ms),
        };
        LatencyStats {
            rtt_ms: rtt_ms.unwrap_or(0.0) as f32,
            jitter_ms: (jitter_us / 1000.0) as f32,
            loss_pct,
        }
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.rtt_ms
    }
}

/// Resumo de latência no log a cada minuto de sessão: RTT do eco UDP (caminho do mouse), RTT do
/// Ping TCP e quanto tempo este lado segurou cada Ping antes do Pong. Separa rede de software.
#[derive(Debug)]
pub struct LatencyReport {
    since: Instant,
    held_us: Vec<u32>,
}

const REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

impl LatencyReport {
    pub fn new() -> Self {
        Self {
            since: Instant::now(),
            held_us: Vec::new(),
        }
    }

    /// Tempo entre a chegada de um Ping e o envio do Pong.
    pub fn held(&mut self, us: u64) {
        if self.held_us.len() < 4096 {
            self.held_us.push(us.min(u64::from(u32::MAX)) as u32);
        }
    }

    pub(super) fn maybe_log(&mut self, tcp: &RttTracker, fast: &FastShared) {
        if self.since.elapsed() < REPORT_EVERY {
            return;
        }
        self.since = Instant::now();
        let udp = summary(fast.take_rtt_samples());
        let held = summary(std::mem::take(&mut self.held_us));
        tracing::info!(
            udp_rtt = %udp,
            tcp_rtt_ms = format_args!("{:.1}", tcp.rtt_ms().unwrap_or(0.0)),
            ping_retido_aqui = %held,
            "latência do último minuto (ms: mediana/p95/máx)"
        );
    }
}

/// "mediana/p95/máx" em ms de amostras em us; "-" sem amostras.
fn summary(mut us: Vec<u32>) -> String {
    if us.is_empty() {
        return "-".into();
    }
    us.sort_unstable();
    let at = |q: f64| f64::from(us[((us.len() - 1) as f64 * q).round() as usize]) / 1000.0;
    format!("{:.1}/{:.1}/{:.1}", at(0.5), at(0.95), at(1.0))
}

/// Backoff exponencial de 250 ms a 5 s.
#[derive(Debug)]
pub struct Backoff {
    next_ms: u64,
}

impl Backoff {
    pub const MIN_MS: u64 = 250;
    pub const MAX_MS: u64 = 5000;

    pub fn new() -> Self {
        Self {
            next_ms: Self::MIN_MS,
        }
    }

    pub fn reset(&mut self) {
        self.next_ms = Self::MIN_MS;
    }

    pub fn next_delay(&mut self) -> std::time::Duration {
        let d = self.next_ms;
        self.next_ms = (d * 2).min(Self::MAX_MS);
        std::time::Duration::from_millis(d)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_is_zero_for_constant_transit_and_converges_to_the_variation() {
        let mut j = Jitter::default();
        for _ in 0..10 {
            j.update(500.0);
        }
        assert_eq!(j.value_us(), 0.0);
        for i in 0..400 {
            j.update(if i % 2 == 0 { 500.0 } else { 600.0 });
        }
        assert!((j.value_us() - 100.0).abs() < 1.0, "{}", j.value_us());
    }

    #[test]
    fn rtt_do_eco_udp_vence_o_do_ping_tcp() {
        let mut t = RttTracker::default();
        t.sample(20_000);
        let fast = |rtt_us| FastStats {
            received: 1,
            jitter_us: 500.0,
            loss_pct: 0.0,
            rtt_us,
        };
        assert_eq!(t.snapshot(Some(fast(Some(4_000.0)))).rtt_ms, 4.0);
        assert_eq!(t.snapshot(Some(fast(None))).rtt_ms, 20.0);
        assert_eq!(t.snapshot(None).rtt_ms, 20.0);
    }

    #[test]
    fn resumo_mediana_p95_max() {
        assert_eq!(summary(Vec::new()), "-");
        let us: Vec<u32> = (1..=100).map(|i| i * 1000).collect();
        assert_eq!(summary(us), "51.0/95.0/100.0");
    }

    #[test]
    fn rtt_is_positive_even_for_zero_samples() {
        let mut t = RttTracker::default();
        t.sample(0);
        assert!(t.snapshot(None).rtt_ms > 0.0);
    }

    #[test]
    fn backoff_doubles_from_250_to_5000_and_resets() {
        let mut b = Backoff::new();
        let ms: Vec<u128> = (0..7).map(|_| b.next_delay().as_millis()).collect();
        assert_eq!(ms, [250, 500, 1000, 2000, 4000, 5000, 5000]);
        b.reset();
        assert_eq!(b.next_delay().as_millis(), 250);
    }
}
