//! Gate por versão (I2) com bytes 0.2 reais e discagem a partir do que a descoberta (I3) anuncia.

use relay_core::compat::v02::allowed_for_peer;
use relay_core::layout::profile_key;
use relay_core::node::layout_host::{LayoutEffect, LayoutHostState};
use relay_core::node::session::{DialDecision, caps_message, dial_decision, keep_new_session};
use relay_core::protocol::{FrameReader, FrameWriter};
use relay_core::types::{
    Caps, DiscoveredPeer, DisplayInfo, Host, LayoutBook, LayoutConfig, LayoutProfile, Machine,
    Message, Os, ProfileKey,
};

const FIXTURE: &str = include_str!("../src/compat/testdata/v02_messages.hex");

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn fixture() -> Vec<(String, Vec<u8>)> {
    FIXTURE
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (n, h) = l.split_once(' ').unwrap();
            (n.to_owned(), from_hex(h.trim()))
        })
        .collect()
}

/// Um peer 0.2 só conhece as variantes do fixture: tag = índice da linha (0..=N-1). Frame =
/// u32 LE de comprimento + corpo cujo primeiro byte é a tag da variante.
fn tag_of(frame: &[u8]) -> u8 {
    let len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
    assert_eq!(frame.len(), 4 + len, "frame bem formado");
    frame[4]
}

fn v02_knows(frame: &[u8]) -> bool {
    (tag_of(frame) as usize) < fixture().len()
}

fn encode(m: &Message) -> Vec<u8> {
    let mut out = Vec::new();
    FrameWriter.encode(m, &mut out).unwrap();
    out
}

fn book() -> LayoutBook {
    LayoutBook {
        revision: 1,
        host: Machine::Mac,
        profiles: vec![],
        active: None,
    }
}

#[test]
fn o_fixture_0_2_tem_tags_contiguas_e_decodifica_no_codec_novo_byte_a_byte() {
    let fx = fixture();
    assert!(fx.len() >= 23);
    for (i, (name, bytes)) in fx.iter().enumerate() {
        assert_eq!(tag_of(bytes) as usize, i, "{name}");
        let msgs = FrameReader::default().push(bytes).unwrap();
        assert_eq!(msgs.len(), 1, "{name}");
        for peer in [None, Some("0.2.0"), Some("0.2.9")] {
            assert!(
                allowed_for_peer(&msgs[0], peer),
                "{name} bloqueada para {peer:?}"
            );
        }
        assert_eq!(&encode(&msgs[0]), bytes, "{name}: reencode idêntico");
    }
}

#[test]
fn variantes_novas_ficam_fora_do_que_um_peer_02_decodifica_e_o_gate_as_barra() {
    let new = [
        Message::Caps(Caps::baseline(Os::Windows)),
        Message::LayoutBookSync(book()),
        Message::LayoutBookEdit(book()),
    ];
    for m in &new {
        assert!(
            !v02_knows(&encode(m)),
            "{m:?} não pode ser decodificável pelo 0.2"
        );
        for peer in [None, Some("0.2.0"), Some("0.2.9")] {
            assert!(!allowed_for_peer(m, peer), "{m:?} vazou para {peer:?}");
        }
        for peer in [Some("0.3.0"), Some("0.3.1"), Some("0.10.0")] {
            assert!(allowed_for_peer(m, peer), "{m:?} barrada para {peer:?}");
        }
    }
}

#[test]
fn caps_message_so_existe_para_peer_simetrico() {
    let caps = Caps::baseline(Os::Mac);
    assert_eq!(caps_message(caps, None), None);
    assert_eq!(caps_message(caps, Some("0.2.9")), None);
    assert_eq!(caps_message(caps, Some("lixo")), None);
    assert_eq!(caps_message(caps, Some("0.3.0")), Some(Message::Caps(caps)));
}

fn cfg(host: Machine) -> LayoutConfig {
    LayoutConfig {
        profiles: vec![LayoutProfile {
            key: ProfileKey(vec!["k".into()]),
            name: "casa".into(),
            displays: vec![],
            windows_detached: false,
        }],
        active: Some("casa".into()),
        host,
        revision: 2,
    }
}

fn disp(id: &str, machine: Machine) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: id.into(),
        machine,
        origin_px: (0, 0),
        size_px: (1920, 1080),
        scale: 1.0,
        size_mm: Some((527.0, 296.0)),
        is_primary: true,
        is_builtin: false,
    }
}

/// Nenhuma operação do host de layouts, com peer 0.2 conectado, produz Send que o 0.2 não entenda.
#[test]
fn nenhum_efeito_do_host_de_layouts_escapa_para_um_peer_02() {
    let all = vec![disp("m1", Machine::Mac), disp("w1", Machine::Windows)];
    let other_book = LayoutBook {
        revision: 9,
        host: Machine::Windows,
        profiles: vec![],
        active: None,
    };
    for local in [Machine::Mac, Machine::Windows] {
        for host in [Machine::Mac, Machine::Windows] {
            let mut st = LayoutHostState::new(local, cfg(host));
            let mut fx: Vec<LayoutEffect> = vec![];
            fx.extend(st.peer_connected(false));
            fx.extend(st.on_book_sync(other_book.clone()));
            fx.extend(st.on_book_edit(other_book.clone()));
            fx.extend(st.on_layout_sync(cfg(host).profiles[0].clone()));
            fx.extend(st.on_displays(&all));
            fx.extend(st.on_new_pairing(Host::Local));
            fx.extend(st.on_new_pairing(Host::Peer));
            fx.extend(st.edit(cfg(host).profiles, None).unwrap_or_default());
            fx.extend(st.set_host(Host::Local).unwrap_or_default());
            fx.extend(st.set_host(Host::Peer).unwrap_or_default());
            let _ = profile_key(&all);
            for e in fx {
                if let LayoutEffect::Send(m) = e {
                    assert!(
                        allowed_for_peer(&m, Some("0.2.9")),
                        "{local:?}/host {host:?}: {m:?} iria para um peer 0.2.9"
                    );
                    assert!(v02_knows(&encode(&m)), "{m:?}");
                }
            }
        }
    }
}

// ---- Descoberta (I3) + discagem (I2) ----

/// O que o parser monta a partir do TXT: 0.3 anuncia `os` e `v` (0.3.0); 0.2 só id/pv/udp.
fn announced(id: &str, v03: bool) -> DiscoveredPeer {
    DiscoveredPeer {
        name: id.into(),
        addrs: vec!["192.168.0.2".into()],
        port: 47800,
        device_id: id.into(),
        protocol_version: 1,
        os: v03.then(Os::current),
        app_version: v03.then(|| "0.3.0".to_string()),
    }
}

#[test]
fn os_dois_lados_tomam_decisoes_complementares_sobre_o_mesmo_anuncio() {
    let (a, b) = ("0aaa", "0bbb");
    let (pa, pb) = (announced(a, true), announced(b, true));
    // Cada lado avalia o anúncio do outro.
    assert_eq!(dial_decision(a, &pb), DialDecision::DialNow);
    assert_eq!(dial_decision(b, &pa), DialDecision::DialAfterFallback);
    // O próprio anúncio é ignorado nos dois lados.
    assert_eq!(dial_decision(a, &pa), DialDecision::Ignore);
    assert_eq!(dial_decision(b, &pb), DialDecision::Ignore);
}

#[test]
fn peer_que_anuncia_sem_versao_e_discado_na_hora_de_qualquer_lado() {
    for local in ["0aaa", "0zzz"] {
        assert_eq!(
            dial_decision(local, &announced("0mmm", false)),
            DialDecision::DialNow,
            "{local}"
        );
    }
}

#[test]
fn sessao_duplicada_os_dois_lados_ficam_com_a_mesma() {
    // A sessão do dialer "0aaa" e a do dialer "0bbb" existem nos dois lados; a ordem de chegada
    // difere em cada um.
    let (a, b) = ("0aaa", "0bbb");
    // Lado A: existente = a (A discou), chega a sessão de b.
    let a_keeps_new = keep_new_session(a, b);
    // Lado B: existente = b (B discou), chega a sessão de a.
    let b_keeps_new = keep_new_session(b, a);
    assert!(!a_keeps_new, "A fica com a sua, discada por a");
    assert!(b_keeps_new, "B troca para a de a");
    assert!(!keep_new_session(a, a), "ids iguais: mantém a existente");
}
