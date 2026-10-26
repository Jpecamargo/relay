//! relay-core: contrato compartilhado (tipos, protocolo, cripto, sessão, layout, transição,
//! keymap). Puro Rust. Cada módulo tem dono (`//! dono: <item>` na primeira linha); ver
//! `OWNERS.md`. Só o F2 edita este arquivo e `types/`.

pub mod audio;
pub mod clipboard;
pub mod compat;
pub mod config;
pub mod crypto;
pub mod discovery;
pub mod engine;
pub mod input_shaping;
pub mod keymap;
pub mod layout;
pub mod net;
pub mod platform;
pub mod protocol;
pub mod transfer;
pub mod transition;
pub mod types;
pub mod version;
pub mod wol;

#[cfg(test)]
mod test_alloc;
