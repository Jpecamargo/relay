//! dono: MAC1
//! Estado gerenciado pelo Tauri: o backend atrás de `AppBackend`.

use std::sync::Arc;

use crate::backend::AppBackend;

#[derive(Clone)]
pub struct AppState {
    pub backend: Arc<dyn AppBackend>,
}

impl AppState {
    pub fn new(backend: Arc<dyn AppBackend>) -> Self {
        Self { backend }
    }
}
