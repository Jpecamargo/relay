//! dono: WI1
//! Layouts no agente (D42–D47): o PC como host publica `LayoutBookSync` com revision+1 para um Mac
//! 0.3; com um Mac 0.2 só-leitura; o Mac 0.3 como host manda o livro, que vira o perfil do
//! Receiver; telas novas criam perfil quando o PC é host.

use super::*;

const MAC_03: &str = "0.3.0";

fn book(revision: u64, host: Machine) -> LayoutBook {
    LayoutBook {
        revision,
        host,
        profiles: vec![profile()],
        active: Some("t".into()),
    }
}

fn pc_profile(name: &str) -> LayoutProfile {
    LayoutProfile {
        name: name.into(),
        ..profile()
    }
}

/// Agente com o PC como host dos layouts, ligado a um Mac 0.3 que já trocou o livro inicial.
async fn host_pc_with_mac03() -> (Agent, Mac) {
    let agent = start_agent_cfg(Arc::new(MemoryKeyStore::default()), None, |c| {
        c.layout.host = Machine::Windows;
        c.layout.revision = 4;
        c.layout.profiles = vec![profile()];
        c.layout.active = Some("t".into());
    });
    let id = identity();
    let peer = pair(&agent, &id).await;
    let mut mac = connect_as(&agent, &id, peer, MAC_03).await;
    mac.pump_until("livro inicial do agente", |m| {
        m.got(|x| matches!(x, Message::LayoutBookSync(_)))
    })
    .await;
    // O Mac devolve o livro dele (mesma revisão e host): a troca inicial termina.
    mac.dialer
        .send(Message::LayoutBookSync(book(4, Machine::Windows)))
        .await
        .unwrap();
    mac.dialer
        .send(Message::Displays(vec![mac_display()]))
        .await
        .unwrap();
    eventually("troca resolvida", || agent.core.status().layout_editable).await;
    (agent, mac)
}

#[tokio::test(flavor = "multi_thread")]
async fn pc_host_editado_envia_o_livro_com_revision_mais_um() {
    let (agent, mut mac) = host_pc_with_mac03().await;
    assert_eq!(agent.core.status().layout_host, Host::Local);
    mac.received.clear();

    let res = agent.core.save_layout(pc_profile("trabalho")).unwrap();
    assert!(res.warnings.is_empty());
    mac.pump_until("LayoutBookSync novo", |m| {
        m.got(|x| {
            matches!(x, Message::LayoutBookSync(b) if b.revision == 5
                && b.host == Machine::Windows
                && b.active.as_deref() == Some("trabalho")
                && b.profiles.len() == 2)
        })
    })
    .await;

    // Gravado no config (e em disco) e anunciado à UI; o Receiver passa a usar o perfil ativo.
    assert_eq!(agent.core.config().layout.revision, 5);
    let on_disk = relay_core::config::load_or_default(&agent.config_path).unwrap();
    assert_eq!(on_disk.layout.active.as_deref(), Some("trabalho"));
    assert_eq!(
        agent.rec.layouts.lock().unwrap().last().unwrap().revision,
        5
    );
    assert_eq!(agent.core.get_layout().name, "trabalho");
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn trocar_perfis_e_host_pelo_pc_host() {
    let (agent, mut mac) = host_pc_with_mac03().await;
    agent
        .core
        .set_layout_profiles(vec![pc_profile("a"), pc_profile("b")], "b".into())
        .unwrap();
    assert!(
        agent
            .core
            .set_layout_profiles(vec![pc_profile("a")], "x".into())
            .is_err()
    );
    // Passar o host ao Mac: o livro sai com host Mac e o PC vira só espelho.
    agent.core.set_layout_host(Host::Peer).unwrap();
    mac.pump_until("host passado ao Mac", |m| {
        m.got(|x| matches!(x, Message::LayoutBookSync(b) if b.host == Machine::Mac && b.revision == 6))
    })
    .await;
    eventually("PC sem host", || {
        agent.core.status().layout_host == Host::Peer
    })
    .await;
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn mac_02_deixa_o_pc_em_so_leitura() {
    let agent = start_agent_cfg(Arc::new(MemoryKeyStore::default()), None, |c| {
        c.layout.host = Machine::Windows;
    });
    let id = identity();
    let peer = pair(&agent, &id).await;
    let mut mac = connect_as(&agent, &id, peer, "0.2.0").await;
    mac.pump_until("conectado", |m| {
        m.got(|x| matches!(x, Message::Displays(_)))
    })
    .await;
    eventually("só-leitura", || {
        let s = agent.core.status();
        s.status == ConnectionStatus::Connected && !s.layout_editable
    })
    .await;
    assert_eq!(agent.core.status().layout_host, Host::Peer);
    let err = |r: Result<(), AgentError>| r.unwrap_err().to_string();
    assert_eq!(
        agent
            .core
            .save_layout(pc_profile("x"))
            .unwrap_err()
            .to_string(),
        "layout_read_only"
    );
    assert_eq!(
        err(agent
            .core
            .set_layout_profiles(vec![pc_profile("x")], "x".into())),
        "layout_read_only"
    );
    assert_eq!(
        err(agent.core.set_layout_host(Host::Local)),
        "layout_read_only"
    );
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn livro_do_mac_host_vira_o_perfil_do_receiver_e_so_leitura_sem_ele() {
    let agent = start_agent();
    let id = identity();
    let peer = pair(&agent, &id).await;
    let mut mac = connect_as(&agent, &id, peer, MAC_03).await;
    mac.pump_until("livro inicial", |m| {
        m.got(|x| matches!(x, Message::LayoutBookSync(_)))
    })
    .await;
    mac.dialer
        .send(Message::LayoutBookSync(book(3, Machine::Mac)))
        .await
        .unwrap();
    eventually("livro adotado", || {
        agent.core.get_layout_book() == book(3, Machine::Mac)
    })
    .await;
    eventually("relay:layout emitido", || {
        let l = agent.rec.layouts.lock().unwrap();
        l.last().is_some_and(|b| b.revision == 3)
    })
    .await;
    // Mac host conectado e simétrico: o PC edita por proposta (LayoutBookEdit), sem mudar já.
    assert!(agent.core.status().layout_editable);
    agent.core.save_layout(pc_profile("proposta")).unwrap();
    mac.pump_until("proposta ao host", |m| {
        m.got(
            |x| matches!(x, Message::LayoutBookEdit(b) if b.active.as_deref() == Some("proposta")),
        )
    })
    .await;
    assert_eq!(agent.core.get_layout_book().revision, 3);
    // Sem o Mac: só-leitura.
    drop(mac);
    eventually("desconectou", || !agent.core.status().layout_editable).await;
    assert_eq!(
        agent
            .core
            .save_layout(pc_profile("x"))
            .unwrap_err()
            .to_string(),
        "layout_read_only"
    );
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn telas_novas_criam_perfil_quando_o_pc_e_host() {
    let (agent, mut mac) = host_pc_with_mac03().await;
    // O livro inicial só tem a chave {dell, mac}; um monitor novo no PC muda o conjunto.
    agent.displays.lock().unwrap().push(display(
        "dell2",
        Machine::Windows,
        (1920, 1080),
        (527.0, 296.0),
        false,
    ));
    (agent.display_cb.lock().unwrap().as_ref().unwrap())();
    mac.pump_until("perfil do conjunto novo", |m| {
        m.got(
            |x| matches!(x, Message::LayoutBookSync(b) if b.revision == 5 && b.profiles.len() == 2),
        )
    })
    .await;
    let payload = agent.core.get_displays();
    assert_eq!((payload.local.len(), payload.peer.len()), (2, 1));
    assert!(!agent.rec.displays.lock().unwrap().is_empty());
    agent.core.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn save_config_with_a_stale_copy_does_not_revert_the_layout() {
    let (agent, _mac) = host_pc_with_mac03().await;
    let stale = agent.core.config();
    agent.core.save_layout(pc_profile("novo")).unwrap();
    agent.core.set_layout_host(Host::Peer).unwrap();
    eventually("host passado", || agent.core.config().layout.revision == 6).await;
    agent.core.save_config(stale).unwrap();
    let cfg = agent.core.config();
    assert_eq!((cfg.layout.revision, cfg.layout.host), (6, Machine::Mac));
    assert_eq!(agent.core.get_layout_book().revision, 6);
    let on_disk = relay_core::config::load_or_default(&agent.config_path).unwrap();
    assert_eq!(on_disk.layout.revision, 6);
    agent.core.shutdown();
}
