//! Placeholder backend for platforms whose capture code is not written yet
//! (Linux: V4L2, macOS: AVFoundation). It reports the gap instead of
//! silently doing nothing.

use super::{CaptureConfig, DeviceInfo, VideoMode, Worker};

const NOT_YET: &str = "Camera capture is not implemented on this operating system yet";

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    Err(NOT_YET.into())
}

pub fn list_modes(_device: &DeviceInfo) -> Result<Vec<VideoMode>, String> {
    Err(NOT_YET.into())
}

pub fn run_capture(
    _device: &DeviceInfo,
    _config: CaptureConfig,
    _worker: &Worker,
) -> Result<(), String> {
    Err(NOT_YET.into())
}
