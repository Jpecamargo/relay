//! dono: M4
//! Áudio no macOS com cpal: enumeração, reprodução do stream do PC e captura do microfone.
//! Stub do F2.

use relay_core::platform::{AudioCapture, AudioFrame, AudioPlayback, PlatformError};
use relay_core::types::AudioDevice;

pub fn list_input_devices() -> Result<Vec<AudioDevice>, PlatformError> {
    Err(PlatformError::NotImplemented)
}

pub fn list_output_devices() -> Result<Vec<AudioDevice>, PlatformError> {
    Err(PlatformError::NotImplemented)
}

#[derive(Debug, Default)]
pub struct MacAudioPlayback {}

impl AudioPlayback for MacAudioPlayback {
    fn start(&mut self, _source: rtrb::Consumer<f32>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_volume(&mut self, _volume: f32) {}

    fn set_device(&mut self, _device_id: Option<&str>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn list_devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}

#[derive(Debug, Default)]
pub struct MacAudioCapture {}

impl AudioCapture for MacAudioCapture {
    fn start(&mut self, _sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_device(&mut self, _device_id: Option<&str>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}
