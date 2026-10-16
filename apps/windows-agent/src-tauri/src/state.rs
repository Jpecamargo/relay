//! dono: WA1
use std::sync::Arc;

use crate::backend::AgentBackend;

/// Estado gerenciado pelo Tauri; os comandos e a bandeja só falam com o backend.
#[derive(Clone)]
pub struct AgentState {
    pub backend: Arc<dyn AgentBackend>,
}

impl AgentState {
    pub fn new(backend: Arc<dyn AgentBackend>) -> Self {
        Self { backend }
    }
}
