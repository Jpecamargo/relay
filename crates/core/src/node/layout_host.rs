//! Máquina de estados pura do host de layouts (D42–D44): quem guarda o livro oficial, quem pode
//! editar e o que enviar/gravar/emitir a cada evento. Sem I/O: cada operação devolve efeitos.

use crate::layout::{profile_key, select_profile};
use crate::types::{
    DisplayInfo, Host, LayoutBook, LayoutConfig, LayoutProfile, Machine, Message, ProfileKey,
};

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutEffect {
    /// Mensagem para o peer.
    Send(Message),
    /// Gravar o livro no config.
    Persist(LayoutBook),
    /// Avisar a UI/engine do livro novo.
    Emit(LayoutBook),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Sem o host conectado (ou com peer 0.2 e host no outro lado), os layouts são só leitura.
    #[error("layouts em só-leitura: o host não está conectado")]
    ReadOnly,
}

#[derive(Debug, Clone)]
pub struct LayoutHostState {
    local: Machine,
    revision: u64,
    host: Machine,
    profiles: Vec<LayoutProfile>,
    active: Option<String>,
    /// `Some(symmetric)` com peer conectado; `None` desconectado.
    peer: Option<bool>,
    /// Peer simétrico conectado cujo `LayoutBookSync` inicial ainda não chegou.
    awaiting_exchange: bool,
}

fn other(m: Machine) -> Machine {
    match m {
        Machine::Mac => Machine::Windows,
        Machine::Windows => Machine::Mac,
    }
}

impl LayoutHostState {
    pub fn new(local: Machine, config: LayoutConfig) -> Self {
        Self {
            local,
            revision: config.revision,
            host: config.host,
            profiles: config.profiles,
            active: config.active,
            peer: None,
            awaiting_exchange: false,
        }
    }

    pub fn to_config(&self) -> LayoutConfig {
        LayoutConfig {
            profiles: self.profiles.clone(),
            active: self.active.clone(),
            host: self.host,
            revision: self.revision,
        }
    }

    pub fn book(&self) -> LayoutBook {
        LayoutBook {
            revision: self.revision,
            host: self.host,
            profiles: self.profiles.clone(),
            active: self.active.clone(),
        }
    }

    /// Host efetivo: com peer 0.2 conectado é sempre o Mac.
    fn effective_host(&self) -> Machine {
        match self.peer {
            Some(false) => Machine::Mac,
            _ => self.host,
        }
    }

    fn is_host(&self) -> bool {
        self.effective_host() == self.local
    }

    /// Pode editar: o host é local, ou o host (o outro lado) está conectado e é simétrico.
    pub fn editable(&self, connected: bool, peer_symmetric: bool) -> bool {
        let host = if connected && !peer_symmetric {
            Machine::Mac
        } else {
            self.host
        };
        host == self.local || (connected && peer_symmetric)
    }

    fn adopt(&mut self, book: LayoutBook) {
        self.revision = book.revision;
        self.host = book.host;
        self.profiles = book.profiles;
        self.active = book.active;
    }

    fn active_profile(&self) -> Option<&LayoutProfile> {
        self.active
            .as_deref()
            .and_then(|n| self.profiles.iter().find(|p| p.name == n))
            .or(self.profiles.first())
    }

    /// Efeitos de uma mudança já aplicada pelo host: grava, emite e avisa o peer.
    fn published(&self) -> Vec<LayoutEffect> {
        let book = self.book();
        let mut out = vec![
            LayoutEffect::Persist(book.clone()),
            LayoutEffect::Emit(book.clone()),
        ];
        match self.peer {
            Some(true) => out.push(LayoutEffect::Send(Message::LayoutBookSync(book))),
            Some(false) => {
                if let Some(p) = self.active_profile() {
                    out.push(LayoutEffect::Send(Message::LayoutSync(p.clone())));
                }
            }
            None => {}
        }
        out
    }

    /// Peer conectou. Simétrico: envia o livro e espera o dele para resolver.
    pub fn peer_connected(&mut self, peer_symmetric: bool) -> Vec<LayoutEffect> {
        self.peer = Some(peer_symmetric);
        self.awaiting_exchange = peer_symmetric;
        if peer_symmetric {
            vec![LayoutEffect::Send(Message::LayoutBookSync(self.book()))]
        } else {
            Vec::new()
        }
    }

    pub fn peer_disconnected(&mut self) {
        self.peer = None;
        self.awaiting_exchange = false;
    }

    /// `LayoutBookSync` recebido. Na primeira troca da conexão resolve o conflito (mesmo
    /// resultado dos dois lados); depois disso, só o não-host aceita, e só revision >= local.
    pub fn on_book_sync(&mut self, peer_book: LayoutBook) -> Vec<LayoutEffect> {
        if self.awaiting_exchange {
            self.awaiting_exchange = false;
            return self.resolve(peer_book);
        }
        // O host só aceita Sync de quem assumiu o papel de host (book.host é o outro lado).
        let handover =
            self.is_host() && peer_book.revision > self.revision && peer_book.host != self.local;
        if (self.is_host() && !handover) || peer_book.revision < self.revision {
            return Vec::new();
        }
        self.adopt(peer_book);
        vec![
            LayoutEffect::Persist(self.book()),
            LayoutEffect::Emit(self.book()),
        ]
    }

    fn resolve(&mut self, peer_book: LayoutBook) -> Vec<LayoutEffect> {
        let local_book = self.book();
        // Os dois lados chegam à mesma escolha: maior revision; empate com o mesmo host, o livro
        // do host; empate com hosts divergentes, o livro do Mac com host Mac.
        let local_wins = if local_book.revision != peer_book.revision {
            local_book.revision > peer_book.revision
        } else if local_book.host == peer_book.host {
            local_book.host == self.local
        } else {
            self.local == Machine::Mac
        };
        let mut winner = if local_wins {
            local_book.clone()
        } else {
            peer_book.clone()
        };
        if local_book.revision == peer_book.revision && local_book.host != peer_book.host {
            winner.host = Machine::Mac;
        }
        if winner == local_book {
            return Vec::new();
        }
        self.adopt(winner);
        vec![
            LayoutEffect::Persist(self.book()),
            LayoutEffect::Emit(self.book()),
        ]
    }

    /// Edição local (salvar perfis, trocar o ativo).
    pub fn edit(
        &mut self,
        profiles: Vec<LayoutProfile>,
        active: Option<String>,
    ) -> Result<Vec<LayoutEffect>, LayoutError> {
        if self.is_host() {
            self.profiles = profiles;
            self.active = active;
            self.revision += 1;
            return Ok(self.published());
        }
        self.send_edit(LayoutBook {
            revision: self.revision,
            host: self.host,
            profiles,
            active,
        })
    }

    /// Escolhe o host. No host: publica já; no não-host: pede ao host por `LayoutBookEdit`.
    pub fn set_host(&mut self, host: Host) -> Result<Vec<LayoutEffect>, LayoutError> {
        let machine = self.machine_of(host);
        if self.is_host() {
            if self.peer == Some(false) {
                return Err(LayoutError::ReadOnly);
            }
            self.host = machine;
            self.revision += 1;
            return Ok(self.published());
        }
        let mut book = self.book();
        book.host = machine;
        self.send_edit(book)
    }

    fn machine_of(&self, host: Host) -> Machine {
        match host {
            Host::Local => self.local,
            Host::Peer => other(self.local),
        }
    }

    fn send_edit(&self, book: LayoutBook) -> Result<Vec<LayoutEffect>, LayoutError> {
        if self.peer == Some(true) {
            Ok(vec![LayoutEffect::Send(Message::LayoutBookEdit(book))])
        } else {
            Err(LayoutError::ReadOnly)
        }
    }

    /// `LayoutBookEdit` recebido: o host aplica (vale a última escrita); o não-host só quando o
    /// Edit o nomeia host, e então assume o papel.
    pub fn on_book_edit(&mut self, edit: LayoutBook) -> Vec<LayoutEffect> {
        // Não-host só aceita o Edit que lhe passa o papel de host (edit.host é ele mesmo).
        if !self.is_host() && !(edit.host == self.local && edit.revision > self.revision) {
            return Vec::new();
        }
        self.revision = self.revision.max(edit.revision) + 1;
        self.host = edit.host;
        self.profiles = edit.profiles;
        self.active = edit.active;
        self.published()
    }

    /// `LayoutSync` de um Mac 0.2: upsert do perfil pelo nome, marcado ativo no espelho.
    pub fn on_layout_sync(&mut self, profile: LayoutProfile) -> Vec<LayoutEffect> {
        if self.is_host() {
            return Vec::new();
        }
        self.active = Some(profile.name.clone());
        match self.profiles.iter_mut().find(|p| p.name == profile.name) {
            Some(slot) => *slot = profile,
            None => self.profiles.push(profile),
        }
        vec![
            LayoutEffect::Persist(self.book()),
            LayoutEffect::Emit(self.book()),
        ]
    }

    /// Pareamento novo, depois da primeira troca de livros (revision local já é a maior das
    /// duas). Quem digitou o código aplica a escolha: vira host (Sync) ou pede ao outro (Edit).
    pub fn on_new_pairing(&mut self, choice: Host) -> Vec<LayoutEffect> {
        if self.peer != Some(true) {
            return Vec::new();
        }
        let machine = self.machine_of(choice);
        self.revision += 1;
        if machine == self.local {
            self.host = machine;
            return self.published();
        }
        let mut book = self.book();
        book.host = machine;
        vec![LayoutEffect::Send(Message::LayoutBookEdit(book))]
    }

    /// Conjunto de telas (local + do peer) mudou. O host cria o perfil da `ProfileKey` se faltar.
    pub fn on_displays(&mut self, displays: &[DisplayInfo]) -> Vec<LayoutEffect> {
        if !self.is_host() {
            return Vec::new();
        }
        let key = profile_key(displays);
        if self.has_profile_for(&key) {
            return Vec::new();
        }
        let Some(mut profile) = select_profile(&[], None, displays) else {
            return Vec::new();
        };
        let mut name = profile.name.clone();
        let mut n = 2;
        while self.profiles.iter().any(|p| p.name == name) {
            name = format!("{} {n}", profile.name);
            n += 1;
        }
        profile.name = name.clone();
        self.profiles.push(profile);
        self.active = Some(name);
        self.revision += 1;
        self.published()
    }

    fn has_profile_for(&self, key: &ProfileKey) -> bool {
        self.profiles.iter().any(|p| &p.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Machine::{Mac, Windows};

    fn prof(name: &str) -> LayoutProfile {
        LayoutProfile {
            key: ProfileKey(vec![name.into()]),
            name: name.into(),
            displays: Vec::new(),
            windows_detached: false,
        }
    }

    fn state(local: Machine, host: Machine, revision: u64, names: &[&str]) -> LayoutHostState {
        LayoutHostState::new(
            local,
            LayoutConfig {
                profiles: names.iter().map(|n| prof(n)).collect(),
                active: names.first().map(|n| (*n).to_string()),
                host,
                revision,
            },
        )
    }

    fn connected(mut s: LayoutHostState, symmetric: bool) -> LayoutHostState {
        s.peer_connected(symmetric);
        s.awaiting_exchange = false;
        s
    }

    /// Troca de livros dos dois lados; devolve os dois estados resolvidos.
    fn exchange(
        mut a: LayoutHostState,
        mut b: LayoutHostState,
    ) -> (LayoutHostState, LayoutHostState) {
        a.peer_connected(true);
        b.peer_connected(true);
        let (ba, bb) = (a.book(), b.book());
        a.on_book_sync(bb);
        b.on_book_sync(ba);
        (a, b)
    }

    fn display(id: &str, machine: Machine) -> DisplayInfo {
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

    #[test]
    fn config_roundtrip() {
        let s = state(Mac, Windows, 7, &["a", "b"]);
        let cfg = s.to_config();
        assert_eq!(
            (cfg.revision, cfg.host, cfg.active.as_deref()),
            (7, Windows, Some("a"))
        );
        assert_eq!(LayoutHostState::new(Mac, cfg.clone()).to_config(), cfg);
        assert_eq!(s.book().profiles.len(), 2);
    }

    #[test]
    fn resolve_highest_revision_wins_same_on_both_sides() {
        let (mac, win) = exchange(state(Mac, Mac, 3, &["m"]), state(Windows, Mac, 5, &["w"]));
        assert_eq!(mac.book(), win.book());
        assert_eq!(mac.book().revision, 5);
        assert_eq!(mac.book().profiles[0].name, "w");
    }

    #[test]
    fn resolve_tie_same_host_takes_host_book() {
        let (mac, win) = exchange(
            state(Mac, Windows, 4, &["m"]),
            state(Windows, Windows, 4, &["w"]),
        );
        assert_eq!(mac.book(), win.book());
        assert_eq!(mac.book().profiles[0].name, "w");
        assert_eq!(mac.book().host, Windows);
    }

    #[test]
    fn resolve_tie_divergent_hosts_becomes_mac() {
        let (mac, win) = exchange(
            state(Mac, Windows, 4, &["m"]),
            state(Windows, Mac, 4, &["w"]),
        );
        assert_eq!(mac.book(), win.book());
        assert_eq!(mac.book().host, Mac);
        assert_eq!(mac.book().profiles[0].name, "m");
        // o lado que perdeu persiste e emite; o que ganhou (mesmo livro) não faz nada
        let mut win = state(Windows, Mac, 4, &["w"]);
        win.peer_connected(true);
        let fx = win.on_book_sync(state(Mac, Windows, 4, &["m"]).book());
        assert!(matches!(
            fx.as_slice(),
            [LayoutEffect::Persist(_), LayoutEffect::Emit(_)]
        ));
    }

    #[test]
    fn host_edit_bumps_and_syncs_symmetric_or_legacy() {
        let mut s = connected(state(Mac, Mac, 2, &["a"]), true);
        let fx = s.edit(vec![prof("b")], Some("b".into())).unwrap();
        let book = s.book();
        assert_eq!(book.revision, 3);
        assert_eq!(
            fx,
            vec![
                LayoutEffect::Persist(book.clone()),
                LayoutEffect::Emit(book.clone()),
                LayoutEffect::Send(Message::LayoutBookSync(book)),
            ]
        );
        let mut s = connected(state(Mac, Mac, 2, &["a"]), false);
        let fx = s.edit(vec![prof("b")], Some("b".into())).unwrap();
        assert_eq!(
            fx.last(),
            Some(&LayoutEffect::Send(Message::LayoutSync(prof("b"))))
        );
    }

    #[test]
    fn non_host_edit_only_sends_edit() {
        let mut s = connected(state(Windows, Mac, 2, &["a"]), true);
        let fx = s.edit(vec![prof("b")], Some("b".into())).unwrap();
        assert!(
            matches!(fx.as_slice(), [LayoutEffect::Send(Message::LayoutBookEdit(b))] if b.profiles[0].name == "b")
        );
        assert_eq!(s.book().profiles[0].name, "a");
        assert_eq!(s.book().revision, 2);
    }

    #[test]
    fn non_host_edit_read_only_when_host_offline_or_legacy_peer() {
        for peer in [None, Some(false)] {
            let mut s = state(Windows, Mac, 2, &["a"]);
            if let Some(sym) = peer {
                s.peer_connected(sym);
            }
            assert_eq!(s.edit(vec![prof("b")], None), Err(LayoutError::ReadOnly));
            assert_eq!(s.book(), state(Windows, Mac, 2, &["a"]).book());
        }
    }

    #[test]
    fn host_applies_edit_with_max_revision_plus_one() {
        let mut s = connected(state(Mac, Mac, 5, &["a"]), true);
        let edit = LayoutBook {
            revision: 8,
            host: Mac,
            profiles: vec![prof("z")],
            active: Some("z".into()),
        };
        let fx = s.on_book_edit(edit);
        assert_eq!(s.book().revision, 9);
        assert_eq!(s.book().profiles[0].name, "z");
        assert!(
            matches!(fx.as_slice(), [LayoutEffect::Persist(_), LayoutEffect::Emit(_), LayoutEffect::Send(Message::LayoutBookSync(b))] if b.revision == 9)
        );
        let edit = LayoutBook {
            revision: 1,
            host: Mac,
            profiles: vec![],
            active: None,
        };
        s.on_book_edit(edit);
        assert_eq!(s.book().revision, 10);
        // não-host ignora
        let mut n = connected(state(Windows, Mac, 5, &["a"]), true);
        assert!(n.on_book_edit(s.book()).is_empty());
    }

    #[test]
    fn non_host_sync_replaces_when_revision_not_lower() {
        let mut s = connected(state(Windows, Mac, 5, &["a"]), true);
        let newer = LayoutBook {
            revision: 5,
            host: Mac,
            profiles: vec![prof("n")],
            active: None,
        };
        let fx = s.on_book_sync(newer);
        assert!(matches!(
            fx.as_slice(),
            [LayoutEffect::Persist(_), LayoutEffect::Emit(_)]
        ));
        assert_eq!(s.book().profiles[0].name, "n");
        let older = LayoutBook {
            revision: 4,
            host: Mac,
            profiles: vec![prof("o")],
            active: None,
        };
        assert!(s.on_book_sync(older).is_empty());
        assert_eq!(s.book().profiles[0].name, "n");
    }

    #[test]
    fn set_host_on_host_publishes() {
        let mut s = connected(state(Mac, Mac, 2, &["a"]), true);
        let fx = s.set_host(Host::Peer).unwrap();
        assert_eq!((s.book().host, s.book().revision), (Windows, 3));
        assert!(matches!(
            fx.as_slice(),
            [
                LayoutEffect::Persist(_),
                LayoutEffect::Emit(_),
                LayoutEffect::Send(Message::LayoutBookSync(_))
            ]
        ));
    }

    #[test]
    fn set_host_on_non_host_sends_edit_or_read_only() {
        let mut s = connected(state(Windows, Mac, 2, &["a"]), true);
        let fx = s.set_host(Host::Local).unwrap();
        assert!(
            matches!(fx.as_slice(), [LayoutEffect::Send(Message::LayoutBookEdit(b))] if b.host == Windows)
        );
        assert_eq!(s.book().host, Mac);
        let mut off = state(Windows, Mac, 2, &["a"]);
        assert_eq!(off.set_host(Host::Local), Err(LayoutError::ReadOnly));
    }

    #[test]
    fn legacy_peer_host_is_mac_and_layout_sync_upserts() {
        let mut win = connected(state(Windows, Windows, 2, &["a"]), false);
        assert!(!win.is_host());
        let mut updated = prof("a");
        updated.windows_detached = true;
        let fx = win.on_layout_sync(updated);
        assert!(matches!(
            fx.as_slice(),
            [LayoutEffect::Persist(_), LayoutEffect::Emit(_)]
        ));
        assert!(win.book().profiles[0].windows_detached);
        win.on_layout_sync(prof("novo"));
        assert_eq!(win.book().profiles.len(), 2);
        assert_eq!(win.book().active.as_deref(), Some("novo"));
        // Mac com peer 0.2 é host mesmo com livro apontando para Windows
        assert!(connected(state(Mac, Windows, 2, &[]), false).is_host());
    }

    #[test]
    fn editable_rule() {
        let s = state(Mac, Mac, 0, &[]);
        assert!(s.editable(false, false));
        let s = state(Windows, Mac, 0, &[]);
        assert!(!s.editable(false, true));
        assert!(!s.editable(true, false));
        assert!(s.editable(true, true));
    }

    #[test]
    fn new_pairing_applies_choice_with_max_revision_plus_one() {
        let (mut mac, _win) = exchange(state(Mac, Mac, 3, &["m"]), state(Windows, Mac, 5, &["w"]));
        let fx = mac.on_new_pairing(Host::Local);
        assert_eq!((mac.book().host, mac.book().revision), (Mac, 6));
        assert!(matches!(
            fx.as_slice(),
            [
                LayoutEffect::Persist(_),
                LayoutEffect::Emit(_),
                LayoutEffect::Send(Message::LayoutBookSync(_))
            ]
        ));

        let (mut mac, _win) = exchange(state(Mac, Mac, 3, &["m"]), state(Windows, Mac, 5, &["w"]));
        let fx = mac.on_new_pairing(Host::Peer);
        assert!(
            matches!(fx.as_slice(), [LayoutEffect::Send(Message::LayoutBookEdit(b))] if b.host == Windows && b.revision == 6)
        );
        assert_eq!(mac.book().host, Mac);
    }

    #[test]
    fn host_creates_profile_for_new_display_set_once() {
        let mut s = connected(state(Mac, Mac, 1, &[]), true);
        let ds = [display("m1", Mac), display("w1", Windows)];
        let fx = s.on_displays(&ds);
        assert_eq!(s.book().revision, 2);
        assert_eq!(s.book().profiles.len(), 1);
        assert_eq!(s.book().profiles[0].key, profile_key(&ds));
        assert!(matches!(
            fx.last(),
            Some(LayoutEffect::Send(Message::LayoutBookSync(_)))
        ));
        assert!(s.on_displays(&ds).is_empty());
        let mut n = connected(state(Windows, Mac, 1, &[]), true);
        assert!(n.on_displays(&ds).is_empty());
    }

    #[test]
    fn host_handover_by_sync_and_by_edit() {
        // o outro assumiu: Sync com revision maior e host diferente de mim
        let mut mac = connected(state(Mac, Mac, 2, &["a"]), true);
        let book = LayoutBook {
            revision: 3,
            host: Windows,
            profiles: vec![prof("a")],
            active: None,
        };
        let fx = mac.on_book_sync(book);
        assert!(matches!(
            fx.as_slice(),
            [LayoutEffect::Persist(_), LayoutEffect::Emit(_)]
        ));
        assert!(!mac.is_host());
        // Sync que não troca o host, ou com revision menor, o host ignora
        let mut mac = connected(state(Mac, Mac, 2, &["a"]), true);
        assert!(
            mac.on_book_sync(LayoutBook {
                revision: 9,
                host: Mac,
                profiles: vec![],
                active: None
            })
            .is_empty()
        );
        assert!(
            mac.on_book_sync(LayoutBook {
                revision: 1,
                host: Windows,
                profiles: vec![],
                active: None
            })
            .is_empty()
        );
        // Edit que me nomeia host: assumo com max+1 e publico
        let mut win = connected(state(Windows, Mac, 2, &["a"]), true);
        let edit = LayoutBook {
            revision: 3,
            host: Windows,
            profiles: vec![prof("a")],
            active: None,
        };
        let fx = win.on_book_edit(edit);
        assert_eq!((win.book().host, win.book().revision), (Windows, 4));
        assert!(matches!(
            fx.as_slice(),
            [
                LayoutEffect::Persist(_),
                LayoutEffect::Emit(_),
                LayoutEffect::Send(Message::LayoutBookSync(_))
            ]
        ));
    }
}
