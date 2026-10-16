//! dono: W1
//! Desliga e restaura a "Aprimorar precisão do ponteiro" (D27). Stub do F2.

/// Salva SPI_GETMOUSE, zera a aceleração e restaura no `Drop`.
#[derive(Debug, Default)]
pub struct PointerPrecisionGuard {}
