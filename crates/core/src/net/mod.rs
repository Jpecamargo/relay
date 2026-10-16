//! dono: C3
//! Sessão de rede: canal de controle TCP cifrado, canal rápido UDP, heartbeat, reconexão e
//! estatísticas. Stubs do F2: assinaturas; o corpo é do C3 (que pode criar submódulos, como
//! `net/fast`, sem tokio).

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("não implementado")]
    NotImplemented,
    #[error("peer não pareado")]
    UnknownPeer,
    #[error("{0}")]
    Io(String),
}

/// Lado do agente: escuta e aceita só peers pareados.
#[derive(Debug, Default)]
pub struct SessionListener;

/// Lado do Mac: disca para o agente.
#[derive(Debug, Default)]
pub struct SessionDialer;
