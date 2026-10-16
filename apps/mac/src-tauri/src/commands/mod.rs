//! dono: MAC1
//! Comandos Tauri do app Mac (glossário "comandos Tauri do Mac"). Cada handler só delega a
//! `AppBackend`; os arquivos são entregues a MI1 (status, pairing, layout, config), MI2
//! (controls, system) e MI3 (audio, clipboard).

pub mod audio;
pub mod clipboard;
pub mod config;
pub mod controls;
pub mod layout;
pub mod pairing;
pub mod status;
pub mod system;
