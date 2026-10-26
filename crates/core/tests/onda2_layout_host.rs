//! Host de layouts (I4) em conversa real: dois `LayoutHostState` (Mac e Windows) trocam as
//! mensagens que um produz e o outro consome, passando pelo codec do protocolo.

use std::collections::VecDeque;

use relay_core::node::layout_host::{LayoutEffect, LayoutError, LayoutHostState};
use relay_core::protocol::{FrameReader, FrameWriter};
use relay_core::types::{
    DisplayInfo, Host, LayoutBook, LayoutConfig, LayoutProfile, Machine, Message, ProfileKey,
};

fn prof(name: &str) -> LayoutProfile {
    LayoutProfile {
        key: ProfileKey(vec![name.into()]),
        name: name.into(),
        displays: vec![],
        windows_detached: false,
    }
}

fn config(host: Machine, revision: u64, names: &[&str]) -> LayoutConfig {
    LayoutConfig {
        profiles: names.iter().map(|n| prof(n)).collect(),
        active: names.first().map(|n| n.to_string()),
        host,
        revision,
    }
}

fn other(m: Machine) -> Machine {
    match m {
        Machine::Mac => Machine::Windows,
        Machine::Windows => Machine::Mac,
    }
}

struct Side {
    st: LayoutHostState,
    disk: Vec<LayoutBook>,
    ui: Vec<LayoutBook>,
}

struct Link {
    mac: Side,
    win: Side,
    wire: VecDeque<(Machine, Vec<u8>)>,
    up: bool,
}

impl Link {
    fn new(mac: LayoutConfig, win: LayoutConfig) -> Self {
        let side = |m, c| Side {
            st: LayoutHostState::new(m, c),
            disk: vec![],
            ui: vec![],
        };
        Self {
            mac: side(Machine::Mac, mac),
            win: side(Machine::Windows, win),
            wire: VecDeque::new(),
            up: false,
        }
    }

    fn side(&mut self, m: Machine) -> &mut Side {
        match m {
            Machine::Mac => &mut self.mac,
            Machine::Windows => &mut self.win,
        }
    }

    fn apply(&mut self, from: Machine, effects: Vec<LayoutEffect>) {
        for e in effects {
            match e {
                LayoutEffect::Send(msg) => {
                    if self.up {
                        let mut bytes = Vec::new();
                        FrameWriter.encode(&msg, &mut bytes).unwrap();
                        self.wire.push_back((other(from), bytes));
                    }
                }
                LayoutEffect::Persist(b) => self.side(from).disk.push(b),
                LayoutEffect::Emit(b) => self.side(from).ui.push(b),
            }
        }
    }

    fn pump(&mut self) {
        for _ in 0..60 {
            let Some((to, bytes)) = self.wire.pop_front() else {
                return;
            };
            for msg in FrameReader::default().push(&bytes).unwrap() {
                let st = &mut self.side(to).st;
                let fx = match msg {
                    Message::LayoutBookSync(b) => st.on_book_sync(b),
                    Message::LayoutBookEdit(b) => st.on_book_edit(b),
                    Message::LayoutSync(p) => st.on_layout_sync(p),
                    other => panic!("mensagem inesperada na conversa: {other:?}"),
                };
                self.apply(to, fx);
            }
        }
        panic!("a conversa não termina (ping-pong de livros)");
    }

    fn connect(&mut self) {
        self.up = true;
        let a = self.mac.st.peer_connected(true);
        self.apply(Machine::Mac, a);
        let b = self.win.st.peer_connected(true);
        self.apply(Machine::Windows, b);
        self.pump();
    }

    fn disconnect(&mut self) {
        self.up = false;
        self.wire.clear();
        self.mac.st.peer_disconnected();
        self.win.st.peer_disconnected();
    }

    fn assert_converged(&self) -> LayoutBook {
        let (m, w) = (self.mac.st.book(), self.win.st.book());
        assert_eq!(m, w, "os dois lados têm de ter o mesmo livro");
        for s in [&self.mac, &self.win] {
            if let Some(last) = s.disk.last() {
                assert_eq!(last, &m, "o último Persist é o livro final");
            }
        }
        m
    }
}

#[test]
fn novo_pareamento_aplica_a_escolha_de_host_dos_dois_lados() {
    // Mac com layouts da 0.2 (revision 2, host Mac); PC recém-instalado.
    for (typer, choice) in [
        (Machine::Mac, Host::Local),
        (Machine::Mac, Host::Peer),
        (Machine::Windows, Host::Local),
        (Machine::Windows, Host::Peer),
    ] {
        {
            let mut l = Link::new(
                config(Machine::Mac, 2, &["casa"]),
                config(Machine::Mac, 0, &[]),
            );
            l.connect();
            let before = l.assert_converged();
            assert_eq!(before.revision, 2);
            assert_eq!(before.profiles.len(), 1, "o PC novo adota o livro do Mac");

            let fx = l.side(typer).st.on_new_pairing(choice);
            let sent = format!("{fx:?}");
            l.apply(typer, fx);
            l.pump();
            let tag = format!("typer={typer:?} choice={choice:?}");
            assert_eq!(l.mac.st.book(), l.win.st.book(), "{tag} fx={sent}");
            let book = l.assert_converged();

            let want_host = if choice == Host::Local {
                typer
            } else {
                other(typer)
            };
            let tag = format!("typer={typer:?} choice={choice:?}");
            assert_eq!(book.host, want_host, "{tag}");
            // Host que muda ou o próprio digitador sendo host publicam; "o outro já é host" é no-op.
            let published = want_host != Machine::Mac || typer == Machine::Mac;
            assert!(
                book.revision > before.revision || !published,
                "{tag}: revision {}",
                book.revision
            );
            assert_eq!(book.profiles, before.profiles, "{tag}: perfis preservados");
            assert_eq!(book.active.as_deref(), Some("casa"), "{tag}");
            assert_eq!(l.mac.st.to_config().host, want_host, "{tag}");
            assert_eq!(l.win.st.to_config().host, want_host, "{tag}");
            assert_eq!(l.win.st.to_config().revision, book.revision, "{tag}");
        }
    }
}

#[test]
fn edicao_pelo_nao_host_vira_proposta_e_so_grava_depois_do_sync() {
    let mut l = Link::new(
        config(Machine::Mac, 4, &["casa"]),
        config(Machine::Mac, 4, &["casa"]),
    );
    l.connect();
    l.assert_converged();
    let (disk_win, disk_mac) = (l.win.disk.len(), l.mac.disk.len());

    let fx = l
        .win
        .st
        .edit(
            vec![prof("casa"), prof("trabalho")],
            Some("trabalho".into()),
        )
        .expect("host conectado e simétrico: editável");
    assert!(
        fx.iter()
            .all(|e| matches!(e, LayoutEffect::Send(Message::LayoutBookEdit(_))))
            && !fx.is_empty(),
        "não-host só propõe: {fx:?}"
    );
    assert_eq!(l.win.disk.len(), disk_win, "nada gravado antes do Sync");
    l.apply(Machine::Windows, fx);
    l.pump();

    let book = l.assert_converged();
    assert_eq!(book.host, Machine::Mac);
    assert_eq!(book.revision, 5);
    assert_eq!(book.active.as_deref(), Some("trabalho"));
    let names: Vec<_> = book.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["casa", "trabalho"]);
    assert!(l.mac.disk.len() > disk_mac, "o host gravou");
    assert!(l.win.disk.len() > disk_win, "o espelho gravou o Sync");
    assert_eq!(l.win.ui.last(), Some(&book));
}

#[test]
fn ultima_escrita_vale_quando_os_dois_lados_editam() {
    let mut l = Link::new(
        config(Machine::Mac, 1, &["a"]),
        config(Machine::Mac, 1, &["a"]),
    );
    l.connect();
    let fx = l.win.st.edit(vec![prof("a"), prof("do-pc")], None).unwrap();
    l.apply(Machine::Windows, fx);
    let fx = l
        .mac
        .st
        .edit(vec![prof("a"), prof("do-mac")], None)
        .unwrap();
    l.apply(Machine::Mac, fx);
    l.pump();
    let book = l.assert_converged();
    // Quem chegou ao host por último vale; o resultado é o mesmo nos dois lados e só um dos dois.
    assert!(book.revision >= 3);
    let names: Vec<_> = book.profiles.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names == ["a", "do-pc"] || names == ["a", "do-mac"],
        "{names:?}"
    );
}

#[test]
fn desconectado_o_nao_host_fica_so_leitura_e_o_livro_nao_muda() {
    let mut l = Link::new(
        config(Machine::Mac, 4, &["casa"]),
        config(Machine::Mac, 4, &["casa"]),
    );
    l.connect();
    l.disconnect();
    let before = l.win.st.book();
    let disk = l.win.disk.len();

    assert!(!l.win.st.editable(false, true));
    assert_eq!(
        l.win.st.edit(vec![prof("x")], None).unwrap_err(),
        LayoutError::ReadOnly
    );
    assert_eq!(
        l.win.st.set_host(Host::Local).unwrap_err(),
        LayoutError::ReadOnly
    );
    assert_eq!(l.win.st.book(), before);
    assert_eq!(l.win.disk.len(), disk);
    // O host continua editando sozinho.
    assert!(l.mac.st.editable(false, true));
    assert!(l.mac.st.edit(vec![prof("x")], None).is_ok());
}

#[test]
fn na_reconexao_o_espelho_recebe_o_que_o_host_mudou_offline() {
    let mut l = Link::new(
        config(Machine::Mac, 4, &["casa"]),
        config(Machine::Mac, 4, &["casa"]),
    );
    l.connect();
    l.disconnect();
    let fx = l
        .mac
        .st
        .edit(vec![prof("casa"), prof("offline")], None)
        .unwrap();
    l.apply(Machine::Mac, fx); // sem enlace: Send descartado
    let host_book = l.mac.st.book();
    assert_eq!(host_book.revision, 5);

    l.connect();
    let book = l.assert_converged();
    assert_eq!(book, host_book);
    assert!(l.win.st.editable(true, true));
    // Reconectado, o não-host volta a poder propor.
    assert!(l.win.st.edit(book.profiles.clone(), None).is_ok());
}

#[test]
fn reconexao_com_edicao_do_espelho_pendente_nao_sobrescreve_o_host() {
    // Revisão maior vence: o PC (não-host) não tem como ter revisão maior sem passar pelo host,
    // mas se tiver (config manipulado), a conversa converge nos dois lados sem laço.
    let mut l = Link::new(
        config(Machine::Mac, 3, &["m"]),
        config(Machine::Mac, 9, &["w"]),
    );
    l.connect();
    let book = l.assert_converged();
    assert_eq!(book.revision, 9);
}

#[test]
fn empate_com_hosts_divergentes_vira_mac_com_o_livro_do_mac() {
    let mut l = Link::new(
        config(Machine::Mac, 5, &["do-mac"]),
        config(Machine::Windows, 5, &["do-pc"]),
    );
    l.connect();
    let book = l.assert_converged();
    assert_eq!(book.host, Machine::Mac);
    assert_eq!(book.profiles[0].name, "do-mac");
}

#[test]
fn empate_com_o_mesmo_host_fica_com_o_livro_do_host() {
    let mut l = Link::new(
        config(Machine::Windows, 5, &["do-mac"]),
        config(Machine::Windows, 5, &["do-pc"]),
    );
    l.connect();
    let book = l.assert_converged();
    assert_eq!(book.host, Machine::Windows);
    assert_eq!(book.profiles[0].name, "do-pc");
}

#[test]
fn troca_de_host_pelo_nao_host_chega_ao_host_e_volta() {
    let mut l = Link::new(
        config(Machine::Mac, 1, &["casa"]),
        config(Machine::Mac, 1, &["casa"]),
    );
    l.connect();
    let fx = l.win.st.set_host(Host::Local).unwrap();
    l.apply(Machine::Windows, fx);
    l.pump();
    let book = l.assert_converged();
    assert_eq!(book.host, Machine::Windows);
    assert!(book.revision >= 2);
    // Agora o Mac é o não-host e pode devolver.
    let fx = l.mac.st.set_host(Host::Local).unwrap();
    l.apply(Machine::Mac, fx);
    l.pump();
    assert_eq!(l.assert_converged().host, Machine::Mac);
}

#[test]
fn peer_02_o_host_efetivo_e_sempre_o_mac_e_o_pc_fica_so_leitura() {
    let mut win = LayoutHostState::new(Machine::Windows, config(Machine::Windows, 3, &["x"]));
    let _ = win.peer_connected(false);
    assert!(!win.editable(true, false));
    assert_eq!(
        win.edit(vec![prof("y")], None).unwrap_err(),
        LayoutError::ReadOnly
    );

    // LayoutSync do Mac 0.2: upsert pelo nome e marca ativo no espelho.
    win.on_layout_sync(prof("novo"));
    win.on_layout_sync(prof("novo"));
    let b = win.book();
    assert_eq!(b.profiles.iter().filter(|p| p.name == "novo").count(), 1);
    assert_eq!(b.active.as_deref(), Some("novo"));

    // Mac 0.3 com PC 0.2: edita e manda LayoutSync do ativo, nunca livro.
    let mut mac = LayoutHostState::new(Machine::Mac, config(Machine::Mac, 1, &["casa"]));
    let _ = mac.peer_connected(false);
    let fx = mac
        .edit(vec![prof("casa"), prof("sala")], Some("sala".into()))
        .unwrap();
    let sends: Vec<_> = fx
        .iter()
        .filter_map(|e| {
            if let LayoutEffect::Send(m) = e {
                Some(m)
            } else {
                None
            }
        })
        .collect();
    assert!(
        matches!(sends.as_slice(), [Message::LayoutSync(p)] if p.name == "sala"),
        "{sends:?}"
    );
}

fn disp(id: &str, machine: Machine, x: i32) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: id.into(),
        machine,
        origin_px: (x, 0),
        size_px: (1920, 1080),
        scale: 1.0,
        size_mm: Some((527.0, 296.0)),
        is_primary: x == 0,
        is_builtin: false,
    }
}

#[test]
fn host_cria_perfil_para_conjunto_novo_de_telas_e_sincroniza_uma_vez() {
    let all = vec![disp("m1", Machine::Mac, 0), disp("w1", Machine::Windows, 0)];
    let mut l = Link::new(config(Machine::Mac, 1, &[]), config(Machine::Mac, 1, &[]));
    l.connect();
    let key = relay_core::layout::profile_key(&all);

    let fx = l.mac.st.on_displays(&all);
    l.apply(Machine::Mac, fx);
    l.pump();
    let book = l.assert_converged();
    assert!(
        book.profiles.iter().any(|p| p.key == key),
        "perfil criado nos dois lados"
    );
    assert!(book.revision > 1);

    // Mesmo conjunto de novo: nada a criar.
    let again = l.mac.st.on_displays(&all);
    assert!(again.is_empty(), "{again:?}");

    // Não-host não cria perfil por conta própria.
    let mut l = Link::new(config(Machine::Mac, 1, &[]), config(Machine::Mac, 1, &[]));
    l.connect();
    let fx = l.win.st.on_displays(&all);
    assert!(
        !fx.iter().any(|e| matches!(e, LayoutEffect::Persist(_))),
        "{fx:?}"
    );
}
