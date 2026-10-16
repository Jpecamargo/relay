//! dono: F2
//! Eventos de input: o evento cru da captura (`CapturedEvent`) e o do protocolo (`InputEvent`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

/// Teclas de mídia enviadas ao destino (D21: brilho nunca é enviado).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MediaKey {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    Next,
    Previous,
}

/// Fase de um gesto de scroll. No fio (FastPacket) vira um u8 por `as_u8`/`from_u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ScrollPhase {
    None,
    Began,
    Changed,
    Ended,
    Momentum,
}

impl ScrollPhase {
    pub const fn as_u8(self) -> u8 {
        match self {
            ScrollPhase::None => 0,
            ScrollPhase::Began => 1,
            ScrollPhase::Changed => 2,
            ScrollPhase::Ended => 3,
            ScrollPhase::Momentum => 4,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ScrollPhase::None),
            1 => Some(ScrollPhase::Began),
            2 => Some(ScrollPhase::Changed),
            3 => Some(ScrollPhase::Ended),
            4 => Some(ScrollPhase::Momentum),
            _ => None,
        }
    }
}

/// Código de tecla cru da captura: keycode virtual do macOS ou scan code do Windows
/// (set 1, com 0xE0xx para teclas estendidas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum KeyCode {
    Mac(u16),
    Scan(u16),
}

/// Evento cru da captura. `Copy` e de tamanho fixo, para trafegar num `rtrb` sem alocar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CapturedEvent {
    /// `flags`: máscara crua de modificadores da plataforma (CGEventFlags no Mac; 0 no Windows).
    Key {
        code: KeyCode,
        down: bool,
        flags: u64,
        autorepeat: bool,
    },
    /// `dx`/`dy`: delta de hardware; `pos`: posição absoluta do cursor no espaço de coordenadas
    /// da máquina de origem (Mac: pontos; Windows: pixels físicos).
    MouseMove {
        dx: i32,
        dy: i32,
        pos: (f64, f64),
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    /// `continuous=true`: deltas em pixels (trackpad). `false`: roda discreta, deltas em linhas.
    Scroll {
        dx_px: f64,
        dy_px: f64,
        phase: ScrollPhase,
        continuous: bool,
    },
    Media {
        key: MediaKey,
        down: bool,
    },
    Magnify {
        delta: f64,
    },
    Rotate {
        delta: f64,
    },
}

/// Evento de input no protocolo (canal rápido para MouseMove/Scroll, confiável para o resto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum InputEvent {
    MouseMove {
        dx: i16,
        dy: i16,
    },
    MouseAbs {
        x: i32,
        y: i32,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    /// `dx`/`dy` em 1/120 de degrau (alta resolução).
    Scroll {
        dx: i16,
        dy: i16,
        phase: ScrollPhase,
    },
    /// `scan`: set 1 do Windows; 0xE0xx = tecla estendida.
    Key {
        scan: u16,
        down: bool,
    },
    Media {
        key: MediaKey,
        down: bool,
    },
}
