//! V4L2 capture and UVC controls. The capture and control handles use
//! separate file descriptors so slow USB transactions never hold up frames.

use super::control::{self, AbsoluteZoom, Channel, Processing, Transport};
use super::frames::Frames;
use super::preview::YuvRange;
use super::{CaptureConfig, DeviceInfo, VideoMode, Worker, ZoomRange, queue, vendor};
use gimbly_vcam_shm::{MAX_DIMENSION, MAX_PIXELS, Nv12View};
use linuxvideo::controls::Cid;
use linuxvideo::format::{FrameIntervals, FrameSizes, PixFormat, PixelFormat};
use linuxvideo::{BufType, CapabilityFlags, Device, Fract};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const NV12: PixelFormat = PixelFormat::from_fourcc(*b"NV12");
const MJPG: PixelFormat = PixelFormat::from_fourcc(*b"MJPG");

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    let devices = linuxvideo::list().map_err(|e| format!("Could not list cameras: {e}"))?;
    let mut list = Vec::new();
    let mut denied = false;
    for device in devices {
        let dev = match device {
            Ok(dev) => dev,
            Err(e) => {
                denied |= e.kind() == io::ErrorKind::PermissionDenied;
                continue;
            }
        };
        let (Ok(caps), Ok(path)) = (dev.capabilities(), dev.path()) else {
            continue;
        };
        if !caps
            .device_capabilities()
            .contains(CapabilityFlags::VIDEO_CAPTURE | CapabilityFlags::STREAMING)
            || crate::vcam::is_loopback_driver(caps.driver())
            || caps.card() == crate::vcam::FRIENDLY_NAME
        {
            continue;
        }
        let usb = usb_parent(&path);
        let id = |attribute| {
            let text = std::fs::read_to_string(usb.as_ref()?.join(attribute)).ok()?;
            u16::from_str_radix(text.trim(), 16).ok()
        };
        list.push(DeviceInfo {
            name: caps.card().into(),
            id: path.to_string_lossy().into_owned(),
            usb_vendor_id: id("idVendor"),
            usb_product_id: id("idProduct"),
        });
    }
    if list.is_empty() && denied {
        return Err(
            "Access to the video device was denied. Check your session's camera permissions."
                .into(),
        );
    }
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

pub(super) fn usb_parent(path: &Path) -> Option<PathBuf> {
    let sys = Path::new("/sys/class/video4linux")
        .join(path.file_name()?)
        .join("device");
    let sys = sys.canonicalize().ok()?;
    sys.ancestors()
        .find(|p| p.join("idVendor").is_file())
        .map(Path::to_owned)
}

struct Mode {
    video: VideoMode,
    pixel: PixelFormat,
    interval: Fract,
}

fn dimensions_valid(w: u32, h: u32) -> bool {
    let (w, h) = (w as usize, h as usize);
    w >= 2
        && h >= 2
        && w <= MAX_DIMENSION
        && h <= MAX_DIMENSION
        && w.is_multiple_of(2)
        && h.is_multiple_of(2)
        && w * h <= MAX_PIXELS
}

fn modes(dev: &Device, config: CaptureConfig) -> Vec<Mode> {
    let mut modes = Vec::new();
    for format in dev.formats(BufType::VIDEO_CAPTURE).take(64).flatten() {
        let pixel = format.pixel_format();
        let supported = matches!(pixel, NV12 | MJPG | PixelFormat::YUYV);
        let codec = if pixel == PixelFormat::YUYV {
            "YUY2".into()
        } else {
            pixel.to_string()
        };
        let sizes = match dev.frame_sizes(pixel) {
            Ok(FrameSizes::Discrete(sizes)) => sizes
                .into_iter()
                .take(128)
                .map(|s| (s.width(), s.height()))
                .collect(),
            Ok(sizes) => {
                let min = (sizes.min_width(), sizes.min_height());
                let max = (sizes.max_width(), sizes.max_height());
                [
                    (config.width, config.height),
                    (1920, 1080),
                    (1280, 720),
                    min,
                    max,
                ]
                .into_iter()
                .filter(|&(w, h)| min.0 <= w && w <= max.0 && min.1 <= h && h <= max.1)
                .collect()
            }
            Err(_) => match dev.format(BufType::VIDEO_CAPTURE) {
                Ok(linuxvideo::format::Format::VideoCapture(p)) => vec![(p.width(), p.height())],
                _ => Vec::new(),
            },
        };
        for (width, height) in sizes {
            if !dimensions_valid(width, height) {
                continue;
            }
            let mut add = |interval: Fract, min_fps: f32, max_fps: f32| {
                let fps = interval.as_f32().recip();
                if fps.is_finite() && fps > 0.0 {
                    modes.push(Mode {
                        video: VideoMode {
                            codec: codec.clone(),
                            width,
                            height,
                            fps,
                            min_fps,
                            max_fps,
                            supported,
                        },
                        pixel,
                        interval,
                    });
                }
            };
            let requested = config.fps.clamp(1, 120);
            match dev.frame_intervals(pixel, width, height) {
                Ok(FrameIntervals::Discrete(rates)) => {
                    for rate in rates.into_iter().take(128) {
                        let interval = *rate.fract();
                        let fps = interval.as_f32().recip();
                        add(interval, fps, fps);
                    }
                }
                Ok(range) => {
                    let (fast, slow) = (range.min(), range.max());
                    let (min, max) = (slow.as_f32().recip(), fast.as_f32().recip());
                    let interval = if requested as f32 > max {
                        fast
                    } else if (requested as f32) < min {
                        slow
                    } else {
                        Fract::new(1, requested)
                    };
                    add(interval, min, max);
                }
                Err(_) => add(Fract::new(1, requested), requested as f32, requested as f32),
            }
        }
    }
    modes
}

fn open(device: &DeviceInfo) -> Result<Device, String> {
    Device::open(&device.id)
        .map_err(|e| format!("Could not open {} ({}): {e}", device.name, device.id))
}

pub fn list_modes(device: &DeviceInfo) -> Result<Vec<VideoMode>, String> {
    Ok(modes(&open(device)?, CaptureConfig::default())
        .into_iter()
        .map(|mode| mode.video)
        .collect())
}

#[repr(C)]
struct XuQuery {
    unit: u8,
    selector: u8,
    query: u8,
    size: u16,
    data: *mut u8,
}

nix::ioctl_readwrite!(xu_query, b'u', 0x21, XuQuery);

struct Xu {
    device: Device,
    unit: u8,
}

impl Xu {
    fn open(device: &DeviceInfo) -> Result<Channel<Self>, String> {
        let descriptors = usb_parent(Path::new(&device.id))
            .and_then(|usb| std::fs::read(usb.join("descriptors")).ok())
            .ok_or("Could not read the camera's USB descriptors")?;
        let (_, unit) =
            control::extension_unit(&descriptors).ok_or("UVC vendor extension unit not found")?;
        Channel::open(Self {
            device: open(device)?,
            unit,
        })
    }

    fn query(&self, selector: u32, request: u8, data: &mut [u8]) -> Result<(), String> {
        let mut query = XuQuery {
            unit: self.unit,
            selector: u8::try_from(selector).map_err(|e| e.to_string())?,
            query: request,
            size: u16::try_from(data.len()).map_err(|e| e.to_string())?,
            data: data.as_mut_ptr(),
        };
        // SAFETY: the ioctl's C structure and borrowed data match the UVC
        // ABI, and the kernel uses the buffer only during the call.
        unsafe { xu_query(self.device.as_raw_fd(), &mut query) }
            .map_err(|e| format!("UVC selector {selector} request {request:#04x}: {e}"))?;
        Ok(())
    }

    fn check_length(&self, selector: u32) -> Result<(), String> {
        let mut length = [0; 2];
        self.query(selector, 0x85, &mut length)?;
        if usize::from(u16::from_le_bytes(length)) != vendor::MESSAGE_LEN {
            return Err(format!("Unexpected UVC block size for selector {selector}"));
        }
        Ok(())
    }
}

impl Transport for Xu {
    fn read(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<usize, String> {
        self.check_length(selector)?;
        self.query(selector, 0x81, block)?;
        Ok(block.len())
    }

    fn write(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<(), String> {
        self.check_length(selector)?;
        self.query(selector, 0x01, block)
    }
    // uvcvideo maps the camera terminal's focus controls to V4L2 controls.
    fn set_auto_focus(&mut self, on: bool) -> Result<(), String> {
        self.device
            .write_control_raw(Cid::FOCUS_AUTO, on.into())
            .map_err(|e| format!("Focus failed: {e}"))
    }

    fn set_focus_position(&mut self, position: u8) -> Result<(), String> {
        self.device
            .write_control_raw(Cid::FOCUS_ABSOLUTE, position.into())
            .map_err(|e| format!("Focus failed: {e}"))
    }

    // uvcvideo maps the processing unit's controls to V4L2 controls with the
    // same values.
    fn get_processing(&mut self, control: Processing) -> Result<i32, String> {
        self.device
            .read_control_raw(processing_cid(control))
            .map_err(|e| format!("Exposure read failed: {e}"))
    }

    fn set_processing(&mut self, control: Processing, value: i32) -> Result<(), String> {
        self.device
            .write_control_raw(processing_cid(control), value)
            .map_err(|e| format!("Exposure failed: {e}"))
    }
}

fn processing_cid(control: Processing) -> Cid {
    match control {
        Processing::Backlight => Cid::BACKLIGHT_COMPENSATION,
        Processing::Gain => Cid::GAIN,
        Processing::PowerLine => Cid::POWER_LINE_FREQUENCY,
    }
}

struct Zoom {
    device: Device,
    range: ZoomRange,
}

impl Zoom {
    fn open(device: &DeviceInfo) -> Option<Self> {
        let device = open(device).ok()?;
        let desc = device
            .controls()
            .take(256)
            .filter_map(Result::ok)
            .find(|c| c.id() == Cid::ZOOM_ABSOLUTE)?;
        (desc.maximum() > desc.minimum()).then_some(Self {
            range: ZoomRange {
                min: desc.minimum(),
                max: desc.maximum(),
                step: desc.step().max(1),
                default: desc.default_value(),
            },
            device,
        })
    }
}

impl AbsoluteZoom for Zoom {
    fn get(&self) -> Option<i32> {
        self.device.read_control_raw(Cid::ZOOM_ABSOLUTE).ok()
    }

    fn set(&mut self, value: i32) -> Result<i32, String> {
        let value = self.range.snap(value);
        if self.get() != Some(value) {
            self.device
                .write_control_raw(Cid::ZOOM_ABSOLUTE, value)
                .map_err(|e| format!("Zoom failed: {e}"))?;
        }
        Ok(self.get().unwrap_or(value))
    }
}

pub fn probe_controls(device: &DeviceInfo, script: Vec<String>) -> Result<Vec<String>, String> {
    let dev = open(device)?;
    let mut lines: Vec<_> = dev
        .controls()
        .take(256)
        .filter_map(Result::ok)
        .map(|c| {
            format!(
                "{} ({:?}): {}..={} step {} default {} value {:?}",
                c.name(),
                c.id(),
                c.minimum(),
                c.maximum(),
                c.step(),
                c.default_value(),
                dev.read_control_raw(c.id())
            )
        })
        .collect();
    if device.is_obsbot() {
        let mut xu = Xu::open(device)?;
        lines.push(format!(
            "vendor zoom: {:?}, sleeping and tracking: {:?}, presets: {:?}",
            xu.read_zoom(),
            xu.read_view(),
            xu.presets()
        ));
        lines.extend(control::probe(&mut xu, script)?);
    } else if !script.is_empty() {
        return Err("Vendor controls are unsupported for this device".into());
    }
    Ok(lines)
}

pub(crate) fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    let fd = fd.as_raw_fd();
    // SAFETY: the borrowed descriptor stays valid for both fcntl calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: the same live descriptor receives its original flags plus O_NONBLOCK.
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn wait_frame(fd: i32, timeout_ms: i32) -> io::Result<bool> {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd and a bounded timeout.
    let result = unsafe { libc::poll(&mut poll, 1, timeout_ms) };
    if result < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
        return Err(io::Error::other("video device disconnected or stopped"));
    }
    Ok(poll.revents & libc::POLLIN != 0)
}

pub fn run_capture(
    device: &DeviceInfo,
    config: CaptureConfig,
    worker: &Worker,
) -> Result<(), String> {
    let dev = open(device)?;
    let modes = modes(&dev, config);
    let mode = modes
        .iter()
        .filter(|m| m.video.supported)
        .max_by_key(|m| super::mode_score(&m.video, config))
        .ok_or("The camera offers no supported video format")?;
    let capture = dev
        .video_capture(PixFormat::new(
            mode.video.width,
            mode.video.height,
            mode.pixel,
        ))
        .map_err(|e| format!("Could not configure {}: {e}", device.name))?;
    let format = capture.format();
    let (w, h, pixel, stride) = (
        format.width(),
        format.height(),
        format.pixel_format(),
        format.bytes_per_line() as usize,
    );
    if !dimensions_valid(w, h)
        || !matches!(pixel, NV12 | MJPG | PixelFormat::YUYV)
        || format.size_image() as usize > MAX_PIXELS * 4
    {
        return Err(format!("Unsupported V4L2 format {pixel} {w}x{h}"));
    }
    let fps = capture
        .set_frame_interval(mode.interval)
        .ok()
        .map(|f| f.as_f32().recip())
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(mode.video.fps);
    let mut stream = capture
        .into_stream()
        .map_err(|e| format!("Could not start {}: {e}", device.name))?;
    set_nonblocking(stream.as_fd()).map_err(|e| format!("Could not configure capture I/O: {e}"))?;
    let model = device.model();
    {
        let mut status = worker.shared.status();
        status.modes = modes.into_iter().map(|mode| mode.video).collect();
        status.format = Some(format!("{pixel} {w}x{h} @ {fps:.0} fps"));
        status.output = Some(config.output_size(w, h));
        status.vendor_control_untested = device.is_obsbot();
    }
    let (tx, rx) = queue::channel();
    std::thread::scope(|scope| {
        let shared = &*worker.shared;
        let repaint = &*worker.repaint;
        scope.spawn(move || {
            if model.is_some() {
                match Xu::open(device) {
                    Ok(mut xu) => {
                        let refresh = xu.initialize(shared);
                        repaint();
                        control::control_loop(xu, &rx, shared, repaint, refresh, &device.id);
                        return;
                    }
                    Err(e) => {
                        shared.status().error = Some(format!("Vendor controls unavailable: {e}"))
                    }
                }
            }
            if let Some(zoom) = Zoom::open(device) {
                let value = zoom.get();
                {
                    let mut status = shared.status();
                    status.zoom_range = Some(zoom.range);
                    status.zoom = value;
                }
                repaint();
                control::zoom_loop(zoom, &rx, shared, repaint);
            }
        });
        // The closure owns tx, so it disconnects before the scope joins.
        let mut frames = Frames::new(worker, config, model.is_some_and(|m| m.standby_picture));
        if config.compressed && pixel == MJPG {
            frames.set_compressed(w, h, fps);
        }
        let mut decoded = Vec::new();
        let mut packed = Vec::new();
        let mut last = Instant::now();
        let (w, h) = (w as usize, h as usize);
        while !worker.should_stop() {
            frames.commands(|command| super::forward_control(worker, Some(&tx), command));
            let epoch = worker.shared.status().video_epoch;
            if !wait_frame(stream.as_raw_fd(), 40)
                .map_err(|e| format!("Camera stream stopped: {e}"))?
            {
                frames.idle();
                if last.elapsed() > Duration::from_secs(8) && !worker.shared.status().sleeping {
                    return Err("The camera stopped delivering frames".into());
                }
                continue;
            }
            let read = stream.dequeue(|buffer| {
                if buffer.is_error() {
                    return Ok(false);
                }
                if pixel == MJPG {
                    frames.packet(&buffer);
                }
                if frames.picture_due() {
                    let view = match pixel {
                        MJPG => super::mjpeg::decode(&buffer, w, h, &mut decoded, &mut packed)
                            .map(|view| (view, YuvRange::Full601)),
                        PixelFormat::YUYV => unpack_yuyv(&buffer, stride, w, h, &mut packed)
                            .map(|view| (view, YuvRange::Limited601)),
                        _ => Nv12View::with_stride(&buffer, w, h, stride, h)
                            .map(|view| (view, YuvRange::Limited601)),
                    };
                    let Some((view, range)) = view else {
                        return Ok(false);
                    };
                    frames.publish(epoch, &view, range);
                }
                Ok(true)
            });
            let read = match read {
                Ok(read) => read,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(format!("Camera stream stopped: {error}")),
            };
            if read {
                last = Instant::now();
                frames.received();
            } else if last.elapsed() > Duration::from_secs(8) {
                return Err("The camera is delivering unreadable frames".into());
            }
        }
        drop(tx);
        Ok(())
    })
}

fn unpack_yuyv<'a>(
    data: &[u8],
    stride: usize,
    w: usize,
    h: usize,
    packed: &'a mut Vec<u8>,
) -> Option<Nv12View<'a>> {
    if stride < 2 * w || data.len() < stride.checked_mul(h)? {
        return None;
    }
    let size = w * h;
    packed.resize(size * 3 / 2, 0);
    let (y, uv) = packed.split_at_mut(size);
    for ((rows, y), uv) in data
        .chunks_exact(stride * 2)
        .zip(y.chunks_exact_mut(w * 2))
        .zip(uv.chunks_exact_mut(w))
    {
        let (top, bottom) = rows.split_at(stride);
        let (yt, yb) = y.split_at_mut(w);
        for ((top, bottom), ((yt, yb), uv)) in top
            .as_chunks::<4>()
            .0
            .iter()
            .zip(bottom.as_chunks::<4>().0)
            .zip(
                yt.as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(yb.as_chunks_mut::<2>().0)
                    .zip(uv.as_chunks_mut::<2>().0),
            )
        {
            *yt = [top[0], top[2]];
            *yb = [bottom[0], bottom[2]];
            *uv = [top[1].midpoint(bottom[1]), top[3].midpoint(bottom[3])];
        }
    }
    Nv12View::packed(packed, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_yuyv_with_padding_and_rejects_truncated_frames() {
        let data = [20, 100, 30, 130, 0, 0, 40, 120, 50, 150, 0, 0];
        let mut packed = Vec::new();
        let view = unpack_yuyv(&data, 6, 2, 2, &mut packed).unwrap();
        assert_eq!(view.y, &[20, 30, 40, 50]);
        assert_eq!(view.uv, &[110, 140]);
        assert!(unpack_yuyv(&data[..11], 6, 2, 2, &mut packed).is_none());
    }
}
