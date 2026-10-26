//! dono: F2
//! Pareamento e descoberta.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::os::Os;

/// Peer pareado, guardado no KeyStore na conta `peer:<device_id>`.
/// `device_id` = hex dos 8 primeiros bytes do BLAKE2s da chave pública.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairedPeer {
    pub device_id: String,
    pub name: String,
    pub public_key: [u8; 32],
    pub mac_address: [u8; 6],
}

/// Peer encontrado pelo mDNS (ou construído a partir do endereço manual).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiscoveredPeer {
    pub name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub device_id: String,
    pub protocol_version: u16,
    /// Chave TXT `os`; `None` = peer 0.2 (não anuncia).
    pub os: Option<Os>,
    /// Chave TXT `v`; `None` = peer 0.2.
    pub app_version: Option<String>,
}
