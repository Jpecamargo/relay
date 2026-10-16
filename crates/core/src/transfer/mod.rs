//! dono: C11b
//! Transferência de arquivos sob demanda numa conexão TCP separada (porta 47804, decisão D4),
//! cifrada com o Noise KK do C2. O arquivo grande não atrasa tecla nem clique porque não divide
//! a conexão da sessão.
//!
//! Protocolo (cada mensagem é uma mensagem Noise dentro de um frame u32):
//! cliente -> servidor: `Request{ids}`; servidor -> cliente: `Begin{total}`, então para cada
//! entrada `Entry{caminho relativo, tamanho, is_dir}`, `Chunk*` e `FileEnd{BLAKE2s}`, e `Done`.
//! Cancelar é fechar a conexão. O cliente grava num diretório de staging dentro do destino e só
//! move cada item de primeiro nível para o lugar final quando ele chega inteiro.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use blake2::{Blake2s256, Digest};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::crypto::{
    CryptoError, NoiseSession, StaticKeypair, noise_accept, noise_dial, read_frame, write_frame,
};
use crate::types::consts::{CHUNK_SIZE, PORT_TRANSFER};
use crate::types::{PairedPeer, TransferProgress};

const MSG_REQUEST: u8 = 1;
const MSG_BEGIN: u8 = 2;
const MSG_ENTRY: u8 = 3;
const MSG_CHUNK: u8 = 4;
const MSG_FILE_END: u8 = 5;
const MSG_DONE: u8 = 6;
const MSG_ERROR: u8 = 7;

/// Máximo de ids por pedido (cabe numa mensagem Noise).
const MAX_IDS: usize = 4096;
/// Intervalo mínimo entre eventos de progresso (10 Hz).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("peer não pareado")]
    UnknownPeer,
    #[error("transferência cancelada")]
    Cancelled,
    #[error("protocolo: {0}")]
    Protocol(String),
    #[error("cripto: {0}")]
    Crypto(String),
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for TransferError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<CryptoError> for TransferError {
    fn from(e: CryptoError) -> Self {
        match e {
            CryptoError::UnknownPeer => Self::UnknownPeer,
            e => Self::Crypto(e.to_string()),
        }
    }
}

fn proto(msg: &str) -> TransferError {
    TransferError::Protocol(msg.to_string())
}

async fn send<S: AsyncWrite + Unpin>(
    io: &mut S,
    session: &mut NoiseSession,
    plain: &[u8],
    cipher: &mut Vec<u8>,
) -> Result<(), TransferError> {
    session.encrypt(plain, cipher)?;
    write_frame(io, cipher).await?;
    Ok(())
}

async fn recv<S: AsyncRead + Unpin>(
    io: &mut S,
    session: &mut NoiseSession,
    plain: &mut Vec<u8>,
) -> Result<(), TransferError> {
    let cipher = read_frame(io).await?;
    session.decrypt(&cipher, plain)?;
    if plain.is_empty() {
        return Err(proto("mensagem vazia"));
    }
    Ok(())
}

/// Item de um id da oferta, já achatado.
struct Item {
    rel: String,
    path: PathBuf,
    size: u64,
    is_dir: bool,
}

/// Achata `root` (arquivo ou diretório, recursivo) em itens com caminho relativo em `/`.
/// Links simbólicos dentro de diretórios são ignorados, para não sair da árvore oferecida.
fn collect(root: &Path, out: &mut Vec<Item>) -> std::io::Result<()> {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other("caminho sem nome"))?;
    let meta = std::fs::metadata(root)?;
    walk(root, &name, meta, out)
}

fn walk(
    path: &Path,
    rel: &str,
    meta: std::fs::Metadata,
    out: &mut Vec<Item>,
) -> std::io::Result<()> {
    if meta.is_dir() {
        out.push(Item {
            rel: rel.to_string(),
            path: path.to_path_buf(),
            size: 0,
            is_dir: true,
        });
        let mut children: Vec<_> = std::fs::read_dir(path)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|c| c.file_name());
        for c in children {
            let m = std::fs::symlink_metadata(c.path())?;
            if m.is_symlink() {
                continue;
            }
            let child_rel = format!("{rel}/{}", c.file_name().to_string_lossy());
            walk(&c.path(), &child_rel, m, out)?;
        }
    } else if meta.is_file() {
        out.push(Item {
            rel: rel.to_string(),
            path: path.to_path_buf(),
            size: meta.len(),
            is_dir: false,
        });
    }
    Ok(())
}

/// Serve os arquivos de uma oferta por id (índice em `paths`, o mesmo de `FileEntry.id`).
pub struct TransferServer {
    me: StaticKeypair,
    known: Vec<PairedPeer>,
    paths: Vec<PathBuf>,
}

impl std::fmt::Debug for TransferServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferServer")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl TransferServer {
    pub fn new(me: StaticKeypair, known: Vec<PairedPeer>, paths: Vec<PathBuf>) -> Self {
        Self { me, known, paths }
    }

    /// Escuta em `0.0.0.0:47804` (ou no endereço dado) e serve cada conexão numa tarefa.
    pub async fn bind(addr: Option<SocketAddr>) -> std::io::Result<TcpListener> {
        TcpListener::bind(addr.unwrap_or(SocketAddr::from(([0, 0, 0, 0], PORT_TRANSFER)))).await
    }

    /// Aceita conexões de `listener` até o erro de accept.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (mut stream, remote) = listener.accept().await?;
            let _ = stream.set_nodelay(true);
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = this.serve_connection(&mut stream).await {
                    tracing::warn!(%remote, "transferência encerrada: {e}");
                }
            });
        }
    }

    /// Faz o handshake (só aceita peer pareado) e atende um pedido.
    pub async fn serve_connection<S>(&self, io: &mut S) -> Result<(), TransferError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut session = noise_accept(io, &self.me, &self.known).await?;
        let mut plain = Vec::new();
        recv(io, &mut session, &mut plain).await?;
        let mut cipher = Vec::new();
        match self.respond(io, &mut session, &plain, &mut cipher).await {
            Ok(()) => Ok(()),
            Err(TransferError::Io(e)) | Err(TransferError::Protocol(e))
                if plain[0] == MSG_REQUEST =>
            {
                let mut msg = vec![MSG_ERROR];
                msg.extend_from_slice(e.as_bytes());
                msg.truncate(1000);
                let _ = send(io, &mut session, &msg, &mut cipher).await;
                Err(TransferError::Io(e))
            }
            Err(e) => Err(e),
        }
    }

    async fn respond<S>(
        &self,
        io: &mut S,
        session: &mut NoiseSession,
        request: &[u8],
        cipher: &mut Vec<u8>,
    ) -> Result<(), TransferError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        if request[0] != MSG_REQUEST || request.len() < 5 {
            return Err(proto("pedido inválido"));
        }
        let count = u32::from_be_bytes(request[1..5].try_into().unwrap()) as usize;
        if count > MAX_IDS || request.len() != 5 + count * 4 {
            return Err(proto("pedido inválido"));
        }
        let mut roots = Vec::with_capacity(count);
        for c in request[5..].chunks_exact(4) {
            let id = u32::from_be_bytes(c.try_into().unwrap()) as usize;
            roots.push(
                self.paths
                    .get(id)
                    .cloned()
                    .ok_or_else(|| proto("id desconhecido"))?,
            );
        }
        let items = tokio::task::spawn_blocking(move || {
            let mut items = Vec::new();
            for r in &roots {
                collect(r, &mut items)?;
            }
            Ok::<_, std::io::Error>(items)
        })
        .await
        .map_err(|e| TransferError::Io(e.to_string()))??;

        let total: u64 = items.iter().map(|i| i.size).sum();
        let mut msg = vec![MSG_BEGIN];
        msg.extend_from_slice(&total.to_be_bytes());
        send(io, session, &msg, cipher).await?;

        let mut buf = vec![0u8; CHUNK_SIZE + 1];
        for item in &items {
            let rel = item.rel.as_bytes();
            let rel_len = u16::try_from(rel.len()).map_err(|_| proto("caminho longo demais"))?;
            let mut msg = vec![MSG_ENTRY, u8::from(item.is_dir)];
            msg.extend_from_slice(&item.size.to_be_bytes());
            msg.extend_from_slice(&rel_len.to_be_bytes());
            msg.extend_from_slice(rel);
            send(io, session, &msg, cipher).await?;
            if item.is_dir {
                continue;
            }
            let mut file = File::open(&item.path).await?;
            let mut hasher = Blake2s256::new();
            buf[0] = MSG_CHUNK;
            loop {
                let n = file.read(&mut buf[1..]).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[1..=n]);
                send(io, session, &buf[..=n], cipher).await?;
            }
            let mut msg = vec![MSG_FILE_END];
            msg.extend_from_slice(&hasher.finalize());
            send(io, session, &msg, cipher).await?;
        }
        send(io, session, &[MSG_DONE], cipher).await
    }
}

/// Baixa ids de uma oferta para um diretório de destino.
#[derive(Debug)]
pub struct TransferClient {
    me: StaticKeypair,
    peer: PairedPeer,
}

impl TransferClient {
    pub fn new(me: StaticKeypair, peer: PairedPeer) -> Self {
        Self { me, peer }
    }

    /// Abre a conexão TCP (com `TCP_NODELAY`) até o servidor de transferência do peer.
    pub async fn connect(addr: SocketAddr) -> std::io::Result<TcpStream> {
        let s = TcpStream::connect(addr).await?;
        s.set_nodelay(true)?;
        Ok(s)
    }

    /// Baixa `ids` para `dest`, devolvendo os caminhos finais dos itens de primeiro nível.
    /// `progress` recebe no máximo 10 eventos por segundo mais o evento final (`done`).
    /// Se `cancel` virar `true`, aborta e não deixa nada parcial em `dest`.
    pub async fn download<S>(
        &self,
        io: &mut S,
        ids: &[u32],
        dest: &Path,
        transfer_id: u32,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TransferProgress),
    ) -> Result<Vec<PathBuf>, TransferError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let staging = dest.join(format!(".relay-transfer-{transfer_id}"));
        let mut state = Progress::new(transfer_id, progress);
        let res = self
            .download_inner(io, ids, dest, &staging, cancel, &mut state)
            .await;
        if res.is_err() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
        }
        let _ = tokio::fs::remove_dir(&staging).await;
        match &res {
            Ok(_) => state.finish(None),
            Err(e) => state.finish(Some(e.to_string())),
        }
        res
    }

    async fn download_inner<S>(
        &self,
        io: &mut S,
        ids: &[u32],
        dest: &Path,
        staging: &Path,
        cancel: &AtomicBool,
        state: &mut Progress<'_>,
    ) -> Result<Vec<PathBuf>, TransferError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        if ids.len() > MAX_IDS {
            return Err(proto("ids demais"));
        }
        let mut session = noise_dial(io, &self.me, &self.peer).await?;
        let mut cipher = Vec::new();
        let mut msg = vec![MSG_REQUEST];
        msg.extend_from_slice(&(ids.len() as u32).to_be_bytes());
        for id in ids {
            msg.extend_from_slice(&id.to_be_bytes());
        }
        send(io, &mut session, &msg, &mut cipher).await?;

        tokio::fs::create_dir_all(staging).await?;
        let mut plain = Vec::new();
        recv(io, &mut session, &mut plain).await?;
        check_error(&plain)?;
        if plain[0] != MSG_BEGIN || plain.len() != 9 {
            return Err(proto("esperava Begin"));
        }
        state.total = u64::from_be_bytes(plain[1..9].try_into().unwrap());
        state.emit(true);

        let mut done_items: Vec<PathBuf> = Vec::new();
        let mut current_top: Option<String> = None;
        let mut file: Option<(File, Blake2s256, u64, u64)> = None; // (arquivo, hash, lido, esperado)
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(TransferError::Cancelled);
            }
            recv(io, &mut session, &mut plain).await?;
            check_error(&plain)?;
            match plain[0] {
                MSG_ENTRY => {
                    if file.is_some() {
                        return Err(proto("Entry antes do FileEnd"));
                    }
                    let (rel, size, is_dir) = parse_entry(&plain)?;
                    let top = rel[0].clone();
                    if current_top.as_deref() != Some(&top) {
                        if let Some(prev) = current_top.take() {
                            done_items.push(commit(staging, &prev, dest).await?);
                        }
                        current_top = Some(top);
                    }
                    let mut path = staging.to_path_buf();
                    path.extend(&rel);
                    state.file_name = rel.last().cloned().unwrap_or_default();
                    if is_dir {
                        tokio::fs::create_dir_all(&path).await?;
                    } else {
                        if let Some(parent) = path.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        file = Some((File::create(&path).await?, Blake2s256::new(), 0, size));
                    }
                }
                MSG_CHUNK => {
                    let (f, hasher, read, _) = file.as_mut().ok_or_else(|| proto("Chunk solto"))?;
                    let data = &plain[1..];
                    f.write_all(data).await?;
                    hasher.update(data);
                    *read += data.len() as u64;
                    state.bytes += data.len() as u64;
                    state.emit(false);
                }
                MSG_FILE_END => {
                    let (mut f, hasher, read, expected) =
                        file.take().ok_or_else(|| proto("FileEnd solto"))?;
                    f.flush().await?;
                    if plain.len() != 33 || read != expected {
                        return Err(proto("arquivo incompleto"));
                    }
                    if hasher.finalize().as_slice() != &plain[1..] {
                        return Err(proto("integridade: hash BLAKE2s difere"));
                    }
                }
                MSG_DONE => break,
                _ => return Err(proto("mensagem desconhecida")),
            }
        }
        if file.is_some() {
            return Err(proto("Done no meio de um arquivo"));
        }
        if let Some(prev) = current_top.take() {
            done_items.push(commit(staging, &prev, dest).await?);
        }
        Ok(done_items)
    }
}

fn check_error(plain: &[u8]) -> Result<(), TransferError> {
    if plain[0] == MSG_ERROR {
        return Err(TransferError::Protocol(format!(
            "servidor: {}",
            String::from_utf8_lossy(&plain[1..])
        )));
    }
    Ok(())
}

fn parse_entry(plain: &[u8]) -> Result<(Vec<String>, u64, bool), TransferError> {
    if plain.len() < 12 {
        return Err(proto("Entry curta"));
    }
    let is_dir = plain[1] != 0;
    let size = u64::from_be_bytes(plain[2..10].try_into().unwrap());
    let len = u16::from_be_bytes(plain[10..12].try_into().unwrap()) as usize;
    let raw = plain
        .get(12..12 + len)
        .ok_or_else(|| proto("Entry curta"))?;
    let raw = std::str::from_utf8(raw).map_err(|_| proto("caminho não é UTF-8"))?;
    let rel = sanitize_path(raw);
    if rel.is_empty() {
        return Err(proto("caminho vazio"));
    }
    Ok((rel, size, is_dir))
}

/// Sanitiza um componente de nome vindo do peer: sem separadores, `:` nem NUL, e nunca `.`/`..`.
pub fn sanitize_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    match s.as_str() {
        "" | "." | ".." => "_".to_string(),
        _ => s,
    }
}

/// Quebra um caminho do peer em componentes seguros. Raiz, `.` e componentes vazios somem
/// (caminho absoluto vira relativo); `..` vira `_`, então o resultado nunca sai do destino.
pub fn sanitize_path(raw: &str) -> Vec<String> {
    raw.split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != ".")
        .map(sanitize_component)
        .collect()
}

/// Move `staging/top` para `dest`, sem sobrescrever nada existente (`nome (2)`, ...).
async fn commit(staging: &Path, top: &str, dest: &Path) -> Result<PathBuf, TransferError> {
    let from = staging.join(top);
    let (stem, ext) = match top.rfind('.') {
        Some(i) if i > 0 && from.is_file() => (&top[..i], &top[i..]),
        _ => (top, ""),
    };
    let mut target = dest.join(top);
    let mut n = 2;
    while tokio::fs::symlink_metadata(&target).await.is_ok() {
        target = dest.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    tokio::fs::rename(&from, &target).await?;
    Ok(target)
}

/// Estado do progresso com limitação de 10 Hz.
struct Progress<'a> {
    transfer_id: u32,
    file_name: String,
    bytes: u64,
    total: u64,
    last: Option<Instant>,
    sink: &'a mut dyn FnMut(TransferProgress),
}

impl<'a> Progress<'a> {
    fn new(transfer_id: u32, sink: &'a mut dyn FnMut(TransferProgress)) -> Self {
        Self {
            transfer_id,
            file_name: String::new(),
            bytes: 0,
            total: 0,
            last: None,
            sink,
        }
    }

    fn event(&self, done: bool, error: Option<String>) -> TransferProgress {
        TransferProgress {
            transfer_id: self.transfer_id,
            file_name: self.file_name.clone(),
            bytes: self.bytes,
            total: self.total,
            done,
            error,
        }
    }

    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if force || self.last.is_none_or(|t| now - t >= PROGRESS_INTERVAL) {
            self.last = Some(now);
            let ev = self.event(false, None);
            (self.sink)(ev);
        }
    }

    fn finish(&mut self, error: Option<String>) {
        let ev = self.event(true, error);
        (self.sink)(ev);
    }
}

#[cfg(test)]
mod tests;
