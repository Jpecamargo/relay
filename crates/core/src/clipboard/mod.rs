//! dono: C11a
//! Sincronização de clipboard no core: oferta e blocos pelo canal confiável, limite de tamanho,
//! anti-eco e modos. Stubs do F2: assinaturas.

use crate::types::{ClipboardConfig, ClipboardContent, Message};

#[derive(Debug)]
pub struct ClipboardSync {
    _config: ClipboardConfig,
}

impl ClipboardSync {
    pub fn new(config: ClipboardConfig) -> Self {
        Self { _config: config }
    }

    /// Conteúdo local novo: devolve as mensagens a enviar (oferta + blocos, ou `ClipboardSkipped`).
    pub fn on_local_change(&mut self, _content: ClipboardContent) -> Vec<Message> {
        Vec::new()
    }

    /// Mensagem de clipboard recebida; devolve o conteúdo remontado quando completa.
    pub fn on_message(&mut self, _msg: Message) -> Option<ClipboardContent> {
        None
    }

    /// "Enviar agora": força a oferta do conteúdo atual.
    pub fn send_now(&mut self, _current: Option<ClipboardContent>) -> Vec<Message> {
        Vec::new()
    }
}
