//! Sessão simétrica: decisão de discagem, sessão duplicada e Caps.

use std::time::Duration;

use crate::types::{Caps, DiscoveredPeer, Message};
use crate::version::{SYMMETRIC_MIN_PEER, parse_version, peer_is_symmetric};

/// Quanto o lado de id maior espera o menor discar antes de discar ele mesmo.
pub const DIAL_FALLBACK: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialDecision {
    DialNow,
    DialAfterFallback,
    Ignore,
}

/// Quem disca para um peer descoberto. Peer 0.2 (sem versão) nunca disca de volta: disca-se já.
/// Entre dois 0.3 disca o de menor `device_id`; o outro espera `DIAL_FALLBACK`.
pub fn dial_decision(local_id: &str, peer: &DiscoveredPeer) -> DialDecision {
    if peer.device_id == local_id {
        return DialDecision::Ignore;
    }
    let symmetric = peer
        .app_version
        .as_deref()
        .and_then(parse_version)
        .zip(parse_version(SYMMETRIC_MIN_PEER))
        .is_some_and(|(v, min)| v >= min);
    if !symmetric || local_id < peer.device_id.as_str() {
        DialDecision::DialNow
    } else {
        DialDecision::DialAfterFallback
    }
}

/// Com duas sessões para o mesmo par, fica a cujo dialer tem o menor `device_id`; empate mantém
/// a existente. Os dois lados avaliam a mesma dupla e chegam à mesma sessão.
pub fn keep_new_session(existing_dialer_id: &str, new_dialer_id: &str) -> bool {
    new_dialer_id < existing_dialer_id
}

/// `Caps` local, só para peer simétrico (um 0.2 derruba a sessão ao ver a variante).
pub fn caps_message(local: Caps, peer_app_version: Option<&str>) -> Option<Message> {
    peer_is_symmetric(peer_app_version).then_some(Message::Caps(local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Os;

    fn peer(id: &str, version: Option<&str>) -> DiscoveredPeer {
        DiscoveredPeer {
            name: "x".into(),
            addrs: vec![],
            port: 1,
            device_id: id.into(),
            protocol_version: 1,
            os: None,
            app_version: version.map(Into::into),
        }
    }

    #[test]
    fn own_id_is_ignored() {
        assert_eq!(
            dial_decision("aa", &peer("aa", Some("0.3.0"))),
            DialDecision::Ignore
        );
    }

    #[test]
    fn peer_without_version_is_dialed_now() {
        assert_eq!(
            dial_decision("zz", &peer("aa", None)),
            DialDecision::DialNow
        );
        assert_eq!(
            dial_decision("zz", &peer("aa", Some("0.2.9"))),
            DialDecision::DialNow
        );
    }

    #[test]
    fn symmetric_peer_is_dialed_by_the_smaller_id_only() {
        assert_eq!(
            dial_decision("aa", &peer("bb", Some("0.3.0"))),
            DialDecision::DialNow
        );
        assert_eq!(
            dial_decision("bb", &peer("aa", Some("0.3.1"))),
            DialDecision::DialAfterFallback
        );
    }

    #[test]
    fn both_sides_keep_the_same_session() {
        // A e B discam um ao outro; cada lado vê a sua sessão (dialer = ele) e a do outro.
        let (a, b) = ("aa", "bb");
        // Lado A: existente é a que ele discou (a), nova é a que B discou (b).
        let a_keeps_new = keep_new_session(a, b);
        // Lado B: existente é a que ele discou (b), nova é a que A discou (a).
        let b_keeps_new = keep_new_session(b, a);
        // A mantém a que A discou; B troca para a que A discou.
        assert!(!a_keeps_new && b_keeps_new);
        // Ordem inversa de chegada: mesma escolha (a sessão de dialer "aa").
        assert!(keep_new_session(b, a) && !keep_new_session(a, b));
        assert!(!keep_new_session("aa", "aa"));
    }

    #[test]
    fn caps_only_to_symmetric_peers() {
        let caps = Caps::baseline(Os::Mac);
        assert_eq!(caps_message(caps, Some("0.3.0")), Some(Message::Caps(caps)));
        assert_eq!(caps_message(caps, Some("0.2.9")), None);
        assert_eq!(caps_message(caps, None), None);
    }
}
