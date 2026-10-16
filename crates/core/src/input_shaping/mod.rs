//! dono: C8
//! Modelagem de input: acumulador de scroll, pinça, rotação e sensibilidade por tela.
//! Stubs do F2: assinaturas.

use std::collections::HashMap;

/// Converte pixels do trackpad em unidades de 1/120, carregando o resto.
#[derive(Debug)]
pub struct ScrollAccumulator {
    pub pixels_per_notch: f64,
}

impl Default for ScrollAccumulator {
    fn default() -> Self {
        Self {
            pixels_per_notch: 40.0,
        }
    }
}

impl ScrollAccumulator {
    /// Devolve as unidades i16 prontas para enviar (o resto fica acumulado).
    pub fn push(&mut self, _delta_px: f64) -> i16 {
        0
    }
}

/// Multiplicador de movimento por `display_id`, com resto fracionário.
#[derive(Debug, Default)]
pub struct SensitivityScaler {
    _by_display: HashMap<String, f32>,
}

impl SensitivityScaler {
    pub fn set(&mut self, _display_id: &str, _multiplier: f32) {}

    /// Aplica a sensibilidade da tela a um delta, saturando em i16.
    pub fn scale(&mut self, _display_id: &str, dx: i32, dy: i32) -> (i16, i16) {
        (
            dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        )
    }
}
