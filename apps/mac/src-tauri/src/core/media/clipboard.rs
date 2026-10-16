//! dono: MI3
//! Clipboard: um trabalhador é dono do `ClipboardBackend` (marcador anti-eco no backend) e do
//! `ClipboardSync` (anti-eco por hash, limite de tamanho, modos). Mudança local, troca de foco e
//! "enviar agora" viram ofertas; ofertas do PC viram escrita marcada no pasteboard. Uma lista de
//! arquivos do PC não é escrita: fica pendente até o usuário colar no Finder ou pedir (D20).

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use relay_core::clipboard::ClipboardSync;
use relay_core::platform::ClipboardBackend;
use relay_core::types::{ClipboardConfig, ClipboardContent, Message};

use super::Sh;

pub(super) enum ClipCmd {
    /// O pasteboard mudou (callback do backend, que já ignora escritas marcadas).
    Local,
    /// Troca de foco com `mode = on_switch`.
    Switch,
    SendNow,
    Msg(Message),
    Config(ClipboardConfig),
    Quit,
}

const TICK: Duration = Duration::from_millis(500);

pub(super) fn spawn(sh: Arc<Sh>) -> Sender<ClipCmd> {
    let (tx, rx) = mpsc::channel();
    let mut backend = sh.platform.clipboard();
    let cb = tx.clone();
    backend.set_change_callback(Box::new(move || {
        let _ = cb.send(ClipCmd::Local);
    }));
    let spawned = std::thread::Builder::new()
        .name("relay-clipboard".into())
        .spawn(move || run(&sh, backend, &rx));
    if let Err(e) = spawned {
        tracing::warn!("clipboard desligado: {e}");
    }
    tx
}

fn run(sh: &Sh, mut backend: Box<dyn ClipboardBackend>, rx: &mpsc::Receiver<ClipCmd>) {
    let mut sync = ClipboardSync::new(ClipboardConfig::default());
    loop {
        let cmd = match rx.recv_timeout(TICK) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                sync.expire(std::time::Instant::now());
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match cmd {
            ClipCmd::Quit => return,
            ClipCmd::Config(c) => sync.set_config(c),
            ClipCmd::Local => {
                if let Some(c) = read(sh, &mut *backend) {
                    send_all(sh, sync.on_local_change(c));
                }
            }
            ClipCmd::Switch => {
                let current = read(sh, &mut *backend);
                send_all(sh, sync.on_switch(current));
            }
            ClipCmd::SendNow => {
                let current = read(sh, &mut *backend);
                send_all(sh, sync.send_now(current));
            }
            ClipCmd::Msg(Message::ClipboardSkipped { reason }) => {
                tracing::info!("o PC não enviou o clipboard: {reason}");
            }
            ClipCmd::Msg(msg) => {
                if let Some(content) = sync.on_message(msg) {
                    apply_remote(sh, &mut *backend, content);
                }
            }
        }
    }
}

/// Lê o pasteboard; uma lista de arquivos também aponta o servidor de transferência para eles.
fn read(sh: &Sh, backend: &mut dyn ClipboardBackend) -> Option<ClipboardContent> {
    let content = match backend.read() {
        Ok(c) => c?,
        Err(e) => {
            tracing::warn!("não consegui ler o clipboard: {e}");
            return None;
        }
    };
    if matches!(content, ClipboardContent::FileList(_)) {
        sh.files.set_paths(sh, backend.local_paths());
    }
    Some(content)
}

fn send_all(sh: &Sh, msgs: Vec<Message>) {
    for m in msgs {
        sh.send(m);
    }
}

fn apply_remote(sh: &Sh, backend: &mut dyn ClipboardBackend, content: ClipboardContent) {
    match content {
        ClipboardContent::FileList(files) => sh.st().pending = Some(files),
        other => {
            if let Err(e) = backend.write_remote(other) {
                tracing::warn!("não consegui aplicar o clipboard do PC: {e}");
            }
        }
    }
}
