//! dono: WI3
//! Arquivos (D20): o agente serve a `FileList` do clipboard do Windows (TCP 47804) e baixa as
//! ofertas do Mac (renderização atrasada do CF_HDROP e `DropFiles`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use relay_core::crypto::StaticKeypair;
use relay_core::transfer::{TransferClient, TransferServer};
use relay_core::types::{PairedPeer, TransferProgress};
use tokio::runtime::Handle;

static NEXT_TRANSFER: AtomicU32 = AtomicU32::new(1);

pub fn next_transfer_id() -> u32 {
    NEXT_TRANSFER.fetch_add(1, Ordering::Relaxed)
}

/// Baixa `ids` da oferta do peer em `addr` para `dest` (criada se não existir).
pub async fn download(
    identity: StaticKeypair,
    peer: PairedPeer,
    addr: SocketAddr,
    ids: Vec<u32>,
    dest: PathBuf,
) -> Result<Vec<PathBuf>, String> {
    tokio::fs::create_dir_all(&dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut io = TransferClient::connect(addr)
        .await
        .map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let mut log = |p: TransferProgress| {
        if p.done {
            tracing::debug!(arquivo = %p.file_name, erro = ?p.error, "transferência concluída");
        }
    };
    TransferClient::new(identity, peer)
        .download(&mut io, &ids, &dest, next_transfer_id(), &cancel, &mut log)
        .await
        .map_err(|e| e.to_string())
}

/// Servidor da `FileList` local: uma escuta fixa; cada nova lista troca os caminhos servidos.
#[derive(Default)]
pub struct FileServer {
    current: Arc<Mutex<Option<Arc<TransferServer>>>>,
    addr: Option<SocketAddr>,
}

impl FileServer {
    /// Abre a escuta (uma vez) em `0.0.0.0:port`. Devolve o endereço real.
    pub fn ensure_listening(&mut self, rt: &Handle, port: u16) -> Option<SocketAddr> {
        if self.addr.is_some() {
            return self.addr;
        }
        let listener = rt
            .block_on(tokio::net::TcpListener::bind(SocketAddr::from((
                [0, 0, 0, 0],
                port,
            ))))
            .map_err(|e| tracing::warn!(error = %e, "porta de transferência indisponível"))
            .ok()?;
        self.addr = listener.local_addr().ok();
        let current = self.current.clone();
        rt.spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.set_nodelay(true);
                let server = current.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(server) = server {
                    tokio::spawn(async move {
                        if let Err(e) = server.serve_connection(&mut stream).await {
                            tracing::warn!("transferência encerrada: {e}");
                        }
                    });
                }
            }
        });
        self.addr
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// Passa a servir `paths` (índice = `FileEntry.id`) para o `peer`.
    pub fn serve(&self, identity: StaticKeypair, peer: PairedPeer, paths: Vec<PathBuf>) {
        let server = Arc::new(TransferServer::new(identity, vec![peer], paths));
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(server);
    }
}

/// Pasta temporária da oferta: `%TEMP%\Relay\<offer_id>`.
pub fn offer_dir(root: &Path, offer_id: u32) -> PathBuf {
    root.join("Relay").join(offer_id.to_string())
}
