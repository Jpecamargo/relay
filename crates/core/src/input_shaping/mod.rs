//! dono: C8
//! Modelagem de input: acumulador de scroll, pinça, rotação e sensibilidade por tela.
//! Nada aqui aloca no caminho do evento (`set`/construtores ficam fora dele).

use std::collections::HashMap;

use crate::types::KeyCombo;

/// Unidades de alta resolução por degrau de roda (1 linha = 1 degrau = 120).
pub const UNITS_PER_NOTCH: f64 = 120.0;

fn sat_i16(v: f64) -> i16 {
    v.clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

/// Converte pixels do trackpad em unidades de 1/120, carregando o resto.
#[derive(Debug)]
pub struct ScrollAccumulator {
    pub pixels_per_notch: f64,
    carry: f64,
}

impl Default for ScrollAccumulator {
    fn default() -> Self {
        Self::new(40.0)
    }
}

impl ScrollAccumulator {
    pub fn new(pixels_per_notch: f64) -> Self {
        Self {
            pixels_per_notch,
            carry: 0.0,
        }
    }

    /// Devolve as unidades i16 prontas para enviar (o resto fica acumulado).
    pub fn push(&mut self, delta_px: f64) -> i16 {
        if !(self.pixels_per_notch > 0.0) || !delta_px.is_finite() {
            return 0;
        }
        let total = delta_px / self.pixels_per_notch * UNITS_PER_NOTCH + self.carry;
        let out = sat_i16(total.trunc());
        self.carry = total - f64::from(out);
        out
    }

    /// Roda discreta: `lines` linhas viram `lines * 120`, saturando.
    pub fn push_lines(lines: f64) -> i16 {
        if !lines.is_finite() {
            return 0;
        }
        sat_i16((lines * UNITS_PER_NOTCH).round())
    }

    /// Descarta o resto (fim de gesto).
    pub fn reset(&mut self) {
        self.carry = 0.0;
    }
}

/// Inverso (receptor Windows -> Mac): unidades de 1/120 para pixels de scroll contínuo.
pub fn units_to_pixels(units: i16, pixels_per_notch: f64) -> f64 {
    f64::from(units) / UNITS_PER_NOTCH * pixels_per_notch
}

/// Inverso: unidades de 1/120 para linhas de scroll (roda).
pub fn units_to_lines(units: i16) -> f64 {
    f64::from(units) / UNITS_PER_NOTCH
}

/// Saída de um passo da pinça: `ctrl_down` só no primeiro passo do gesto; `wheel` em 1/120
/// (0 = nada a enviar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinchStep {
    pub ctrl_down: bool,
    pub wheel: i16,
}

/// Pinça como Ctrl+scroll: Ctrl down, ±120 por `pinch_per_notch` de magnificação, Ctrl up no fim.
#[derive(Debug)]
pub struct PinchTracker {
    pub pinch_per_notch: f64,
    acc: f64,
    active: bool,
}

impl Default for PinchTracker {
    fn default() -> Self {
        Self::new(0.1)
    }
}

impl PinchTracker {
    pub fn new(pinch_per_notch: f64) -> Self {
        Self {
            pinch_per_notch,
            acc: 0.0,
            active: false,
        }
    }

    pub fn push(&mut self, delta: f64) -> PinchStep {
        let ctrl_down = !self.active;
        self.active = true;
        let mut wheel = 0;
        if self.pinch_per_notch > 0.0 && delta.is_finite() {
            self.acc += delta;
            let notches = (self.acc / self.pinch_per_notch).trunc();
            self.acc -= notches * self.pinch_per_notch;
            wheel = sat_i16(notches * UNITS_PER_NOTCH);
        }
        PinchStep { ctrl_down, wheel }
    }

    /// Fim do gesto: devolve `true` se há Ctrl up a enviar.
    pub fn end(&mut self) -> bool {
        let was = self.active;
        self.active = false;
        self.acc = 0.0;
        was
    }
}

/// Rotação mapeável para um atalho. Sem atalho configurado (padrão), a rotação é ignorada.
#[derive(Debug)]
pub struct RotationMapper {
    pub combo: Option<KeyCombo>,
    pub degrees_per_step: f64,
    acc: f64,
}

impl Default for RotationMapper {
    fn default() -> Self {
        Self {
            combo: None,
            degrees_per_step: 15.0,
            acc: 0.0,
        }
    }
}

impl RotationMapper {
    /// Devolve o atalho e quantas vezes disparar (sempre >= 1), ou `None`.
    pub fn push(&mut self, delta: f64) -> Option<(&KeyCombo, u32)> {
        let combo = self.combo.as_ref()?;
        if !(self.degrees_per_step > 0.0) || !delta.is_finite() {
            return None;
        }
        self.acc += delta.abs();
        let steps = (self.acc / self.degrees_per_step).trunc();
        if steps < 1.0 {
            return None;
        }
        self.acc -= steps * self.degrees_per_step;
        Some((combo, steps.min(f64::from(u32::MAX)) as u32))
    }
}

#[derive(Debug)]
struct Scale {
    multiplier: f64,
    rest_x: f64,
    rest_y: f64,
}

/// Multiplicador de movimento por `display_id`, com resto fracionário.
#[derive(Debug, Default)]
pub struct SensitivityScaler {
    by_display: HashMap<String, Scale>,
}

impl SensitivityScaler {
    pub fn set(&mut self, display_id: &str, multiplier: f32) {
        self.by_display.insert(
            display_id.to_owned(),
            Scale {
                multiplier: f64::from(multiplier),
                rest_x: 0.0,
                rest_y: 0.0,
            },
        );
    }

    /// Aplica a sensibilidade da tela a um delta, saturando em i16.
    /// Tela sem multiplicador configurado usa 1,0.
    pub fn scale(&mut self, display_id: &str, dx: i32, dy: i32) -> (i16, i16) {
        let Some(s) = self.by_display.get_mut(display_id) else {
            return (sat_i16(f64::from(dx)), sat_i16(f64::from(dy)));
        };
        let apply = |d: i32, rest: &mut f64| {
            let v = f64::from(d) * s.multiplier + *rest;
            let t = v.trunc();
            *rest = v - t;
            sat_i16(t)
        };
        (apply(dx, &mut s.rest_x), apply(dy, &mut s.rest_y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Modifier;

    /// LCG determinístico (sem dependência de proptest).
    struct Lcg(u64);
    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn scroll_sum_within_one_unit_of_ideal() {
        let mut rng = Lcg(42);
        for case in 0..1000 {
            let ppn = 10.0 + rng.next_f64() * 70.0;
            let mut acc = ScrollAccumulator::new(ppn);
            let (mut total_px, mut sum) = (0.0, 0i64);
            for _ in 0..(1 + case % 50) {
                let d = (rng.next_f64() - 0.5) * 30.0;
                total_px += d;
                sum += i64::from(acc.push(d));
            }
            let ideal = total_px / ppn * 120.0;
            assert!((sum as f64 - ideal).abs() <= 1.0, "case {case}");
        }
    }

    #[test]
    fn default_ppn_and_discrete_wheel() {
        assert_eq!(ScrollAccumulator::default().pixels_per_notch, 40.0);
        assert_eq!(ScrollAccumulator::push_lines(1.0), 120);
        assert_eq!(ScrollAccumulator::push_lines(-2.0), -240);
        assert_eq!(ScrollAccumulator::push_lines(1000.0), i16::MAX);
    }

    #[test]
    fn inverse_to_pixels_and_lines() {
        assert_eq!(units_to_pixels(120, 40.0), 40.0);
        assert_eq!(units_to_pixels(-60, 40.0), -20.0);
        assert_eq!(units_to_lines(240), 2.0);
        assert_eq!(units_to_lines(-30), -0.25);
    }

    #[test]
    fn pinch_exact_sequence() {
        let mut p = PinchTracker::new(1.0);
        let a = p.push(0.6);
        assert_eq!(
            a,
            PinchStep {
                ctrl_down: true,
                wheel: 0
            }
        );
        let b = p.push(0.6);
        assert_eq!(
            b,
            PinchStep {
                ctrl_down: false,
                wheel: 120
            }
        );
        let c = p.push(-2.0); // acc 0.2 - 2.0 = -1.8 -> -1 passo
        assert_eq!(
            c,
            PinchStep {
                ctrl_down: false,
                wheel: -120
            }
        );
        assert!(p.end());
        assert!(!p.end());
        // novo gesto volta a começar com Ctrl down
        assert!(p.push(0.1).ctrl_down);
    }

    #[test]
    fn rotation_default_none_and_mapped() {
        let mut r = RotationMapper::default();
        assert!(r.push(90.0).is_none());
        r.combo = Some(KeyCombo {
            modifiers: vec![Modifier::Ctrl],
            key: "KeyR".into(),
        });
        assert!(r.push(10.0).is_none());
        let (combo, n) = r.push(10.0).unwrap(); // 20 acumulados, 15 por passo
        assert_eq!((combo.key.as_str(), n), ("KeyR", 1));
        assert_eq!(r.push(-70.0).unwrap().1, 5); // 5 + 70 = 75
    }

    #[test]
    fn sensitivity_fractional_and_saturation() {
        let mut s = SensitivityScaler::default();
        s.set("d1", 1.5);
        let sum: i32 = (0..1000).map(|_| i32::from(s.scale("d1", 1, 0).0)).sum();
        assert!((sum - 1500).abs() <= 1);
        // tela desconhecida: multiplicador 1
        assert_eq!(s.scale("x", 7, -7), (7, -7));
        s.set("d2", 2.0);
        assert_eq!(s.scale("d2", 30000, -30000), (i16::MAX, i16::MIN));
        assert_eq!(s.scale("d2", i32::MAX, i32::MIN), (i16::MAX, i16::MIN));
    }
}
