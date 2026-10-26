//! dono: W4b
//! Conversão pura de áudio para a reprodução WASAPI, testada no Mac.

/// Preenche `out` (bytes f32 little-endian, estéreo intercalado) com amostras de `next`
/// multiplicadas por `gain`; o que faltar vira silêncio (underrun). `out.len()` deve ser
/// múltiplo de 4. Devolve quantas amostras reais foram consumidas de `next`.
pub fn fill_f32_le(out: &mut [u8], gain: f32, mut next: impl FnMut() -> Option<f32>) -> usize {
    let mut taken = 0;
    let mut dry = false;
    for dst in out.chunks_exact_mut(4) {
        let v = if dry {
            0.0
        } else {
            match next() {
                Some(v) => {
                    taken += 1;
                    v * gain
                }
                None => {
                    dry = true;
                    0.0
                }
            }
        };
        dst.copy_from_slice(&v.to_le_bytes());
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_f32_le_gain_and_underrun() {
        let mut src = vec![1.0f32, -1.0, 0.5].into_iter();
        let mut out = [0xFFu8; 20];
        let taken = fill_f32_le(&mut out, 0.5, || src.next());
        assert_eq!(taken, 3);
        let v: Vec<f32> = out
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(v, vec![0.5, -0.5, 0.25, 0.0, 0.0]);
    }
}
