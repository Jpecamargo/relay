//! Comparação de versões entre os dois apps: quem está atrás se atualiza (Mac e PC sempre iguais).

use semver::Version;

use crate::types::{Message, Os, VersionSync};

/// Versão deste build; os apps herdam a do workspace.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Primeira versão do agente que decodifica `Message::RequestUpdate`. Variante desconhecida
/// derruba a sessão, então o Mac não envia o pedido a quem é anterior a esta.
pub const UPDATE_REQUEST_MIN_PEER: &str = "0.1.8";

/// Primeira versão com o núcleo simétrico (Caps, livro de layouts, descoberta nos dois sentidos).
/// Tudo que é novo na 0.3 só vai a peer nesta versão ou acima.
pub const SYMMETRIC_MIN_PEER: &str = "0.3.0";

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

/// `true` se o peer é >= `SYMMETRIC_MIN_PEER`. Sem versão, ou com versão ilegível, é `false`.
pub fn peer_is_symmetric(peer_version: Option<&str>) -> bool {
    match (
        peer_version.and_then(parse_version),
        parse_version(SYMMETRIC_MIN_PEER),
    ) {
        (Some(peer), Some(min)) => peer >= min,
        _ => false,
    }
}

/// Pedido de atualização ao par, se (e só se) ele está atrás e sabe recebê-lo. Um Mac < 0.3.0 não
/// trata `RequestUpdate` (D49): ele mesmo mostra "atualize" ao ver a versão maior.
pub fn update_request(local: &str, peer_version: Option<&str>, peer_os: Os) -> Option<Message> {
    if peer_os == Os::Mac && !peer_is_symmetric(peer_version) {
        return None;
    }
    (version_sync(local, peer_version) == VersionSync::PeerBehind).then(|| Message::RequestUpdate {
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
        assert_eq!(update_request("0.2.0", Some("0.1.8"), Os::Windows), asked);
        assert_eq!(update_request("0.2.0", Some("0.1.7"), Os::Windows), None);
        assert_eq!(update_request("0.2.0", Some("0.2.0"), Os::Windows), None);
        assert_eq!(update_request("0.2.0", Some("0.3.0"), Os::Windows), None);
        assert_eq!(update_request("0.2.0", None, Os::Windows), None);
    }

    #[test]
    fn symmetric_means_a_readable_version_from_0_3_0() {
        assert!(peer_is_symmetric(Some("0.3.0")));
        assert!(peer_is_symmetric(Some(" v0.3.1 ")));
        assert!(peer_is_symmetric(Some("1.0.0")));
        assert!(!peer_is_symmetric(Some("0.2.9")));
        assert!(!peer_is_symmetric(Some("0.3.0-beta.1")));
        assert!(!peer_is_symmetric(Some("fake")));
        assert!(!peer_is_symmetric(Some("")));
        assert!(!peer_is_symmetric(None));
    }

    #[test]
    fn a_mac_older_than_0_3_0_is_never_asked_to_update() {
        // 0.2.x do Mac não trata RequestUpdate (D49), mesmo sendo >= UPDATE_REQUEST_MIN_PEER.
        assert_eq!(update_request("0.3.0", Some("0.2.0"), Os::Mac), None);
        assert_eq!(
            update_request("0.3.1", Some("0.3.0"), Os::Mac).is_some(),
            true
        );
        // O PC 0.2 continua recebendo o pedido (regra de UPDATE_REQUEST_MIN_PEER).
        assert!(update_request("0.3.0", Some("0.2.0"), Os::Windows).is_some());
        assert_eq!(update_request("0.3.0", Some("0.1.7"), Os::Windows), None);
    }

    #[test]
    fn a_peer_without_a_readable_version_changes_nothing() {
        assert_eq!(version_sync("0.2.0", None), InSync);
        assert_eq!(version_sync("0.2.0", Some("")), InSync);
        assert_eq!(version_sync("0.2.0", Some("fake")), InSync);
        assert_eq!(version_sync("0.2.0", Some("  v0.2.0 ")), InSync);
    }
}
