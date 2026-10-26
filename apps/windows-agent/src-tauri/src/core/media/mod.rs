//! dono: WI3
//! Mídia do agente: áudio (som do PC → Mac e microfone do Mac → VB-CABLE), clipboard e arquivos.
//!
//! O núcleo (WI1) fala com a mídia por três pontos: `message_hook` (mensagens recebidas),
//! `engine_events` (troca de foco / atalho "enviar agora") e [`MediaPort`] (queda/volta da sessão,
//! config, estado de áudio e saída assíncrona). Tudo o que bloqueia (Win32, rede, disco) roda numa
//! thread própria; o pump só enfileira.

pub mod audio;
pub mod files;
pub mod platform;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use relay_core::clipboard::ClipboardSync;
use relay_core::crypto::{StaticKeypair, load_active_peer, load_or_create_identity};
use relay_core::engine::{EngineEvent, EventHandler};
use relay_core::net::AudioLink;
use relay_core::platform::{ClipboardBackend, KeyStore};
use relay_core::types::{
    AudioState, ClipboardContent, ClipboardFormat, CommandResult, Config, FileEntry, HotkeyAction,
    Message, PairedPeer,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use self::audio::{AudioEngine, SharedAudioState};
use self::files::FileServer;
use self::platform::{AudioHost, DropTarget};
use super::{ACTIVE_PEER_ACCOUNT, MessageHook};

/// Saída assíncrona da mídia para o pump.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaOut {
    /// Mensagem a enviar ao Mac.
    Send(Message),
    /// `audio_state()` mudou: republicar o snapshot.
    State,
}

/// Ligação da mídia ao pump (parte do `AgentDeps`).
pub trait MediaPort: Send {
    /// `Some` quando a sessão fica `Connected` (com IP do Mac e chaves de áudio), `None` ao cair.
    fn link_changed(&mut self, link: Option<AudioLink>);
    fn config_changed(&mut self, config: &Config);
    fn audio_state(&self) -> AudioState;
    /// "Enviar clipboard agora" (mesmo efeito do atalho).
    fn clipboard_send_now(&mut self);
    /// Fila de saída; devolvida uma única vez.
    fn take_outbox(&mut self) -> Option<UnboundedReceiver<MediaOut>>;
}

/// Fonte dos PNGs de `CaptureScreen` (WI2 empilha; a mídia esvazia e oferece ao Mac).
pub type CaptureSource = Box<dyn FnMut() -> Vec<Vec<u8>> + Send>;

pub struct MediaDeps {
    pub keystore: Arc<dyn KeyStore>,
    pub clipboard: Box<dyn ClipboardBackend>,
    pub audio: Arc<dyn AudioHost>,
    pub drop_target: DropTarget,
    /// Raiz das pastas temporárias das ofertas (`%TEMP%`).
    pub temp_root: PathBuf,
    pub captures: CaptureSource,
}

/// Endereços reais de escuta (úteis com portas efêmeras e nos testes).
#[derive(Debug, Clone, Copy, Default)]
pub struct MediaAddrs {
    pub transfer: Option<SocketAddr>,
    pub mic: Option<SocketAddr>,
}

enum Cmd {
    Msg(Message),
    Link(Option<AudioLink>),
    Config(Box<Config>),
    ClipboardChanged,
    ClipboardSwitch,
    ClipboardSendNow,
    Shutdown,
}

/// Quem baixa arquivos do Mac precisa de identidade, peer e endereço.
#[derive(Default)]
struct RemoteCtx {
    addr: Option<SocketAddr>,
    peer: Option<PairedPeer>,
    identity: Option<StaticKeypair>,
    /// Última oferta de clipboard recebida (nome da pasta temporária).
    offer_id: u32,
}

pub struct Media {
    tx: mpsc::Sender<Cmd>,
    state: SharedAudioState,
    addrs: Arc<Mutex<MediaAddrs>>,
    outbox: Option<UnboundedReceiver<MediaOut>>,
    thread: Option<JoinHandle<()>>,
}

impl Media {
    pub fn start(deps: MediaDeps, config: &Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let (out, outbox) = unbounded_channel();
        let state: SharedAudioState = Arc::default();
        let addrs: Arc<Mutex<MediaAddrs>> = Arc::default();
        let mut w = Worker::new(
            deps,
            config.clone(),
            rx,
            out,
            state.clone(),
            addrs.clone(),
            tx.clone(),
        );
        w.install_hooks();
        let thread = std::thread::Builder::new()
            .name("relay-media".into())
            .spawn(move || w.run())
            .ok();
        Self {
            tx,
            state,
            addrs,
            outbox: Some(outbox),
            thread,
        }
    }

    /// Gancho de mensagens recebidas (`AgentDeps::message_hook`). Não bloqueia.
    pub fn hook(&self) -> MessageHook {
        let tx = self.tx.clone();
        Box::new(move |msg| {
            if matches!(
                msg,
                Message::ClipboardOffer { .. }
                    | Message::ClipboardChunk { .. }
                    | Message::FileOffer { .. }
                    | Message::DropFiles { .. }
                    | Message::AudioControl { .. }
            ) {
                let _ = tx.send(Cmd::Msg(msg.clone()));
            }
            Vec::new()
        })
    }

    /// Eventos do motor (`AgentDeps::engine_events`): troca de foco e "enviar agora".
    pub fn engine_events(&self) -> EventHandler {
        let tx = self.tx.clone();
        Box::new(move |ev| match ev {
            EngineEvent::ClipboardSwitch => {
                let _ = tx.send(Cmd::ClipboardSwitch);
            }
            EngineEvent::Hotkey(HotkeyAction::ClipboardSendNow) => {
                let _ = tx.send(Cmd::ClipboardSendNow);
            }
            _ => {}
        })
    }

    pub fn addrs(&self) -> MediaAddrs {
        *self.addrs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl MediaPort for Media {
    fn link_changed(&mut self, link: Option<AudioLink>) {
        let _ = self.tx.send(Cmd::Link(link));
    }

    fn config_changed(&mut self, config: &Config) {
        let _ = self.tx.send(Cmd::Config(Box::new(config.clone())));
    }

    fn audio_state(&self) -> AudioState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clipboard_send_now(&mut self) {
        let _ = self.tx.send(Cmd::ClipboardSendNow);
    }

    fn take_outbox(&mut self) -> Option<UnboundedReceiver<MediaOut>> {
        self.outbox.take()
    }
}

impl Drop for Media {
    /// Desliga o áudio e restaura a saída padrão antes de devolver o controle.
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

const TICK: Duration = Duration::from_millis(50);
/// Ofertas de arquivos lembradas (para `DropFiles` e colagem).
const MAX_OFFERS: usize = 8;

struct Worker {
    rx: mpsc::Receiver<Cmd>,
    tx: mpsc::Sender<Cmd>,
    out: UnboundedSender<MediaOut>,
    keystore: Arc<dyn KeyStore>,
    clipboard: Box<dyn ClipboardBackend>,
    sync: ClipboardSync,
    audio: AudioEngine,
    state: SharedAudioState,
    addrs: Arc<Mutex<MediaAddrs>>,
    drop_target: DropTarget,
    captures: CaptureSource,
    config: Config,
    link: Option<AudioLink>,
    ctx: Arc<Mutex<RemoteCtx>>,
    offers: VecDeque<(u32, Vec<FileEntry>)>,
    server: FileServer,
    rt: Arc<tokio::runtime::Runtime>,
    temp_root: PathBuf,
}

impl Worker {
    fn new(
        deps: MediaDeps,
        config: Config,
        rx: mpsc::Receiver<Cmd>,
        out: UnboundedSender<MediaOut>,
        state: SharedAudioState,
        addrs: Arc<Mutex<MediaAddrs>>,
        tx: mpsc::Sender<Cmd>,
    ) -> Self {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("relay-media-io")
                .enable_all()
                .build()
                .expect("runtime da mídia"),
        );
        Self {
            rx,
            tx,
            out,
            keystore: deps.keystore,
            clipboard: deps.clipboard,
            sync: ClipboardSync::new(config.clipboard.clone()),
            audio: AudioEngine::new(deps.audio),
            state,
            addrs,
            drop_target: deps.drop_target,
            captures: deps.captures,
            config,
            link: None,
            ctx: Arc::default(),
            offers: VecDeque::new(),
            server: FileServer::default(),
            rt,
            temp_root: deps.temp_root,
        }
    }

    /// Registra os callbacks do clipboard. Roda antes da thread subir, para uma cópia logo após
    /// `Media::start` não se perder (corrida vista no CI do Windows).
    fn install_hooks(&mut self) {
        let tx = Mutex::new(self.tx.clone());
        self.clipboard.set_change_callback(Box::new(move || {
            let _ = tx
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::ClipboardChanged);
        }));
        self.install_file_provider();
    }

    fn run(mut self) {
        loop {
            match self.rx.recv_timeout(TICK) {
                Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.on_cmd(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        // Restaura a saída padrão antes de largar o runtime.
        self.audio.stop_all();
        self.publish_audio();
    }

    fn send(&self, m: Message) {
        let _ = self.out.send(MediaOut::Send(m));
    }

    fn send_all(&self, msgs: Vec<Message>) {
        msgs.into_iter().for_each(|m| self.send(m));
    }

    fn publish_audio(&self) {
        let st = self.audio.state();
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = st;
        self.addrs.lock().unwrap_or_else(|e| e.into_inner()).mic = self.audio.mic_addr();
        let _ = self.out.send(MediaOut::State);
    }

    fn tick(&mut self) {
        self.sync.expire(Instant::now());
        // PNGs de CaptureScreen viram oferta de clipboard (D7); nunca vão no fio como resultado.
        for png in (self.captures)() {
            // Ação explícita do usuário: vale mesmo com a sincronização de clipboard desligada.
            let mut forced = self.config.clipboard.clone();
            forced.enabled = true;
            if !forced.formats.contains(&ClipboardFormat::Png) {
                forced.formats.push(ClipboardFormat::Png);
            }
            self.sync.set_config(forced);
            let msgs = self.sync.send_now(Some(ClipboardContent::Png(png)));
            self.sync.set_config(self.config.clipboard.clone());
            self.send_all(msgs);
        }
    }

    fn on_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Msg(m) => self.on_message(m),
            Cmd::Link(l) => self.on_link(l),
            Cmd::Config(c) => {
                self.sync.set_config(c.clipboard.clone());
                self.config = *c;
            }
            Cmd::ClipboardChanged => {
                if let Some(c) = self.current() {
                    let msgs = self.sync.on_local_change(c);
                    self.send_all(msgs);
                }
            }
            Cmd::ClipboardSwitch => {
                let cur = self.current();
                let msgs = self.sync.on_switch(cur);
                self.send_all(msgs);
            }
            Cmd::ClipboardSendNow => {
                let cur = self.current();
                let msgs = self.sync.send_now(cur);
                self.send_all(msgs);
            }
            Cmd::Shutdown => {}
        }
    }

    /// Lê o clipboard local; uma `FileList` passa a ser servida pelo `TransferServer`.
    fn current(&mut self) -> Option<ClipboardContent> {
        let c = self.clipboard.read().ok().flatten()?;
        if matches!(c, ClipboardContent::FileList(_)) {
            self.serve_local_files();
        }
        Some(c)
    }

    fn serve_local_files(&mut self) {
        let (identity, peer) = {
            let ctx = self.ctx.lock().unwrap_or_else(|e| e.into_inner());
            (ctx.identity.clone(), ctx.peer.clone())
        };
        if let (Some(identity), Some(peer)) = (identity, peer) {
            self.server
                .serve(identity, peer, self.clipboard.local_paths());
        }
    }

    fn on_link(&mut self, link: Option<AudioLink>) {
        if let Some(l) = &link {
            let identity = load_or_create_identity(&*self.keystore).ok();
            let peer = load_active_peer(&*self.keystore, ACTIVE_PEER_ACCOUNT)
                .ok()
                .flatten();
            let mut ctx = self.ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.addr = Some(SocketAddr::new(
                l.remote_ip,
                self.config.network.ports.transfer,
            ));
            ctx.identity = identity;
            ctx.peer = peer;
            drop(ctx);
            let addr = self
                .server
                .ensure_listening(self.rt.handle(), self.config.network.ports.transfer);
            self.addrs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .transfer = addr;
        } else {
            self.audio.stop_all();
            self.publish_audio();
        }
        self.link = link;
    }

    fn on_message(&mut self, msg: Message) {
        match msg {
            Message::AudioControl { mac_to_pc, .. } => self.on_audio_control(mac_to_pc),
            Message::FileOffer { offer_id, files } => self.remember_offer(offer_id, files),
            Message::DropFiles { offer_id, .. } => self.on_drop(offer_id),
            other => {
                if let Message::ClipboardOffer { offer_id, .. } = &other {
                    self.ctx.lock().unwrap_or_else(|e| e.into_inner()).offer_id = *offer_id;
                }
                if let Some(content) = self.sync.on_message(other) {
                    self.apply_remote(content);
                }
            }
        }
    }

    fn remember_offer(&mut self, offer_id: u32, files: Vec<FileEntry>) {
        self.offers.retain(|(id, _)| *id != offer_id);
        self.offers.push_back((offer_id, files));
        while self.offers.len() > MAX_OFFERS {
            self.offers.pop_front();
        }
    }

    /// Conteúdo remoto completo: vai ao clipboard com o marcador `RelayRemote`. Uma `FileList`
    /// vira CF_HDROP com renderização atrasada (o provedor baixa quando o Explorer cola).
    fn apply_remote(&mut self, content: ClipboardContent) {
        if let ClipboardContent::FileList(files) = &content {
            let id = self.ctx.lock().unwrap_or_else(|e| e.into_inner()).offer_id;
            self.remember_offer(id, files.clone());
        }
        if let Err(e) = self.clipboard.write_remote(content) {
            tracing::warn!(error = %e, "não consegui gravar o clipboard remoto");
        }
    }

    fn install_file_provider(&mut self) {
        let ctx = self.ctx.clone();
        let rt = self.rt.clone();
        let root = self.temp_root.clone();
        self.clipboard.set_file_provider(Box::new(move |entries| {
            let (addr, peer, identity, offer_id) = {
                let c = ctx.lock().unwrap_or_else(|e| e.into_inner());
                (c.addr, c.peer.clone(), c.identity.clone(), c.offer_id)
            };
            let (Some(addr), Some(peer), Some(identity)) = (addr, peer, identity) else {
                return Err("sem sessão com o Mac".into());
            };
            let ids = entries.iter().map(|e| e.id).collect();
            let dest = files::offer_dir(&root, offer_id);
            // Chamado pela thread da janela do clipboard (WM_RENDERFORMAT): bloqueia até baixar.
            rt.block_on(files::download(identity, peer, addr, ids, dest))
        }));
    }

    /// `DropFiles`: baixa a oferta para a pasta do Explorer sob o cursor ou, sem ela, Downloads.
    /// `point_px` (o Mac manda (-1,-1) = "sob o cursor atual") é ignorado de propósito: o destino
    /// sempre usa a posição real do cursor (`explorer_folder_at_cursor` lê `GetCursorPos`).
    fn on_drop(&mut self, offer_id: u32) {
        let Some((_, entries)) = self.offers.iter().find(|(id, _)| *id == offer_id) else {
            tracing::warn!(offer_id, "DropFiles de oferta desconhecida");
            return;
        };
        let ids: Vec<u32> = entries.iter().map(|e| e.id).collect();
        let Some(dest) = (self.drop_target)() else {
            tracing::warn!("sem pasta de destino para o arraste");
            return;
        };
        let (addr, peer, identity) = {
            let c = self.ctx.lock().unwrap_or_else(|e| e.into_inner());
            (c.addr, c.peer.clone(), c.identity.clone())
        };
        let (Some(addr), Some(peer), Some(identity)) = (addr, peer, identity) else {
            return;
        };
        // O futuro do download não é `Send` (progresso é `&mut dyn FnMut`): thread própria.
        let rt = self.rt.clone();
        std::thread::spawn(move || {
            if let Err(e) = rt.block_on(files::download(identity, peer, addr, ids, dest)) {
                tracing::warn!(error = %e, "arraste falhou");
            }
        });
    }

    fn on_audio_control(&mut self, mic: bool) {
        let o = self.audio.apply(
            mic,
            self.link.as_ref(),
            &self.config.audio,
            &self.config.network.ports,
        );
        self.publish_audio();
        if let Some(err) = o.error {
            // Sem mensagem nova (contrato no mural): estado real + `CommandResult` de id 0.
            self.send(Message::AudioControl {
                pc_to_mac: false,
                mac_to_pc: o.state.mic_from_peer,
            });
            self.send(Message::CommandResult {
                id: 0,
                result: CommandResult::err(0, err),
            });
        }
    }
}
