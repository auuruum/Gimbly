//! Linux virtual camera producer using a user-configured v4l2loopback node.
//! Never open a physical video output or try to load kernel modules.

use super::FRIENDLY_NAME;
use crate::camera::{CaptureConfig, YuvRange, set_nonblocking};
use gimbly_vcam_shm::{
    Nv12View, OutputFormat, PreferredFormat,
    pixel::{convert_scaled, fill_placeholder},
};
use linuxvideo::format::{PixFormat, PixelFormat};
use linuxvideo::{CapabilityFlags, Device, VideoOutputDevice};
use std::io;
use std::os::fd::{AsFd, AsRawFd};

fn loopback() -> Result<Device, String> {
    for device in linuxvideo::list().map_err(|e| e.to_string())?.flatten() {
        if let Ok(caps) = device.capabilities()
            && caps.driver() == "v4l2 loopback"
            && caps.card() == FRIENDLY_NAME
        {
            if !caps
                .device_capabilities()
                .contains(CapabilityFlags::VIDEO_OUTPUT)
            {
                return Err(format!(
                    "{FRIENDLY_NAME} already has a producer. Close it and try again."
                ));
            }
            return Ok(device);
        }
    }
    Err(format!(
        "No writable v4l2loopback device named {FRIENDLY_NAME:?}. See the Linux setup in CONTRIBUTING.md."
    ))
}

pub fn register() -> Result<Option<String>, String> {
    loopback()?;
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
    unsafe { set_parameters(device.as_raw_fd(), &mut params) }
        .map_err(|e| format!("Could not set the virtual camera frame rate: {e}"))?;
    Ok(())
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
        let device = loopback()?
            .video_output(PixFormat::new(w, h, PixelFormat::YUYV))
            .map_err(|e| format!("Could not configure {FRIENDLY_NAME}: {e}"))?;
        let actual = device.format();
        if actual.width() != w
            || actual.height() != h
            || actual.pixel_format() != PixelFormat::YUYV
            || actual.bytes_per_line() != w * 2
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

    pub fn publish(&mut self, view: &Nv12View<'_>, range: YuvRange) -> Result<(), String> {
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

    fn write_frame(&mut self) -> Result<(), String> {
        // SAFETY: this is a nonblocking write of a complete frame with a
        // valid buffer for the duration of the syscall.
        let written = unsafe {
            libc::write(
                self.device.as_raw_fd(),
                self.buffer.as_ptr().cast(),
                self.buffer.len(),
            )
        };
        if written == self.buffer.len() as isize {
            self.published = true;
            return Ok(());
        }
        if written < 0 {
            let e = io::Error::last_os_error();
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(());
            }
            return Err(format!("Virtual camera output stopped: {e}"));
        }
        Err("Virtual camera accepted only part of a frame".into())
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
