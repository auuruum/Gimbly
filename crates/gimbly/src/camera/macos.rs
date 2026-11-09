//! AVFoundation decodes capture into NV12. USB endpoint-zero control uses
//! IOKit through nusb and does not claim the video interface from AVFoundation.

use super::control::{self, AbsoluteZoom, Channel, Transport};
use super::frames::Frames;
use super::{CaptureConfig, DeviceInfo, VideoMode, Worker, YuvRange, ZoomRange, queue, vendor};
use dispatch2::DispatchQueue;
use gimbly_vcam_shm::{MAX_DIMENSION, MAX_PIXELS, Nv12View};
use nusb::MaybeFuture;
use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::*;
use objc2_core_media::{CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetDimensions};
use objc2_core_video::*;
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

const USB_TIMEOUT: Duration = Duration::from_millis(500);
const NV12_VIDEO: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
const NV12_FULL: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;

fn usb_id(id: &str) -> Option<(u32, u16, u16)> {
    // AVFoundation USB IDs can encode locationID, VID and PID. Require
    // all three to match IOKit before sending control to any device.
    let id = id.strip_prefix("0x")?;
    if id.len() != 16 {
        return None;
    }
    let id = u64::from_str_radix(id, 16).ok()?;
    Some(((id >> 32) as u32, (id >> 16) as u16, id as u16))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    autoreleasepool(|_| {
        let usb = nusb::list_devices()
            .wait()
            .ok()
            .map(|devices| devices.collect::<Vec<_>>())
            .unwrap_or_default();
        // SAFETY: enumerates video devices using the system media-type string.
        // This API also returns third-party cameras on older macOS releases.
        #[allow(deprecated)]
        let devices = unsafe {
            AVCaptureDevice::devicesWithMediaType(
                AVMediaTypeVideo.ok_or("AVFoundation video media type is unavailable")?,
            )
        };
        Ok(devices
            .iter()
            .filter_map(|device| {
                // SAFETY: immutable metadata on a retained device.
                let (name, id) = unsafe {
                    (
                        device.localizedName().to_string(),
                        device.uniqueID().to_string(),
                    )
                };
                if name == crate::vcam::FRIENDLY_NAME {
                    return None;
                }
                let detected = usb_id(&id).filter(|&(loc, vid, pid)| {
                    usb.iter().any(|d| {
                        (d.location_id(), d.vendor_id(), d.product_id()) == (loc, vid, pid)
                    })
                });
                Some(DeviceInfo {
                    name,
                    id,
                    usb_vendor_id: detected.map(|(_, vid, _)| vid),
                    usb_product_id: detected.map(|(_, _, pid)| pid),
                })
            })
            .collect())
    })
}

fn open(info: &DeviceInfo) -> Result<Retained<AVCaptureDevice>, String> {
    // SAFETY: the unique ID was obtained from AVFoundation.
    unsafe { AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(&info.id)) }
        .ok_or_else(|| format!("{} is no longer connected", info.name))
}

struct Mode {
    video: VideoMode,
    format: Retained<AVCaptureDeviceFormat>,
    duration: CMTime,
}

fn modes(device: &AVCaptureDevice, config: CaptureConfig) -> Vec<Mode> {
    let mut modes = Vec::new();
    // SAFETY: only immutable format metadata is queried. The frame duration
    // is either a reported bound or a duration within the reported interval.
    unsafe {
        for format in device.formats().iter() {
            let desc = format.formatDescription();
            let size = CMVideoFormatDescriptionGetDimensions(&desc);
            let (Ok(w), Ok(h)) = (u32::try_from(size.width), u32::try_from(size.height)) else {
                continue;
            };
            if PreferredSize::checked(w as usize, h as usize).is_none() {
                continue;
            }
            let code = desc.media_sub_type().to_be_bytes();
            let codec = match &code {
                b"420v" | b"420f" => "NV12".to_string(),
                b"yuvs" | b"2vuy" => "YUY2".to_string(),
                b"jpeg" | b"dmb1" => "MJPG".to_string(),
                _ => String::from_utf8_lossy(&code).into_owned(),
            };
            for rate in format.videoSupportedFrameRateRanges().iter() {
                let (min, max) = (rate.minFrameRate(), rate.maxFrameRate());
                if !min.is_finite() || !max.is_finite() || min <= 0.0 || max < min {
                    continue;
                }
                let fps = f64::from(config.fps.clamp(1, 120)).clamp(min, max);
                let duration = if fps == max {
                    rate.minFrameDuration()
                } else if fps == min {
                    rate.maxFrameDuration()
                } else {
                    CMTime::with_seconds(fps.recip(), 60_000)
                };
                modes.push(Mode {
                    video: VideoMode {
                        codec: codec.clone(),
                        width: w,
                        height: h,
                        fps: fps as f32,
                        min_fps: min as f32,
                        max_fps: max as f32,
                        supported: true,
                    },
                    format: format.clone(),
                    duration,
                });
            }
        }
    }
    modes
}

pub fn list_modes(info: &DeviceInfo) -> Result<Vec<VideoMode>, String> {
    autoreleasepool(|_| {
        let device = open(info)?;
        Ok(modes(&device, CaptureConfig::default())
            .into_iter()
            .map(|m| m.video)
            .collect())
    })
}

struct Usb(nusb::Device);

impl Usb {
    fn open(info: &DeviceInfo) -> Result<Self, String> {
        let (loc, vid, pid) =
            usb_id(&info.id).ok_or("This camera has no matching USB video device")?;
        let device = nusb::list_devices()
            .wait()
            .map_err(|e| e.to_string())?
            .find(|d| (d.location_id(), d.vendor_id(), d.product_id()) == (loc, vid, pid))
            .ok_or("USB video device is no longer connected")?;
        device
            .open()
            .wait()
            .map(Self)
            .map_err(|e| format!("Could not open USB camera controls: {e}"))
    }

    fn read(
        &self,
        index: u16,
        selector: u32,
        request: u8,
        block: &mut [u8],
    ) -> Result<usize, String> {
        let data = self
            .0
            .control_in(
                ControlIn {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value: (selector as u16) << 8,
                    index,
                    length: block.len() as u16,
                },
                USB_TIMEOUT,
            )
            .wait()
            .map_err(|e| format!("UVC read failed: {e}"))?;
        let target = block
            .get_mut(..data.len())
            .ok_or("USB returned an oversized control block")?;
        target.copy_from_slice(&data);
        Ok(data.len())
    }

    fn write(&self, index: u16, selector: u32, data: &[u8]) -> Result<(), String> {
        self.0
            .control_out(
                ControlOut {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request: 1,
                    value: (selector as u16) << 8,
                    index,
                    data,
                },
                USB_TIMEOUT,
            )
            .wait()
            .map_err(|e| format!("UVC write failed: {e}"))
    }
}

struct Xu {
    usb: Usb,
    index: u16,
}

impl Xu {
    fn open(info: &DeviceInfo) -> Result<Channel<Self>, String> {
        let usb = Usb::open(info)?;
        let config = usb.0.active_configuration().map_err(|e| e.to_string())?;
        let (interface, unit) = control::extension_unit(config.as_bytes())
            .ok_or("UVC vendor extension unit not found")?;
        Channel::open(Self {
            usb,
            index: u16::from_be_bytes([unit, interface]),
        })
    }

    fn check_length(&self, selector: u32) -> Result<(), String> {
        let mut bytes = [0; 2];
        if self.usb.read(self.index, selector, 0x85, &mut bytes)? != bytes.len()
            || usize::from(u16::from_le_bytes(bytes)) != vendor::MESSAGE_LEN
        {
            return Err(format!("Unexpected UVC block size for selector {selector}"));
        }
        Ok(())
    }
}

impl Transport for Xu {
    fn read(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<usize, String> {
        self.check_length(selector)?;
        self.usb.read(self.index, selector, 0x81, block)
    }

    fn write(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<(), String> {
        self.check_length(selector)?;
        self.usb.write(self.index, selector, block)
    }
}

struct Zoom {
    usb: Usb,
    index: u16,
    range: ZoomRange,
}

impl Zoom {
    const SELECTOR: u32 = 0x0b;

    fn open(info: &DeviceInfo) -> Option<Self> {
        let usb = Usb::open(info).ok()?;
        let config = usb.0.active_configuration().ok()?;
        let (interface, terminal) = control::zoom_terminal(config.as_bytes())?;
        let index = u16::from_be_bytes([terminal, interface]);
        let min = Self::read(&usb, index, 0x82)?;
        let max = Self::read(&usb, index, 0x83)?;
        let step = Self::read(&usb, index, 0x84).unwrap_or(1).max(1);
        let default = Self::read(&usb, index, 0x87).unwrap_or(min);
        (min < max).then_some(Self {
            usb,
            index,
            range: ZoomRange {
                min,
                max,
                step,
                default,
            },
        })
    }

    fn read(usb: &Usb, index: u16, request: u8) -> Option<i32> {
        let mut bytes = [0; 2];
        (usb.read(index, Self::SELECTOR, request, &mut bytes).ok()? == 2)
            .then(|| i32::from(u16::from_le_bytes(bytes)))
    }
}

impl AbsoluteZoom for Zoom {
    fn get(&self) -> Option<i32> {
        Self::read(&self.usb, self.index, 0x81)
    }

    fn set(&mut self, value: i32) -> Result<i32, String> {
        let value = self.range.snap(value);
        if self.get() != Some(value) {
            self.usb
                .write(self.index, Self::SELECTOR, &(value as u16).to_le_bytes())?;
        }
        Ok(self.get().unwrap_or(value))
    }
}

pub fn probe_controls(device: &DeviceInfo, script: Vec<String>) -> Result<Vec<String>, String> {
    if device.is_obsbot() && device.usb_product_id == Some(control::TINY_3_LITE) {
        let mut xu = Xu::open(device)?;
        let mut lines = vec![format!(
            "vendor zoom: {:?}, sleeping: {:?}, presets: {:?}",
            xu.read_zoom(),
            xu.read_sleep(),
            xu.presets()
        )];
        lines.extend(control::probe(&mut xu, script)?);
        return Ok(lines);
    }
    if !script.is_empty() {
        return Err("Vendor controls are unsupported for this device".into());
    }
    Ok(match Zoom::open(device) {
        Some(zoom) => vec![format!("zoom: {:?}, value {:?}", zoom.range, zoom.get())],
        None => vec!["No standard zoom control reported".into()],
    })
}

fn authorize(worker: &Worker) -> Result<(), String> {
    // SAFETY: the completion block is retained by AVFoundation. It captures
    // only a Send channel and may run on any system dispatch queue.
    unsafe {
        let video = AVMediaTypeVideo.ok_or("AVFoundation video media type is unavailable")?;
        match AVCaptureDevice::authorizationStatusForMediaType(video) {
            AVAuthorizationStatus::Authorized => return Ok(()),
            AVAuthorizationStatus::NotDetermined => {}
            _ => return Err("Camera access is disabled. Allow Gimbly in System Settings → Privacy & Security → Camera.".into()),
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let completed = block2::RcBlock::new(move |allowed: objc2::runtime::Bool| {
            let _ = tx.try_send(allowed.as_bool());
        });
        AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &completed);
        while !worker.should_stop() {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(true) => return Ok(()),
                Ok(false) => return Err("Camera access was denied".into()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("The camera permission request was interrupted".into()),
            }
        }
        Ok(())
    }
}

struct PreferredSize(usize, usize);

impl PreferredSize {
    fn checked(w: usize, h: usize) -> Option<Self> {
        (w >= 2
            && h >= 2
            && w <= MAX_DIMENSION
            && h <= MAX_DIMENSION
            && w.is_multiple_of(2)
            && h.is_multiple_of(2)
            && w * h <= MAX_PIXELS)
            .then_some(Self(w, h))
    }
}

struct Frame {
    bytes: Vec<u8>,
    size: PreferredSize,
    range: YuvRange,
    epoch: u64,
}

struct DelegateData {
    frames: SyncSender<Frame>,
    recycled: mpsc::Receiver<Vec<u8>>,
    shared: Arc<super::Shared>,
}

define_class!(
    // SAFETY: the delegate is retained until its serial callback queue is
    // drained. The only non-Sync field is used on that queue.
    #[unsafe(super(NSObject))]
    #[ivars = DelegateData]
    struct VideoDelegate;

    unsafe impl NSObjectProtocol for VideoDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for VideoDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn receive_frame(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let data = self.ivars();
            let Ok(mut bytes) = data.recycled.try_recv() else {
                return;
            };
            let epoch = data.shared.status().video_epoch;
            if let Some((size, range)) = copy_frame(sample, &mut bytes) {
                let _ = data.frames.try_send(Frame {
                    bytes,
                    size,
                    range,
                    epoch,
                });
            } else {
                let _ = data.frames.try_send(Frame {
                    bytes,
                    size: PreferredSize(0, 0),
                    range: YuvRange::Limited,
                    epoch,
                });
            }
        }
    }
);

fn copy_frame(sample: &CMSampleBuffer, bytes: &mut Vec<u8>) -> Option<(PreferredSize, YuvRange)> {
    // SAFETY: the callback holds a live sample. The plane pointers are used
    // only under a read-only pixel buffer lock and copied to owned memory.
    let buffer = unsafe { sample.image_buffer() }?;
    let size = PreferredSize::checked(
        CVPixelBufferGetWidth(&buffer),
        CVPixelBufferGetHeight(&buffer),
    )?;
    let PreferredSize(w, h) = size;
    let range = match CVPixelBufferGetPixelFormatType(&buffer) {
        NV12_VIDEO => YuvRange::Limited,
        NV12_FULL => YuvRange::Full,
        _ => return None,
    };
    if CVPixelBufferGetPlaneCount(&buffer) != 2 {
        return None;
    }
    let lock = CVPixelBufferLockFlags::ReadOnly;
    // SAFETY: matching lock/unlock for a retained CVPixelBuffer.
    if unsafe { CVPixelBufferLockBaseAddress(&buffer, lock) } != 0 {
        return None;
    }
    let result = (|| {
        bytes.resize(w * h * 3 / 2, 0);
        let (y, uv) = bytes.split_at_mut(w * h);
        for (plane, dst) in [y, uv].into_iter().enumerate() {
            let ptr = CVPixelBufferGetBaseAddressOfPlane(&buffer, plane).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(&buffer, plane);
            let height = CVPixelBufferGetHeightOfPlane(&buffer, plane);
            if ptr.is_null() || stride < w || height < dst.len() / w || stride > MAX_DIMENSION * 4 {
                return None;
            }
            for (row, out) in dst.chunks_exact_mut(w).enumerate() {
                // SAFETY: row lies within the locked plane, the stride
                // covers w bytes, and out is owned and does not overlap.
                unsafe {
                    std::ptr::copy_nonoverlapping(ptr.add(row * stride), out.as_mut_ptr(), w)
                };
            }
        }
        Some((size, range))
    })();
    // SAFETY: this releases the read-only lock acquired above.
    unsafe { CVPixelBufferUnlockBaseAddress(&buffer, lock) };
    result
}

pub fn run_capture(
    info: &DeviceInfo,
    config: CaptureConfig,
    worker: &Worker,
) -> Result<(), String> {
    autoreleasepool(|_| capture(info, config, worker))
}

fn capture(info: &DeviceInfo, config: CaptureConfig, worker: &Worker) -> Result<(), String> {
    authorize(worker)?;
    if worker.should_stop() {
        return Ok(());
    }
    let device = open(info)?;
    let modes = modes(&device, config);
    let mode = modes
        .iter()
        .max_by_key(|m| super::mode_score(&m.video, config))
        .ok_or("The camera offers no supported video format")?;
    let (w, h, fps) = (mode.video.width, mode.video.height, mode.video.fps);
    let (tx, rx) = mpsc::sync_channel(1);
    let (recycle, recycled) = mpsc::sync_channel(1);
    let _ = recycle.try_send(Vec::new());
    let queue = DispatchQueue::new("gimbly.capture", None);
    let delegate: Retained<VideoDelegate> = {
        let delegate = VideoDelegate::alloc().set_ivars(DelegateData {
            frames: tx,
            recycled,
            shared: worker.shared.clone(),
        });
        // SAFETY: initializes the allocated delegate's NSObject superclass.
        unsafe { msg_send![super(delegate), init] }
    };
    // SAFETY: session configuration and start/stop all run on this capture
    // thread. The delegate and queue outlive all callbacks.
    let (session, output) = unsafe {
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
            .map_err(|e| e.to_string())?;
        let output = AVCaptureVideoDataOutput::new();
        let session = AVCaptureSession::new();
        if !session.canAddInput(&input) {
            return Err("AVFoundation rejected the camera input".into());
        }
        session.beginConfiguration();
        let configured = (|| -> Result<(), String> {
            session.addInput(&input);
            if !session.canAddOutput(&output) {
                return Err("AVFoundation rejected the camera output".into());
            }
            session.addOutput(&output);
            device.lockForConfiguration().map_err(|e| e.to_string())?;
            device.setActiveFormat(&mode.format);
            if mode.duration.compare(device.activeVideoMaxFrameDuration()) > 0 {
                device.setActiveVideoMaxFrameDuration(mode.duration);
                device.setActiveVideoMinFrameDuration(mode.duration);
            } else {
                device.setActiveVideoMinFrameDuration(mode.duration);
                device.setActiveVideoMaxFrameDuration(mode.duration);
            }
            device.unlockForConfiguration();
            Ok(())
        })();
        session.commitConfiguration();
        configured?;
        let formats = output.availableVideoCVPixelFormatTypes();
        let pixel_format = [NV12_VIDEO, NV12_FULL]
            .into_iter()
            .find(|format| formats.iter().any(|value| value.as_u32() == *format))
            .ok_or("AVFoundation did not offer an NV12 video output")?;
        let settings = NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[&*NSString::from_str("PixelFormatType")],
            &[NSNumber::new_u32(pixel_format).into()],
        );
        output.setVideoSettings(Some(&settings));
        output.setAlwaysDiscardsLateVideoFrames(true);
        output.setSampleBufferDelegate_queue(
            Some(ProtocolObject::from_ref(&*delegate)),
            Some(&queue),
        );
        session.startRunning();
        (session, output)
    };
    let tiny = info.is_obsbot() && info.usb_product_id == Some(control::TINY_3_LITE);
    {
        let mut status = worker.shared.status();
        status.modes = modes.into_iter().map(|m| m.video).collect();
        status.format = Some(format!("NV12 {w}x{h} @ {fps:.0} fps"));
        status.output = Some(config.output_size(w, h));
        status.vendor_control_untested = info.is_obsbot();
    }
    let (commands, control_rx) = queue::channel();
    let result = std::thread::scope(|scope| {
        let shared = &*worker.shared;
        let repaint = &*worker.repaint;
        scope.spawn(move || {
            if tiny {
                match Xu::open(info) {
                    Ok(mut xu) => {
                        let refresh = xu.initialize(shared);
                        repaint();
                        control::control_loop(xu, &control_rx, shared, repaint, refresh);
                        return;
                    }
                    Err(e) => {
                        shared.status().error = Some(format!("Vendor controls unavailable: {e}"))
                    }
                }
            }
            if let Some(zoom) = Zoom::open(info) {
                let value = zoom.get();
                {
                    let mut status = shared.status();
                    status.zoom_range = Some(zoom.range);
                    status.zoom = value;
                }
                repaint();
                control::zoom_loop(zoom, &control_rx, shared, repaint);
            }
        });
        let mut frames = Frames::new(worker, config, tiny);
        let mut last = Instant::now();
        while !worker.should_stop() {
            frames.commands(|command| super::forward_control(worker, Some(&commands), command));
            match rx.recv_timeout(Duration::from_millis(40)) {
                Ok(frame) => {
                    let PreferredSize(w, h) = frame.size;
                    if let Some(view) = Nv12View::packed(&frame.bytes, w, h) {
                        worker.shared.status().output =
                            Some(config.output_size(w as u32, h as u32));
                        frames.publish(frame.epoch, &view, frame.range);
                        frames.received();
                        last = Instant::now();
                    }
                    let _ = recycle.try_send(frame.bytes);
                }
                Err(RecvTimeoutError::Timeout) => frames.idle(),
                Err(_) => return Err("The AVFoundation capture stream ended".into()),
            }
            if last.elapsed() > Duration::from_secs(8) && !worker.shared.status().sleeping {
                return Err("AVFoundation stopped delivering video. Check camera permissions and close other video apps.".into());
            }
        }
        drop(commands);
        Ok(())
    });
    // SAFETY: detach before releasing the delegate, then drain its serial
    // callback queue to finish any already dispatched calls.
    unsafe {
        output.setSampleBufferDelegate_queue(None, None);
        session.stopRunning();
    }
    queue.exec_sync(|| {});
    result
}

#[cfg(test)]
mod tests {
    use super::usb_id;

    #[test]
    fn parses_only_the_complete_usb_identifier() {
        assert_eq!(
            usb_id("0x142000003564ff04"),
            Some((0x14200000, 0x3564, 0xff04))
        );
        assert_eq!(usb_id("0x3564ff04"), None);
        assert_eq!(usb_id("BuiltInCamera-0"), None);
    }
}
