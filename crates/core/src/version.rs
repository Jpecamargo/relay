//! Comparação de versões entre os dois apps: quem está atrás se atualiza (Mac e PC sempre iguais).

use semver::Version;

use crate::types::{Message, VersionSync};

/// Versão deste build; os apps herdam a do workspace.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Primeira versão do agente que decodifica `Message::RequestUpdate`. Variante desconhecida
/// derruba a sessão, então o Mac não envia o pedido a quem é anterior a esta.
pub const UPDATE_REQUEST_MIN_PEER: &str = "0.1.8";

/// Versão informada no Hello, ou `None` se vazia ou que não seja semver (peer que não informa).
pub fn parse_version(raw: &str) -> Option<Version> {
    Version::parse(raw.trim().trim_start_matches('v')).ok()
}

/// Quem está atrás, vendo daqui. Sem versão do par, ou com versão ilegível, nada muda.
pub fn version_sync(local: &str, peer: Option<&str>) -> VersionSync {
    let (Some(local), Some(peer)) = (parse_version(local), peer.and_then(parse_version)) else {
        return VersionSync::InSync;
    };
    match local.cmp(&peer) {
        std::cmp::Ordering::Equal => VersionSync::InSync,
        std::cmp::Ordering::Less => VersionSync::LocalBehind,
        std::cmp::Ordering::Greater => {
            let knows_request =
                parse_version(UPDATE_REQUEST_MIN_PEER).is_some_and(|min| peer >= min);
            if knows_request {
                VersionSync::PeerBehind
            } else {
                VersionSync::PeerBehindManual
            }
        }
    }
}

/// Pedido que o Mac manda ao agente ao conectar, se (e só se) o agente está atrás e sabe recebê-lo.
pub fn update_request(local: &str, peer: Option<&str>) -> Option<Message> {
    (version_sync(local, peer) == VersionSync::PeerBehind).then(|| Message::RequestUpdate {
        version: local.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use VersionSync::*;

    #[test]
    fn the_older_side_is_the_one_that_updates() {
        assert_eq!(version_sync("0.1.8", Some("0.1.8")), InSync);
        assert_eq!(version_sync("0.1.8", Some("0.2.0")), LocalBehind);
        assert_eq!(version_sync("0.1.9", Some("0.1.8")), PeerBehind);
        assert_eq!(version_sync("0.1.10", Some("0.1.9")), PeerBehind);
        assert_eq!(version_sync("0.1.9", Some("0.1.10")), LocalBehind);
        assert_eq!(version_sync("0.2.0-beta.1", Some("0.2.0")), LocalBehind);
    }

    #[test]
    fn a_peer_too_old_for_the_request_is_updated_by_hand() {
        assert_eq!(version_sync("0.1.8", Some("0.1.7")), PeerBehindManual);
        assert_eq!(version_sync("0.2.0", Some("0.1.0")), PeerBehindManual);
    }

    #[test]
    fn only_a_newer_mac_with_an_agent_that_understands_asks_for_the_update() {
        let asked = Some(Message::RequestUpdate {
            version: "0.2.0".into(),
        });
        assert_eq!(update_request("0.2.0", Some("0.1.8")), asked);
        assert_eq!(update_request("0.2.0", Some("0.1.7")), None);
        assert_eq!(update_request("0.2.0", Some("0.2.0")), None);
        assert_eq!(update_request("0.2.0", Some("0.3.0")), None);
        assert_eq!(update_request("0.2.0", None), None);
    }

    #[test]
    fn a_peer_without_a_readable_version_changes_nothing() {
        assert_eq!(version_sync("0.2.0", None), InSync);
        assert_eq!(version_sync("0.2.0", Some("")), InSync);
        assert_eq!(version_sync("0.2.0", Some("fake")), InSync);
        assert_eq!(version_sync("0.2.0", Some("  v0.2.0 ")), InSync);
    }
}
