//! Relógio monotônico e estimadores de rede (também usado pelas threads do `fast`, que não têm runtime assíncrono).

use std::sync::OnceLock;
use std::time::Instant;

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

    /// `fast_*`: jitter (us) e perda (%) medidos no canal rápido, quando já chegou pacote nele.
    pub fn snapshot(&self, fast: Option<(f64, f32)>) -> LatencyStats {
        let (jitter_us, loss_pct) = match fast {
            Some((j, l)) => (j, l),
            None => (self.jitter.value_us(), 0.0),
        };
        LatencyStats {
            rtt_ms: self.rtt_ms.unwrap_or(0.0) as f32,
            jitter_ms: (jitter_us / 1000.0) as f32,
            loss_pct,
        }
    }
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
