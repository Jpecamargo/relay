//! Histograma em µs, impressão e JSON (escrito à mão: sem serde_json no pacote).

use hdrhistogram::Histogram;

pub struct Summary {
    pub channel: &'static str,
    pub sent: u32,
    pub received: u32,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
    pub mean: f64,
}

impl Summary {
    /// `rtts_us`: um valor por pacote devolvido (0 = não voltou).
    pub fn from_rtts(channel: &'static str, sent: u32, rtts_us: &[u64]) -> Self {
        let mut h = Histogram::<u64>::new_with_bounds(1, 600_000_000, 3).expect("limites válidos");
        let mut received = 0;
        for &r in rtts_us.iter().filter(|&&r| r != 0) {
            received += 1;
            let _ = h.record(r.max(1));
        }
        let q = |p: f64| {
            if received == 0 {
                0
            } else {
                h.value_at_quantile(p)
            }
        };
        Self {
            channel,
            sent,
            received,
            p50: q(0.50),
            p95: q(0.95),
            p99: q(0.99),
            max: if received == 0 { 0 } else { h.max() },
            mean: if received == 0 { 0.0 } else { h.mean() },
        }
    }

    pub fn lost(&self) -> u32 {
        self.sent.saturating_sub(self.received)
    }

    pub fn print(&self) {
        println!(
            "{:<9} enviados {:>6}  recebidos {:>6}  perdidos {:>5}  p50 {:>7} us  p95 {:>7} us  p99 {:>7} us  max {:>8} us",
            self.channel,
            self.sent,
            self.received,
            self.lost(),
            self.p50,
            self.p95,
            self.p99,
            self.max
        );
    }

    fn json(&self) -> String {
        format!(
            "\"{}\":{{\"sent\":{},\"received\":{},\"lost\":{},\"p50_us\":{},\"p95_us\":{},\"p99_us\":{},\"max_us\":{},\"mean_us\":{:.1}}}",
            self.channel,
            self.sent,
            self.received,
            self.lost(),
            self.p50,
            self.p95,
            self.p99,
            self.max,
            self.mean
        )
    }
}

pub fn to_json(mode: &str, rate_hz: u32, fast: &Summary, reliable: &Summary) -> String {
    format!(
        "{{\"mode\":\"{mode}\",\"rate_hz\":{rate_hz},{},{}}}\n",
        fast.json(),
        reliable.json()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_loss() {
        let mut rtts: Vec<u64> = (1..=100).collect();
        rtts.push(0);
        let s = Summary::from_rtts("fast", 101, &rtts);
        assert_eq!((s.received, s.lost()), (100, 1));
        assert!((49..=51).contains(&s.p50), "p50 = {}", s.p50);
        assert!(s.p99 >= 98 && s.max == 100);
    }

    #[test]
    fn json_has_both_channels() {
        let a = Summary::from_rtts("fast", 2, &[10, 20]);
        let b = Summary::from_rtts("reliable", 2, &[30, 0]);
        let j = to_json("send", 500, &a, &b);
        assert!(j.contains("\"fast\":{\"sent\":2") && j.contains("\"reliable\":{\"sent\":2"));
        assert!(j.contains("\"lost\":1"));
    }
}
