//! dono: F2
//! Conteúdo de clipboard e entradas de arquivo.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Entrada de uma lista de arquivos. `id` é o índice na lista de caminhos locais do emissor
/// (`ClipboardBackend::local_paths`), usado pelo `TransferServer` para servir sob demanda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileEntry {
    pub id: u32,
    pub name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ClipboardContent {
    Text(String),
    Html(String),
    Rtf(Vec<u8>),
    Png(Vec<u8>),
    FileList(Vec<FileEntry>),
}

/// Formato de clipboard (para `ClipboardOffer.formats` e `ClipboardConfig.formats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ClipboardFormat {
    Text,
    Html,
    Rtf,
    Png,
    FileList,
}

impl ClipboardContent {
    pub fn format(&self) -> ClipboardFormat {
        match self {
            ClipboardContent::Text(_) => ClipboardFormat::Text,
            ClipboardContent::Html(_) => ClipboardFormat::Html,
            ClipboardContent::Rtf(_) => ClipboardFormat::Rtf,
            ClipboardContent::Png(_) => ClipboardFormat::Png,
            ClipboardContent::FileList(_) => ClipboardFormat::FileList,
        }
    }
}
