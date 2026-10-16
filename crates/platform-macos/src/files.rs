//! dono: M5
//! Helpers de arquivos: pasteboard de arraste, app em primeiro plano, revelar no Finder.
//! Stub do F2.

use std::path::{Path, PathBuf};

pub fn drag_pasteboard_files() -> Vec<PathBuf> {
    Vec::new()
}

pub fn frontmost_bundle_id() -> Option<String> {
    None
}

pub fn reveal_in_finder(_paths: &[&Path]) {}
