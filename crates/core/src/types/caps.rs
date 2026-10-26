//! Capacidades de uma máquina como bitset: o painel esconde o que falta.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::os::Os;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Caps {
    pub os: Os,
    pub features: u32,
}

impl Caps {
    pub const COMMANDS: u32 = 1 << 0;
    pub const VOLUME: u32 = 1 << 1;
    pub const SCREEN_CAPTURE: u32 = 1 << 2;
    pub const METRICS: u32 = 1 << 3;
    pub const MIC_SINK: u32 = 1 << 4;
    pub const MIC_SOURCE: u32 = 1 << 5;
    pub const FOREGROUND_APP: u32 = 1 << 6;
    pub const WOL_TARGET: u32 = 1 << 7;
    pub const DEFERRED_FILE_LIST: u32 = 1 << 8;
    pub const FILE_PULL: u32 = 1 << 9;

    const ALL: u32 = (1 << 10) - 1;

    /// `true` se todos os bits de `bit` estão ligados.
    pub const fn has(&self, bit: u32) -> bool {
        self.features & bit == bit
    }

    /// Capacidades de um peer que não as anuncia (0.2.x) ou o ponto de partida local.
    pub const fn baseline(os: Os) -> Caps {
        let features = match os {
            Os::Windows => Self::ALL & !(Self::MIC_SOURCE | Self::FILE_PULL),
            Os::Mac => Self::MIC_SOURCE | Self::FILE_PULL,
            Os::Linux => 0,
        };
        Caps { os, features }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_baseline_has_everything_but_mic_source_and_file_pull() {
        let caps = Caps::baseline(Os::Windows);
        assert_eq!(caps.os, Os::Windows);
        for bit in [
            Caps::COMMANDS,
            Caps::VOLUME,
            Caps::SCREEN_CAPTURE,
            Caps::METRICS,
            Caps::MIC_SINK,
            Caps::FOREGROUND_APP,
            Caps::WOL_TARGET,
            Caps::DEFERRED_FILE_LIST,
        ] {
            assert!(caps.has(bit), "{bit}");
        }
        assert!(!caps.has(Caps::MIC_SOURCE));
        assert!(!caps.has(Caps::FILE_PULL));
    }

    #[test]
    fn mac_baseline_is_mic_source_and_file_pull() {
        let caps = Caps::baseline(Os::Mac);
        assert_eq!(caps.features, Caps::MIC_SOURCE | Caps::FILE_PULL);
        assert!(caps.has(Caps::MIC_SOURCE) && caps.has(Caps::FILE_PULL));
        assert!(!caps.has(Caps::COMMANDS));
    }

    #[test]
    fn linux_baseline_is_empty() {
        let caps = Caps::baseline(Os::Linux);
        assert_eq!((caps.os, caps.features), (Os::Linux, 0));
        assert!(!caps.has(Caps::COMMANDS));
    }
}
