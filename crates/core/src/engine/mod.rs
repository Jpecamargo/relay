//! dono: C10
//! Motor de papéis, genérico sobre as traits de `platform`: Controller (captura → saída),
//! Receiver (entrada → injeção) e CommandDispatcher. Stubs do F2: assinaturas.

use crate::types::{CapturedEvent, CommandResult, InputEvent, Message, RemoteCommand};

/// Lado que captura e envia (Mac; o agente no sentido inverso).
#[derive(Debug, Default)]
pub struct Controller;

impl Controller {
    /// Roda na thread de captura, sem alocar.
    pub fn handle(&mut self, _event: CapturedEvent) {}
}

/// Lado que recebe e injeta.
#[derive(Debug, Default)]
pub struct Receiver;

impl Receiver {
    /// Aplica um evento recebido. Devolve mensagens a enviar de volta (ex.: `FocusReturn`).
    pub fn on_input(&mut self, _event: InputEvent) -> Vec<Message> {
        Vec::new()
    }
}

/// Correlaciona `Command{id}` com `CommandResult{id}`; sem resposta em 5 s gera `timeout`.
#[derive(Debug, Default)]
pub struct CommandDispatcher;

impl CommandDispatcher {
    /// Registra o comando e devolve o `id` a enviar em `Message::Command`.
    pub fn register(&mut self, _command: &RemoteCommand) -> u32 {
        0
    }

    pub fn on_result(&mut self, _result: CommandResult) {}
}
