//! dono: W4b
//! Streams de áudio: loopback WASAPI de endpoint, loopback por processo e reprodução.
//! Stubs do F2.

use relay_core::platform::{AudioCapture, AudioFrame, AudioPlayback, PlatformError};
use relay_core::types::AudioDevice;

#[derive(Debug, Default)]
pub struct LoopbackCapture {}

impl AudioCapture for LoopbackCapture {
    fn start(&mut self, _sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_device(&mut self, _device_id: Option<&str>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }
}

/// Loopback por processo (Windows 10 2004+); em build anterior devolve `Unsupported`.
#[derive(Debug, Default)]
pub struct ProcessLoopbackCapture {}

impl AudioCapture for ProcessLoopbackCapture {
    fn start(&mut self, _sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError> {
        Err(PlatformError::NotImplemented)
    }

    fn stop(&mut self) {}

    fn set_device(&mut self, _device_id: Option<&str>) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

/// Reprodução num endpoint (CABLE Input) a partir de um ring.
#[derive(Debug, Default)]
pub struct EndpointRender {}

impl AudioPlayback for EndpointRender {
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
