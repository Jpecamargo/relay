//! dono: C11b
//! Transferência de arquivos sob demanda numa conexão TCP separada (porta 47804) cifrada.
//! Stubs do F2: assinaturas.

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("não implementado")]
    NotImplemented,
    #[error("peer não pareado")]
    UnknownPeer,
    #[error("{0}")]
    Io(String),
}

/// Serve os arquivos de uma oferta por id.
#[derive(Debug, Default)]
pub struct TransferServer;

/// Baixa ids de uma oferta para um diretório de destino.
#[derive(Debug, Default)]
pub struct TransferClient;
