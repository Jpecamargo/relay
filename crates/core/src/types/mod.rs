//! dono: F2
//! Tipos compartilhados do contrato: domínio, protocolo, config e payloads de UI.
//! Só o F2 edita este diretório. Todos derivam serde + ts-rs (`#[ts(export)]`).

pub mod clipboard;
pub mod command;
pub mod config;
pub mod consts;
pub mod display;
pub mod hotkey;
pub mod input;
pub mod ipc;
pub mod message;
pub mod peer;
pub mod status;

pub use clipboard::*;
pub use command::*;
pub use config::*;
pub use display::*;
pub use hotkey::*;
pub use input::*;
pub use ipc::*;
pub use message::*;
pub use peer::*;
pub use status::*;

#[cfg(test)]
mod tests;
