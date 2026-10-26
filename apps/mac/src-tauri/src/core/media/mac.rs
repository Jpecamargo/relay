//! dono: MI3
//! `MediaPlatform` do macOS: M4 (áudio), M5 (clipboard e arquivos).

use std::path::PathBuf;

use relay_core::platform::{AudioCapture, ClipboardBackend};
use relay_core::types::AudioDevices;
use relay_platform_macos::audio::{self, MacAudioCapture};
use relay_platform_macos::clipboard::MacClipboard;
use relay_platform_macos::files;

use super::MediaPlatform;

const FINDER: &str = "com.apple.finder";

#[derive(Debug, Default)]
pub struct MacMedia;

impl MediaPlatform for MacMedia {
    fn capture(&self) -> Box<dyn AudioCapture> {
        Box::new(MacAudioCapture::default())
    }

    fn clipboard(&self) -> Box<dyn ClipboardBackend> {
        Box::new(MacClipboard::new())
    }

    fn audio_devices(&self) -> AudioDevices {
        audio::list_devices().unwrap_or_else(|e| {
            tracing::warn!("não consegui listar os dispositivos de áudio: {e}");
            AudioDevices::default()
        })
    }

    fn drag_files(&self) -> Vec<PathBuf> {
        files::drag_pasteboard_files()
    }

    fn finder_frontmost(&self) -> bool {
        files::frontmost_bundle_id().is_some_and(|id| id == FINDER)
    }

    fn reveal(&self, paths: &[PathBuf]) {
        let refs: Vec<&std::path::Path> = paths.iter().map(PathBuf::as_path).collect();
        files::reveal_in_finder(&refs);
    }

    fn downloads_dir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
            .join("Relay")
    }
}
