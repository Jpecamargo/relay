//! relay-platform-windows: backends Windows (hooks LL, SendInput, telas, Core Audio, clipboard,
//! serviço). Cada módulo tem um item dono (`//! dono: <item>`); ver `crates/core/OWNERS.md`.
//! Só o F2 edita este arquivo.
//!
//! Todos os módulos compilam em qualquer target: o código que chama a API do Windows fica sob
//! `#[cfg(windows)]` dentro do módulo (as dependências `windows`, `wasapi` etc. só existem no
//! target Windows), e a lógica pura (mapeamentos, parsers, conversões) fica sem cfg para ser
//! testada no Mac. Validação do código Windows: check cruzado (`scripts/check-windows.sh`).

pub mod audio_convert;
pub mod audio_endpoints;
pub mod audio_stream;
pub mod clipboard;
pub mod clipboard_formats;
pub mod cursor;
pub mod displays;
pub mod dpi;
pub mod edid;
pub mod explorer;
pub mod foreground;
pub mod hooks;
pub mod hooks_map;
pub mod inject;
pub mod inject_map;
pub mod keyboard;
pub mod metrics;
pub mod mouse_accel;
pub mod net;
pub mod pipe;
pub mod power;
pub mod screen_capture;
pub mod service;
pub mod shell;
