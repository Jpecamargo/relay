//! Sistema operacional de uma máquina e o lado relativo (este computador x o par).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    /// SO deste build.
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    /// Texto usado no anúncio mDNS (chave `os`).
    pub const fn as_txt(self) -> &'static str {
        match self {
            Os::Mac => "mac",
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }

    /// Inverso de [`Os::as_txt`]; texto desconhecido devolve `None`.
    pub fn from_txt(txt: &str) -> Option<Os> {
        match txt {
            "mac" => Some(Os::Mac),
            "windows" => Some(Os::Windows),
            "linux" => Some(Os::Linux),
            _ => None,
        }
    }
}

/// Lado relativo usado na IPC: `Local` é este computador, `Peer` o par.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Host {
    Local,
    Peer,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_text_round_trips_and_rejects_unknown() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            assert_eq!(Os::from_txt(os.as_txt()), Some(os));
        }
        assert_eq!(Os::from_txt("Mac"), None);
        assert_eq!(Os::from_txt(""), None);
    }

    #[test]
    fn current_matches_the_build_target() {
        #[cfg(target_os = "macos")]
        assert_eq!(Os::current(), Os::Mac);
        #[cfg(target_os = "windows")]
        assert_eq!(Os::current(), Os::Windows);
    }
}
