//! dono: MI3
//! Arquivos (D20). Mac→PC: o Mac serve os caminhos da última lista oferecida (clipboard ou
//! arraste) num `TransferServer`, e o PC baixa quando o Explorer cola. PC→Mac: Cmd+V com o
//! Finder na frente (ou "Receber arquivos") baixa para `~/Downloads/Relay` e revela no Finder.
//! Arraste Mac→PC: se o botão esquerdo estava pressionado ao entrar no PC e o pasteboard de
//! arraste tem arquivos, o mouse up remoto manda `DropFiles`.

use std::net::{SocketAddr, TcpListener as StdListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use relay_core::transfer::{TransferClient, TransferServer};
use relay_core::types::{FileEntry, Message, TransferProgress};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::Sh;
use crate::core::SessionCtx;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Ponto do `DropFiles` quando o Mac não sabe onde o cursor está no PC: o agente usa a posição
/// atual do cursor dele.
pub(super) const DROP_AT_CURSOR: (i32, i32) = (-1, -1);

/// Servidor de transferência do Mac (porta `transfer`) e a lista que ele serve.
#[derive(Default)]
pub(crate) struct Files {
    inner: Mutex<FilesInner>,
}

#[derive(Default)]
struct FilesInner {
    server: Arc<Mutex<Option<Arc<TransferServer>>>>,
    addr: Option<SocketAddr>,
    task: Option<JoinHandle<()>>,
    /// Download em andamento (um por vez).
    download: Option<Arc<Download>>,
    next_transfer: u32,
}

struct Download {
    cancel: AtomicBool,
    wake: Notify,
}

impl Files {
    fn lock(&self) -> std::sync::MutexGuard<'_, FilesInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        self.lock().addr
    }

    pub fn shutdown(&self) {
        let mut g = self.lock();
        if let Some(t) = g.task.take() {
            t.abort();
        }
        if let Some(d) = g.download.take() {
            cancel(&d);
        }
    }

    /// Abre o servidor (uma vez) na porta local `transfer`.
    pub fn listen(&self, sh: &Sh, ctx: &SessionCtx) {
        let mut g = self.lock();
        if g.task.is_some() {
            return;
        }
        let bound = StdListener::bind((ctx.bind_ip, ctx.local_ports.transfer)).and_then(|l| {
            l.set_nonblocking(true)?;
            Ok(l)
        });
        let std_listener = match bound {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("transferência: porta {}: {e}", ctx.local_ports.transfer);
                return;
            }
        };
        g.addr = std_listener.local_addr().ok();
        let server = g.server.clone();
        let _enter = sh.rt.enter();
        g.task = Some(sh.rt.spawn(async move {
            let Ok(listener) = TcpListener::from_std(std_listener) else {
                return;
            };
            accept_loop(listener, server).await;
        }));
    }

    /// A partir de agora o PC baixa estes caminhos (`FileEntry.id` = índice).
    pub fn set_paths(&self, sh: &Sh, paths: Vec<PathBuf>) {
        let Some(ctx) = sh.session() else { return };
        let server = TransferServer::new(ctx.identity.clone(), vec![ctx.peer.clone()], paths);
        *self.lock().server.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(server));
    }
}

async fn accept_loop(listener: TcpListener, server: Arc<Mutex<Option<Arc<TransferServer>>>>) {
    while let Ok((mut stream, remote)) = listener.accept().await {
        let _ = stream.set_nodelay(true);
        let current = server.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(current) = current else { continue };
        tokio::spawn(async move {
            if let Err(e) = current.serve_connection(&mut stream).await {
                tracing::warn!(%remote, "transferência encerrada: {e}");
            }
        });
    }
}

fn cancel(d: &Download) {
    d.cancel.store(true, Ordering::Relaxed);
    d.wake.notify_one();
}

/// A sessão caiu: aborta o download em andamento (o stream fecha) e esquece a lista servida.
pub(super) fn session_down(sh: &Sh) {
    let mut g = sh.files.lock();
    if let Some(d) = g.download.take() {
        cancel(&d);
    }
    *g.server.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

// ---- PC → Mac ----

/// Baixa a lista pendente do PC para `~/Downloads/Relay` e revela no Finder.
pub(super) fn receive_files(sh: &Arc<Sh>) {
    let Some(ctx) = sh.session() else { return };
    let Some(files) = sh.st().pending.clone() else {
        tracing::info!("nenhum arquivo do PC à espera");
        return;
    };
    let (dl, transfer_id) = {
        let mut g = sh.files.lock();
        if g.download.is_some() {
            return;
        }
        g.next_transfer = g.next_transfer.wrapping_add(1);
        let dl = Arc::new(Download {
            cancel: AtomicBool::new(false),
            wake: Notify::new(),
        });
        g.download = Some(dl.clone());
        (dl, g.next_transfer)
    };
    let sh = sh.clone();
    let rt = sh.rt.clone();
    // O callback de progresso do core não é `Send`: o future roda numa thread bloqueante.
    let handle = rt.clone();
    handle.spawn_blocking(move || {
        rt.block_on(async move {
            let result = download(&sh, &ctx, &files, transfer_id, &dl).await;
            {
                let mut g = sh.files.lock();
                if g.download.as_ref().is_some_and(|d| Arc::ptr_eq(d, &dl)) {
                    g.download = None;
                }
            }
            match result {
                Ok(paths) => {
                    sh.st().pending = None;
                    sh.platform.reveal(&paths);
                }
                // O core já emitiu o fim com erro; falha antes de começar emite aqui.
                Err(None) => {}
                Err(Some(e)) => sh.events.transfer_progress(&TransferProgress {
                    transfer_id,
                    file_name: String::new(),
                    bytes: 0,
                    total: 0,
                    done: true,
                    error: Some(e),
                }),
            }
        })
    });
}

/// `Err(None)`: o erro já foi publicado como progresso final pelo core.
async fn download(
    sh: &Sh,
    ctx: &SessionCtx,
    files: &[FileEntry],
    transfer_id: u32,
    dl: &Download,
) -> Result<Vec<PathBuf>, Option<String>> {
    let dest = sh.platform.downloads_dir();
    std::fs::create_dir_all(&dest).map_err(|e| Some(format!("{}: {e}", dest.display())))?;
    let addr = SocketAddr::new(ctx.peer_ip, ctx.remote_ports.transfer);
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| Some("tempo esgotado ao conectar no PC".to_string()))?
        .map_err(|e| Some(e.to_string()))?;
    let _ = stream.set_nodelay(true);
    let ids: Vec<u32> = files.iter().map(|f| f.id).collect();
    let client = TransferClient::new(ctx.identity.clone(), ctx.peer.clone());
    let mut progress = |p: TransferProgress| sh.events.transfer_progress(&p);
    let run = client.download(
        &mut stream,
        &ids,
        &dest,
        transfer_id,
        &dl.cancel,
        &mut progress,
    );
    // `cancel` só vale entre blocos; o `wake` fecha o stream (derruba o future) de vez.
    tokio::select! {
        r = run => r.map_err(|_| None),
        () = dl.wake.notified() => Err(None),
    }
}

/// Cmd+V no Finder com arquivos do PC pendentes.
pub(super) fn paste_requested(sh: &Arc<Sh>) {
    if sh.st().pending.is_some() && sh.platform.finder_frontmost() {
        receive_files(sh);
    }
}

// ---- arraste Mac → PC ----

pub(super) fn focus_changed(sh: &Arc<Sh>, remote: bool, left_down: bool) {
    if !(remote && left_down) {
        sh.st().armed = None;
        return;
    }
    let paths = sh.platform.drag_files();
    if paths.is_empty() {
        return;
    }
    let files: Vec<FileEntry> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| entry_for(i as u32, p))
        .collect();
    let offer_id = {
        let mut st = sh.st();
        st.next_offer = st.next_offer.wrapping_add(1);
        st.armed = Some(st.next_offer);
        st.next_offer
    };
    sh.files.set_paths(sh, paths);
    sh.send(Message::FileOffer { offer_id, files });
}

pub(super) fn left_button_up(sh: &Arc<Sh>) {
    let armed = sh.st().armed.take();
    if let Some(offer_id) = armed {
        sh.send(Message::DropFiles {
            offer_id,
            point_px: DROP_AT_CURSOR,
        });
    }
}

fn entry_for(id: u32, path: &std::path::Path) -> FileEntry {
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    FileEntry {
        id,
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: if is_dir {
            dir_size(path)
        } else {
            meta.map_or(0, |m| m.len())
        },
        is_dir,
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}
