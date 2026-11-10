//! Linux virtual camera producer for a dedicated v4l2loopback node.

use super::FRIENDLY_NAME;
use crate::camera::{CaptureConfig, YuvRange, set_nonblocking};
use gimbly_vcam_shm::{
    Nv12View, OutputFormat, PreferredFormat,
    pixel::{convert_scaled, fill_placeholder},
};
use linuxvideo::format::{PixFormat, PixelFormat};
use linuxvideo::{CapabilityFlags, Device, VideoOutputDevice};
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};

pub(crate) fn is_loopback_driver(driver: &str) -> bool {
    matches!(driver, "v4l2 loopback" | "v4l2loopback")
}

pub(super) enum NodeError {
    Missing,
    Unavailable(String),
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "Create {FRIENDLY_NAME} from the Virtual Camera button or follow the Linux setup in CONTRIBUTING.md."
            ),
            Self::Unavailable(error) => f.write_str(error),
        }
    }
}

fn candidates(sysfs: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(sysfs) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            name.to_str()?.strip_prefix("video")?.parse::<u32>().ok()?;
            let label = std::fs::read_to_string(entry.path().join("name")).ok()?;
            (label.trim_end() == FRIENDLY_NAME).then(|| Path::new("/dev").join(name))
        })
        .collect())
}

pub(super) fn loopback() -> Result<Device, NodeError> {
    let candidates = candidates(Path::new("/sys/class/video4linux")).map_err(|e| {
        NodeError::Unavailable(format!("Could not look for the virtual camera: {e}"))
    })?;
    let mut error = NodeError::Missing;
    for path in candidates {
        let device = Device::open(&path).and_then(|device| Ok((device.capabilities()?, device)));
        match device {
            Ok((caps, device))
                if is_loopback_driver(caps.driver()) && caps.card() == FRIENDLY_NAME =>
            {
                if caps
                    .device_capabilities()
                    .contains(CapabilityFlags::VIDEO_OUTPUT)
                {
                    return Ok(device);
                }
                error = NodeError::Unavailable(format!(
                    "{FRIENDLY_NAME} is busy. Close apps using it and try again."
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                error = NodeError::Unavailable(format!(
                    "Gimbly cannot write to {}. Check your video-device permissions and try again.",
                    path.display()
                ));
            }
            Err(e) => {
                error = NodeError::Unavailable(format!("Could not open {}: {e}", path.display()))
            }
        }
    }
    Err(error)
}

pub fn register() -> Result<Option<String>, String> {
    loopback().map_err(|e| e.to_string())?;
    Ok(None)
}

// v4l2_streamparm consists of a buffer type and a 200-byte union. For
// VIDEO_OUTPUT the union starts with v4l2_outputparm and its frame interval.
#[repr(C)]
struct OutputParameters {
    kind: u32,
    capability: u32,
    output_mode: u32,
    numerator: u32,
    denominator: u32,
    reserved: [u32; 46],
}

nix::ioctl_readwrite!(set_parameters, b'V', 22, OutputParameters);

fn set_rate(device: &VideoOutputDevice, fps: u32) -> Result<(), String> {
    let mut params = OutputParameters {
        kind: 2, // V4L2_BUF_TYPE_VIDEO_OUTPUT
        capability: 0,
        output_mode: 0,
        numerator: 1,
        denominator: fps,
        reserved: [0; 46],
    };
    // SAFETY: the initialized struct matches v4l2_streamparm's ABI, and
    // the pointer is valid for this ioctl on the open loopback descriptor.
    match unsafe { set_parameters(device.as_raw_fd(), &mut params) } {
        Ok(_) | Err(nix::errno::Errno::ENOTTY | nix::errno::Errno::EINVAL) => Ok(()),
        Err(e) => Err(format!("Could not set the virtual camera frame rate: {e}")),
    }
}

pub struct Output {
    device: VideoOutputDevice,
    format: PreferredFormat,
    buffer: Vec<u8>,
    published: bool,
}

impl Output {
    pub fn open(config: CaptureConfig, size: Option<(u32, u32)>) -> Result<Self, String> {
        let (w, h) = config.virtual_size(size.ok_or("Camera output size is not available")?);
        let format = PreferredFormat::validated(w, h, config.fps)
            .ok_or("Unsupported virtual camera dimensions")?;
        let device = loopback()
            .map_err(|e| e.to_string())?
            .video_output(PixFormat::new(w, h, PixelFormat::YUYV))
            .map_err(|e| format!("Could not configure {FRIENDLY_NAME}: {e}"))?;
        let actual = device.format();
        if actual.width() != w
            || actual.height() != h
            || actual.pixel_format() != PixelFormat::YUYV
            || actual.bytes_per_line() != w * 2
            || actual.size_image() != w * h * 2
        {
            return Err(format!(
                "{FRIENDLY_NAME} rejected {w}x{h} YUYV. Close any apps using the virtual camera and try again."
            ));
        }
        set_nonblocking(device.as_fd())
            .map_err(|e| format!("Could not configure virtual camera I/O: {e}"))?;
        set_rate(&device, format.fps)?;
        Ok(Self {
            device,
            format,
            buffer: vec![0; w as usize * h as usize * 2],
            published: false,
        })
    }

    pub fn publish(&mut self, view: &Nv12View<'_>, range: YuvRange) -> Result<bool, String> {
        let (w, h) = (self.format.width as usize, self.format.height as usize);
        convert_scaled(view, OutputFormat::Yuy2, &mut self.buffer, w, h);
        if range == YuvRange::Full601 {
            // The loopback advertises limited-range YUYV.
            for [y0, u, y1, v] in self.buffer.as_chunks_mut::<4>().0 {
                *y0 = (16 + (u16::from(*y0) * 219 + 127) / 255) as u8;
                *y1 = (16 + (u16::from(*y1) * 219 + 127) / 255) as u8;
                *u = (16 + (u16::from(*u) * 224 + 127) / 255) as u8;
                *v = (16 + (u16::from(*v) * 224 + 127) / 255) as u8;
            }
        }
        self.write_frame()
    }

    fn write_frame(&mut self) -> Result<bool, String> {
        match self.device.write(&self.buffer) {
            Ok(n) if n == self.buffer.len() => {
                self.published = true;
                Ok(true)
            }
            Ok(_) => Err("Virtual camera accepted only part of a frame".into()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(format!("Virtual camera output stopped: {e}")),
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if self.published {
            // Readers that stay attached can hold the last buffer after the
            // producer closes. Replace it before dropping the descriptor.
            fill_placeholder(
                OutputFormat::Yuy2,
                &mut self.buffer,
                self.format.width as usize,
                self.format.height as usize,
            );
            let _ = self.write_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_numeric_video_nodes_with_the_gimbly_label() {
        let dir = std::env::temp_dir().join(format!("gimbly-loopback-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        for (node, label) in [
            ("video3", "Gimbly Virtual Camera\n"),
            ("video4", "OBS Virtual Camera\n"),
            ("video-other", "Gimbly Virtual Camera\n"),
            ("video5", "Gimbly Virtual Camera (old)\n"),
        ] {
            std::fs::create_dir(dir.join(node)).unwrap();
            std::fs::write(dir.join(node).join("name"), label).unwrap();
        }
        let found = candidates(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(found, [PathBuf::from("/dev/video3")]);
        assert!(is_loopback_driver("v4l2 loopback"));
        assert!(is_loopback_driver("v4l2loopback"));
        assert!(!is_loopback_driver("uvcvideo"));
    }
}
