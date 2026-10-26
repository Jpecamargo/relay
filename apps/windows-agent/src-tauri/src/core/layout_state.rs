//! dono: WI1
//! Lógica pura dos layouts no agente (livro, perfil ativo, validação das edições da UI), sem
//! estado nem I/O: roda nos testes do Mac. O estado vivo é o `LayoutHostState` do core.

use relay_core::layout::{DEFAULT_PROFILE_NAME, profile_key, select_profile, validate};
use relay_core::node::layout_host::LayoutHostState;
use relay_core::types::{DisplaysPayload, Host, LayoutBook, LayoutConfig, LayoutProfile, Machine};

pub(super) const LOCAL: Machine = Machine::Windows;

pub(super) fn book_of(l: &LayoutConfig) -> LayoutBook {
    LayoutBook {
        revision: l.revision,
        host: l.host,
        profiles: l.profiles.clone(),
        active: l.active.clone(),
    }
}

/// Perfil em uso: o do nome `active`, senão o primeiro; `None` sem perfis.
pub(super) fn active_profile(book: &LayoutBook) -> Option<LayoutProfile> {
    book.active
        .as_deref()
        .and_then(|n| book.profiles.iter().find(|p| p.name == n))
        .or(book.profiles.first())
        .cloned()
}

/// Perfil que o Receiver e o Controller reverso usam: o mesmo cálculo do Mac (o ativo adaptado às
/// telas conectadas, com as telas do Mac seguindo o arranjo atual do macOS). Sem as telas do Mac
/// ainda não há conjunto para casar: vale o ativo como gravado (retorno na primeira conexão).
pub(super) fn effective_profile(
    book: &LayoutBook,
    displays: &DisplaysPayload,
) -> Option<LayoutProfile> {
    if displays.peer.is_empty() {
        return active_profile(book);
    }
    let all: Vec<_> = displays
        .local
        .iter()
        .chain(&displays.peer)
        .cloned()
        .collect();
    select_profile(&book.profiles, book.active.as_deref(), &all).or_else(|| active_profile(book))
}

pub(super) fn empty_profile() -> LayoutProfile {
    LayoutProfile {
        key: profile_key(&[]),
        name: String::new(),
        displays: Vec::new(),
        windows_detached: false,
    }
}

/// `save_layout`: valida, normaliza o nome e faz o upsert (um por nome). Devolve a lista nova e
/// o nome do perfil, que passa a ser o ativo.
pub(super) fn upsert(
    current: &[LayoutProfile],
    mut profile: LayoutProfile,
) -> Result<(Vec<LayoutProfile>, String), String> {
    validate(&profile).map_err(|e| e.to_string())?;
    profile.name = profile.name.trim().to_owned();
    if profile.name.is_empty() {
        profile.name = DEFAULT_PROFILE_NAME.to_owned();
    }
    let name = profile.name.clone();
    let mut profiles = current.to_vec();
    match profiles.iter_mut().find(|p| p.name == name) {
        Some(slot) => *slot = profile,
        None => profiles.push(profile),
    }
    Ok((profiles, name))
}

/// `set_layout_profiles`: nomes não vazios e únicos, e o ativo precisa existir.
pub(super) fn check_profiles(
    profiles: Vec<LayoutProfile>,
    active: &str,
) -> Result<(Vec<LayoutProfile>, String), String> {
    let mut names = std::collections::HashSet::new();
    for p in &profiles {
        let name = p.name.trim();
        if name.is_empty() {
            return Err("perfil sem nome".into());
        }
        if !names.insert(name) {
            return Err(format!("já existe um perfil chamado {name}"));
        }
    }
    let active = active.trim().to_owned();
    if !names.contains(active.as_str()) {
        return Err(format!("perfil {active} não existe"));
    }
    let profiles = profiles
        .into_iter()
        .map(|p| LayoutProfile {
            name: p.name.trim().to_owned(),
            ..p
        })
        .collect();
    Ok((profiles, active))
}

/// `(layout_host, layout_editable)` do status: com peer 0.2 conectado o host efetivo é o Mac.
pub(super) fn status_fields(
    state: &LayoutHostState,
    connected: bool,
    peer_symmetric: bool,
) -> (Host, bool) {
    let host = if connected && !peer_symmetric {
        Machine::Mac
    } else {
        state.book().host
    };
    let side = if host == LOCAL {
        Host::Local
    } else {
        Host::Peer
    };
    (side, state.editable(connected, peer_symmetric))
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_core::types::{LayoutConfig, ProfileKey};

    fn prof(name: &str) -> LayoutProfile {
        LayoutProfile {
            key: ProfileKey(vec!["a".into()]),
            name: name.into(),
            displays: Vec::new(),
            windows_detached: false,
        }
    }

    #[test]
    fn active_profile_falls_back_to_first() {
        let mut book = book_of(&LayoutConfig::default());
        assert!(active_profile(&book).is_none());
        book.profiles = vec![prof("a"), prof("b")];
        assert_eq!(active_profile(&book).unwrap().name, "a");
        book.active = Some("b".into());
        assert_eq!(active_profile(&book).unwrap().name, "b");
        book.active = Some("gone".into());
        assert_eq!(active_profile(&book).unwrap().name, "a");
    }

    #[test]
    fn effective_profile_follows_the_macos_arrangement() {
        use relay_core::engine::testutil::{dell_display, display};
        use relay_core::types::{PlacedDisplay, RectMm};
        let mac = |id: &str, x: i32, primary: bool| {
            let mut d = display(id, Machine::Mac, (1440, 900), (300.0, 190.0), primary);
            d.origin_px = (x, 0);
            d
        };
        let placed = |id: &str, machine, x: f32| PlacedDisplay {
            display_id: id.into(),
            machine,
            rect_mm: RectMm {
                x,
                y: 0.0,
                w: 300.0,
                h: 190.0,
            },
            sensitivity: 1.0,
            rotation: None,
        };
        // Gravado: D2 à direita de D1. O macOS agora a põe à esquerda.
        let saved = LayoutProfile {
            key: ProfileKey(vec!["d1".into(), "d2".into(), "dell".into()]),
            name: "p".into(),
            displays: vec![
                placed("d1", Machine::Mac, 0.0),
                placed("d2", Machine::Mac, 300.0),
                placed("dell", Machine::Windows, 600.0),
            ],
            windows_detached: false,
        };
        let book = LayoutBook {
            revision: 1,
            host: Machine::Mac,
            profiles: vec![saved],
            active: Some("p".into()),
        };
        let displays = DisplaysPayload {
            local: vec![dell_display()],
            peer: vec![mac("d1", 0, true), mac("d2", -1440, false)],
        };
        let x = |p: &LayoutProfile, id: &str| {
            p.displays
                .iter()
                .find(|d| d.display_id == id)
                .unwrap()
                .rect_mm
                .x
        };
        let eff = effective_profile(&book, &displays).unwrap();
        assert!(
            x(&eff, "d2") < x(&eff, "d1"),
            "D2 deve ficar à esquerda de D1"
        );
        // Sem as telas do Mac, vale o gravado.
        let none = DisplaysPayload {
            peer: vec![],
            ..displays
        };
        assert_eq!(x(&effective_profile(&book, &none).unwrap(), "d2"), 300.0);
    }

    #[test]
    fn upsert_replaces_by_name_and_names_the_blank() {
        let (list, name) = upsert(&[prof("a")], prof(" a ")).unwrap();
        assert_eq!((list.len(), name.as_str()), (1, "a"));
        let (list, name) = upsert(&list, prof("  ")).unwrap();
        assert_eq!((list.len(), name.as_str()), (2, DEFAULT_PROFILE_NAME));
    }

    #[test]
    fn check_profiles_rejects_blank_duplicate_and_missing_active() {
        assert!(check_profiles(vec![prof(" ")], "a").is_err());
        assert!(check_profiles(vec![prof("a"), prof("a ")], "a").is_err());
        assert!(check_profiles(vec![prof("a")], "b").is_err());
        let (l, a) = check_profiles(vec![prof(" a")], " a ").unwrap();
        assert_eq!((l[0].name.as_str(), a.as_str()), ("a", "a"));
    }

    #[test]
    fn status_fields_follow_the_effective_host() {
        let cfg = LayoutConfig {
            host: Machine::Windows,
            ..LayoutConfig::default()
        };
        let st = LayoutHostState::new(LOCAL, cfg);
        assert_eq!(status_fields(&st, false, false), (Host::Local, true));
        // peer 0.2 conectado: o host efetivo é o Mac e o PC não edita
        assert_eq!(status_fields(&st, true, false), (Host::Peer, false));
        assert_eq!(status_fields(&st, true, true), (Host::Local, true));
        let st = LayoutHostState::new(LOCAL, LayoutConfig::default());
        assert_eq!(status_fields(&st, false, true), (Host::Peer, false));
        assert_eq!(status_fields(&st, true, true), (Host::Peer, true));
    }
}
