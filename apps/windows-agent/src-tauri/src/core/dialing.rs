//! Quando este PC disca (D41): a decisão sobre cada par descoberto e a espera do fallback.
//! Lógica pura, sem rede nem relógio próprio, para testar sem a plataforma.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use relay_core::node::session::{DialDecision, dial_decision};
use relay_core::types::{Caps, DiscoveredPeer, Os};

pub(super) struct DialPlanner {
    fallback: Duration,
    /// Par que o outro lado deveria discar: só disca depois de `fallback` sem sessão.
    waiting: Option<(DiscoveredPeer, Instant)>,
}

impl DialPlanner {
    pub(super) fn new(fallback: Duration) -> Self {
        Self {
            fallback,
            waiting: None,
        }
    }

    /// Par visto pelo mDNS. `Some` = disca agora. Só vale o par pareado (o ativo) e nunca o
    /// próprio id; sem pareamento ninguém é discado.
    pub(super) fn on_discovered(
        &mut self,
        local_id: &str,
        paired_id: Option<&str>,
        peer: &DiscoveredPeer,
        now: Instant,
    ) -> Option<DiscoveredPeer> {
        if paired_id != Some(peer.device_id.as_str()) {
            return None;
        }
        match dial_decision(local_id, peer) {
            DialDecision::Ignore => None,
            DialDecision::DialNow => {
                self.waiting = None;
                Some(peer.clone())
            }
            DialDecision::DialAfterFallback => {
                match &mut self.waiting {
                    // A mesma espera segue (o mDNS repete o anúncio): só atualiza o endereço.
                    Some((p, _)) if p.device_id == peer.device_id => *p = peer.clone(),
                    _ => self.waiting = Some((peer.clone(), now + self.fallback)),
                }
                None
            }
        }
    }

    /// Relógio do pump: `Some` = o fallback venceu sem sessão de pé, disca. Com sessão de pé a
    /// espera recomeça (são 10 s sem sessão, não 10 s desde o anúncio).
    pub(super) fn due(&mut self, now: Instant, session_up: bool) -> Option<DiscoveredPeer> {
        let (_, at) = self.waiting.as_mut()?;
        if now < *at {
            return None;
        }
        if session_up {
            *at = now + self.fallback;
            return None;
        }
        self.waiting.take().map(|(p, _)| p)
    }

    /// Pareamento desfeito ou trocado.
    pub(super) fn forget(&mut self) {
        self.waiting = None;
    }
}

/// Endereço de controle do par: IPv4 antes de IPv6 e link-local por último.
pub(super) fn target_of(peer: &DiscoveredPeer) -> Option<SocketAddr> {
    let mut ips: Vec<IpAddr> = peer.addrs.iter().filter_map(|a| a.parse().ok()).collect();
    ips.sort_by_key(|ip| (!ip.is_ipv4(), is_link_local(ip)));
    ips.first().map(|ip| SocketAddr::new(*ip, peer.port))
}

fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// `Caps` deste PC: o baseline do Windows, sem MIC_SINK se o VB-CABLE não foi detectado.
pub(super) fn local_caps(vbcable_detected: bool) -> Caps {
    let mut caps = Caps::baseline(Os::Windows);
    if !vbcable_detected {
        caps.features &= !Caps::MIC_SINK;
    }
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, version: Option<&str>) -> DiscoveredPeer {
        DiscoveredPeer {
            name: "mac".into(),
            addrs: vec!["fe80::1".into(), "192.168.0.5".into()],
            port: 47800,
            device_id: id.into(),
            protocol_version: 1,
            os: Some(Os::Mac),
            app_version: version.map(Into::into),
        }
    }

    const FALLBACK: Duration = Duration::from_secs(10);

    #[test]
    fn the_smaller_id_dials_now_and_the_larger_waits_for_the_fallback() {
        let t0 = Instant::now();
        let mut p = DialPlanner::new(FALLBACK);
        let mac = peer("bb", Some("0.3.0"));
        assert!(p.on_discovered("aa", Some("bb"), &mac, t0).is_some());

        let mut p = DialPlanner::new(FALLBACK);
        assert!(p.on_discovered("cc", Some("bb"), &mac, t0).is_none());
        assert!(p.due(t0 + Duration::from_secs(9), false).is_none());
        let due = p.due(t0 + FALLBACK, false).expect("fallback venceu");
        assert_eq!(due.device_id, "bb");
        assert!(p.due(t0 + FALLBACK * 2, false).is_none(), "uma vez só");
    }

    #[test]
    fn a_session_up_restarts_the_wait() {
        let t0 = Instant::now();
        let mut p = DialPlanner::new(FALLBACK);
        p.on_discovered("cc", Some("bb"), &peer("bb", Some("0.3.0")), t0);
        assert!(p.due(t0 + FALLBACK, true).is_none());
        assert!(
            p.due(t0 + FALLBACK + Duration::from_secs(9), false)
                .is_none()
        );
        assert!(p.due(t0 + FALLBACK * 2, false).is_some());
    }

    #[test]
    fn repeated_announcements_do_not_push_the_deadline() {
        let t0 = Instant::now();
        let mut p = DialPlanner::new(FALLBACK);
        let mac = peer("bb", Some("0.3.0"));
        p.on_discovered("cc", Some("bb"), &mac, t0);
        p.on_discovered("cc", Some("bb"), &mac, t0 + Duration::from_secs(8));
        assert!(p.due(t0 + FALLBACK, false).is_some());
    }

    #[test]
    fn only_the_paired_peer_is_dialed_and_never_ourselves() {
        let t0 = Instant::now();
        let mut p = DialPlanner::new(FALLBACK);
        let other = peer("dd", Some("0.3.0"));
        assert!(p.on_discovered("aa", Some("bb"), &other, t0).is_none());
        assert!(p.on_discovered("aa", None, &other, t0).is_none());
        assert!(
            p.on_discovered("aa", Some("aa"), &peer("aa", Some("0.3.0")), t0)
                .is_none()
        );
    }

    #[test]
    fn a_peer_without_version_is_dialed_now_whatever_the_ids() {
        let mut p = DialPlanner::new(FALLBACK);
        assert!(
            p.on_discovered("zz", Some("bb"), &peer("bb", None), Instant::now())
                .is_some()
        );
    }

    #[test]
    fn target_prefers_ipv4_over_link_local_ipv6() {
        let t = target_of(&peer("bb", None)).unwrap();
        assert_eq!(t, "192.168.0.5:47800".parse().unwrap());
        let mut none = peer("bb", None);
        none.addrs.clear();
        assert!(target_of(&none).is_none());
    }

    #[test]
    fn mic_sink_goes_away_without_the_vbcable() {
        assert!(local_caps(true).has(Caps::MIC_SINK));
        let caps = local_caps(false);
        assert!(!caps.has(Caps::MIC_SINK));
        assert_eq!(caps.features | Caps::MIC_SINK, local_caps(true).features);
    }
}
