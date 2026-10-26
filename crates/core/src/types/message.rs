//! dono: F2
//! Mensagens do canal confiável (bincode 2 com prefixo u32, D7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::caps::Caps;
use super::clipboard::{ClipboardFormat, FileEntry};
use super::command::{CommandResult, RemoteCommand};
use super::display::{DisplayInfo, LayoutBook, LayoutProfile};
use super::input::InputEvent;
use super::status::SystemMetrics;

/// Quem abriu a conexão: o Mac disca (`Dialer`), o agente escuta (`Listener`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Role {
    Dialer,
    Listener,
}

/// Enum do canal confiável. A ordem das variantes é parte do formato bincode: só acrescente
/// no fim (e só o F2 mexe aqui).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Message {
    Hello {
        protocol_version: u16,
        device_id: String,
        role: Role,
        app_version: String,
    },
    HelloAck,
    Ping {
        id: u32,
        #[ts(type = "number")]
        ts_us: u64,
    },
    Pong {
        id: u32,
        #[ts(type = "number")]
        ts_us: u64,
    },
    Input {
        #[ts(type = "number")]
        seq: u64,
        #[ts(type = "number")]
        ts_us: u64,
        event: InputEvent,
    },
    Displays(Vec<DisplayInfo>),
    LayoutSync(LayoutProfile),
    FocusEnter {
        display_id: String,
        entry_px: (i32, i32),
    },
    FocusReturn {
        display_id: String,
        point_px: (i32, i32),
    },
    ForegroundApp {
        exe: String,
    },
    Command {
        id: u32,
        cmd: RemoteCommand,
    },
    CommandResult {
        id: u32,
        result: CommandResult,
    },
    ClipboardOffer {
        offer_id: u32,
        formats: Vec<ClipboardFormat>,
        #[ts(type = "number")]
        total: u64,
    },
    ClipboardChunk {
        offer_id: u32,
        index: u32,
        data: Vec<u8>,
    },
    ClipboardSkipped {
        reason: String,
    },
    FileOffer {
        offer_id: u32,
        files: Vec<FileEntry>,
    },
    DropFiles {
        offer_id: u32,
        point_px: (i32, i32),
    },
    AudioControl {
        /// O som do PC no Mac saiu na 0.2.0: sempre `false`, mantido para o formato bincode
        /// continuar igual ao das 0.1.x (Mac e PC em versões diferentes até o PC se atualizar).
        pc_to_mac: bool,
        mac_to_pc: bool,
    },
    Metrics(SystemMetrics),
    TakeControl,
    Unpair,
    Bye,
    /// Mac -> agente: o Mac está na `version` e pede que o agente se atualize. Peers anteriores
    /// a esta variante não a decodificam (derruba a sessão): só se envia a quem a conhece, ver
    /// `version::UPDATE_REQUEST_MIN_PEER`.
    RequestUpdate {
        version: String,
    },
    /// Capacidades do peer, enviadas depois do `HelloAck`. Só a peer >= `SYMMETRIC_MIN_PEER`
    /// (ver `compat::v02::allowed_for_peer`).
    Caps(Caps),
    /// Livro de layouts oficial (D42). Só a peer >= `SYMMETRIC_MIN_PEER`.
    LayoutBookSync(LayoutBook),
    /// Proposta de edição do livro pelo lado que não é o host. Só a peer >= `SYMMETRIC_MIN_PEER`.
    LayoutBookEdit(LayoutBook),
}
