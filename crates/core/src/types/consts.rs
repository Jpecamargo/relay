//! dono: F2
//! Constantes compartilhadas do protocolo (decisões D4, D7, D8, D9).

/// Versão do protocolo, enviada no `Hello` e no TXT `pv` do mDNS.
pub const PROTOCOL_VERSION: u16 = 1;
/// TCP: controle e pareamento.
pub const PORT_CONTROL: u16 = 47800;
/// UDP: input (cada lado escuta nela).
pub const PORT_INPUT: u16 = 47801;
/// UDP: microfone Mac→PC (o Windows escuta).
pub const PORT_AUDIO_MAC_TO_PC: u16 = 47803;
/// TCP: transferência de arquivos.
pub const PORT_TRANSFER: u16 = 47804;
/// Tipo do serviço mDNS anunciado pelo agente.
pub const MDNS_SERVICE_TYPE: &str = "_relay._tcp.local.";
/// `service` do Keychain e do Credential Manager.
pub const KEYCHAIN_SERVICE: &str = "com.joaocamargo.relay";
/// Tamanho máximo de um frame do canal confiável (cabe numa mensagem Noise).
pub const MAX_FRAME: usize = 65000;
/// Tamanho dos blocos de clipboard e de arquivos (60 KiB).
pub const CHUNK_SIZE: usize = 61440;
/// Intervalo do heartbeat (Ping) do canal confiável.
pub const HEARTBEAT_MS: u64 = 500;
