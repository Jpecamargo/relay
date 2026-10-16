//! relay-platform-macos: backends macOS. Todo o código fica atrás de cfg(target_os = "macos").
//! Cada módulo tem um item dono (`//! dono: <item>`); ver `crates/core/OWNERS.md`.
//! Só o F2 edita este arquivo.
#![cfg(target_os = "macos")]

pub mod audio;
pub mod capture;
pub mod clipboard;
pub mod cursor;
pub mod displays;
pub mod dock;
pub mod files;
pub mod inject;
pub mod login_item;
pub mod permissions;
pub mod secure_input;
pub mod system_events;
