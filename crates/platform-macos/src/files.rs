//! dono: M5
//! Helpers de arquivos: pasteboard de arraste, app em primeiro plano, revelar no Finder.

use std::path::{Path, PathBuf};

use objc2_app_kit::{NSPasteboard, NSWorkspace};
use objc2_foundation::{NSArray, NSString, NSURL};

const DRAG_PASTEBOARD: &str = "Apple CFPasteboard drag";
const FILE_URL_TYPE: &str = "public.file-url";

/// Caminhos dos itens com `public.file-url` de um pasteboard (na ordem dos itens).
pub(crate) fn file_paths_from(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pasteboard.pasteboardItems() else {
        return Vec::new();
    };
    let kind = NSString::from_str(FILE_URL_TYPE);
    items
        .iter()
        .filter_map(|item| item.stringForType(&kind))
        .filter_map(|s| NSURL::URLWithString(&s))
        .filter(|url| url.isFileURL())
        .filter_map(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
        .collect()
}

/// URLs de arquivo do pasteboard de arraste (vazio se não houver).
pub fn drag_pasteboard_files() -> Vec<PathBuf> {
    let name = NSString::from_str(DRAG_PASTEBOARD);
    file_paths_from(&NSPasteboard::pasteboardWithName(&name))
}

/// Bundle id do app em primeiro plano (`None` se não houver ou não tiver bundle).
pub fn frontmost_bundle_id() -> Option<String> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    app.bundleIdentifier().map(|s| s.to_string())
}

/// Revela os caminhos no Finder (seleciona-os na janela da pasta).
pub fn reveal_in_finder(paths: &[&Path]) {
    let urls: Vec<_> = paths
        .iter()
        .filter_map(|p| p.to_str())
        .map(|p| NSURL::fileURLWithPath(&NSString::from_str(p)))
        .collect();
    if urls.is_empty() {
        return;
    }
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmost_bundle_id_nao_entra_em_panico() {
        let _ = frontmost_bundle_id();
    }

    #[test]
    fn drag_pasteboard_files_nao_entra_em_panico() {
        let _ = drag_pasteboard_files();
    }

    #[test]
    fn reveal_vazio_e_noop() {
        reveal_in_finder(&[]);
    }
}
