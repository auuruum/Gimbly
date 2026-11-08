//! Windows camera backend built on Media Foundation.
//!
//! Capture uses an `IMFSourceReader` with advanced video processing enabled,
//! so Media Foundation decodes the camera's MJPEG stream and hands us NV12.
//! Zoom uses the standard UVC camera-terminal control through
//! `IAMCameraControl`, which the Media Foundation video capture source
//! exposes. Vendor commands (sleep, gimbal, presets) are not implemented yet.

use super::preview::{self, YuvRange};
use super::vendor;
use super::{
    CaptureConfig, Command, DeviceInfo, GimbalCommand, SessionState, VideoMode, Worker, ZoomRange,
    parse_usb_ids,
};
use gimbly_vcam_shm::{Nv12View, SharedMemory, now_ms};
use std::time::Instant;
use windows::Win32::Media::DirectShow::{
    CameraControl_Flags_Manual, CameraControl_Zoom, IAMCameraControl,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsTopologyInfo, KSIDENTIFIER_0_0, KSNODETYPE_DEV_SPECIFIC, KSP_NODE,
    KSPROPERTY_TYPE_GET, KSPROPERTY_TYPE_SET, KSPROPERTY_TYPE_TOPOLOGY,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::core::{GUID, Interface, PWSTR};

/// USB product ID of the OBSBOT Tiny 3 Lite.
const TINY_3_LITE: u16 = 0xff04;

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

/// Initialises COM (multithreaded) and Media Foundation for the current
/// thread and shuts both down on drop.
struct MfThread {
    com: bool,
}

impl MfThread {
    fn init() -> Result<Self, String> {
        // SAFETY: balanced by Drop on the same thread.
        unsafe {
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_LITE) {
                if com {
                    CoUninitialize();
                }
                return Err(format!("Media Foundation is unavailable: {e}"));
            }
            Ok(Self { com })
        }
    }
}

impl Drop for MfThread {
    fn drop(&mut self) {
        // SAFETY: matches the successful calls in `init`.
        unsafe {
            let _ = MFShutdown();
            if self.com {
                CoUninitialize();
            }
        }
    }
}

fn activate_string(activate: &IMFActivate, key: &GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: the out-pointers are valid; the string is freed with
    // CoTaskMemFree as documented for GetAllocatedString.
    unsafe {
        activate
            .GetAllocatedString(key, &mut value, &mut len)
            .ok()?;
        let text = value.to_string().ok();
        CoTaskMemFree(Some(value.0 as _));
        text
    }
}

/// Enumerates video capture devices as Media Foundation activation objects.
fn enumerate() -> Result<Vec<(DeviceInfo, IMFActivate)>, String> {
    // SAFETY: standard MF enumeration; the returned array is freed with
    // CoTaskMemFree after taking ownership of each element.
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1).map_err(|e| e.to_string())?;
        let attributes = attributes.ok_or("MFCreateAttributes returned nothing")?;
        attributes
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .map_err(|e| e.to_string())?;
        let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attributes, &mut array, &mut count)
            .map_err(|e| format!("Could not enumerate cameras: {e}"))?;
        let mut devices = Vec::new();
        if !array.is_null() {
            for slot in std::slice::from_raw_parts_mut(array, count as usize) {
                let Some(activate) = slot.take() else {
                    continue;
                };
                let name = activate_string(&activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
                    .unwrap_or_else(|| "Unknown camera".into());
                let Some(id) = activate_string(
                    &activate,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                ) else {
                    continue;
                };
                let (usb_vendor_id, usb_product_id) = parse_usb_ids(&id);
                devices.push((
                    DeviceInfo {
                        name,
                        id,
                        usb_vendor_id,
                        usb_product_id,
                    },
                    activate,
                ));
            }
            CoTaskMemFree(Some(array as _));
        }
        Ok(devices)
    }
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    // Enumerate on a short-lived MTA thread so the UI thread's COM apartment
    // does not matter.
    std::thread::spawn(|| {
        let _mf = MfThread::init()?;
        Ok(enumerate()?.into_iter().map(|(info, _)| info).collect())
    })
    .join()
    .map_err(|_| "Camera enumeration panicked".to_string())?
}

fn frame_size(media_type: &IMFMediaType) -> Option<(u32, u32)> {
    // SAFETY: plain attribute read.
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?;
    Some(((packed >> 32) as u32, packed as u32))
}

fn frame_rate(media_type: &IMFMediaType) -> Option<(u32, u32)> {
    // SAFETY: plain attribute read.
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.ok()?;
    let (num, den) = ((packed >> 32) as u32, packed as u32);
    (den != 0).then_some((num, den))
}

fn subtype(media_type: &IMFMediaType) -> Option<GUID> {
    // SAFETY: plain attribute read.
    unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.ok()
}

fn subtype_name(guid: GUID) -> String {
    // Video subtypes are FOURCC GUIDs: the FOURCC is stored in data1.
    let fourcc = guid.data1.to_le_bytes();
    if fourcc.iter().all(|b| b.is_ascii_graphic()) {
        String::from_utf8_lossy(&fourcc).into_owned()
    } else {
        format!("{guid:?}")
    }
}

fn frame_rate_range(media_type: &IMFMediaType) -> Option<(f32, f32)> {
    let read = |key: &GUID| {
        // SAFETY: plain attribute read.
        let packed = unsafe { media_type.GetUINT64(key) }.ok()?;
        let (num, den) = ((packed >> 32) as u32, packed as u32);
        (den != 0).then(|| num as f32 / den as f32)
    };
    Some((
        read(&MF_MT_FRAME_RATE_RANGE_MIN)?,
        read(&MF_MT_FRAME_RATE_RANGE_MAX)?,
    ))
}

/// Pixel formats we can capture, best first (uncompressed needs no decoder).
fn subtype_rank(guid: &GUID) -> u8 {
    match *guid {
        s if s == MFVideoFormat_NV12 => 3,
        s if s == MFVideoFormat_YUY2 => 2,
        s if s == MFVideoFormat_MJPG => 1,
        _ => 0,
    }
}

/// Every native format the camera reports.
fn native_types(reader: &IMFSourceReader) -> Result<Vec<(VideoMode, IMFMediaType)>, String> {
    let mut modes = Vec::new();
    for index in 0.. {
        // SAFETY: plain query; MF_E_NO_MORE_TYPES ends the list.
        let media_type = match unsafe { reader.GetNativeMediaType(STREAM, index) } {
            Ok(t) => t,
            Err(e) if e.code() == MF_E_NO_MORE_TYPES => break,
            Err(e) => return Err(format!("Could not read camera formats: {e}")),
        };
        let (Some((width, height)), Some((num, den)), Some(sub)) = (
            frame_size(&media_type),
            frame_rate(&media_type),
            subtype(&media_type),
        ) else {
            continue;
        };
        let fps = num as f32 / den as f32;
        let (min_fps, max_fps) = frame_rate_range(&media_type).unwrap_or((fps, fps));
        let mode = VideoMode {
            codec: subtype_name(sub),
            width,
            height,
            fps,
            min_fps,
            max_fps,
            supported: subtype_rank(&sub) > 0,
        };
        modes.push((mode, media_type));
    }
    Ok(modes)
}

/// Picks the native format for `config`: exact size first, then a frame
/// rate that matches (directly or inside the type's range), then the
/// preferred pixel format.
///
/// Above 30 fps MJPEG wins: on the Tiny 3 Lite the NV12 types are produced
/// by Windows from the MJPEG stream and only deliver 30 fps when 60 is
/// requested, while MJPEG delivers the full rate.
fn choose_native_type(
    modes: Vec<(VideoMode, IMFMediaType)>,
    config: CaptureConfig,
) -> Result<(VideoMode, IMFMediaType), String> {
    let target = config.fps as f32;
    let score = |m: &VideoMode| {
        let exact_size = m.width == config.width && m.height == config.height;
        let area_gap = -((m.width as i64 * m.height as i64)
            - (config.width as i64 * config.height as i64))
            .abs();
        let fps_ok = (m.fps - target).abs() < 0.5 || (m.min_fps <= target && target <= m.max_fps);
        let fps_gap = -((m.fps - target).abs() * 1000.0) as i64;
        let rank = if config.fps > 30 && m.codec == "MJPG" {
            u8::MAX
        } else {
            m.codec_rank()
        };
        (exact_size, fps_ok, area_gap, fps_gap, rank)
    };
    modes
        .into_iter()
        .filter(|(m, _)| m.supported)
        .max_by_key(|(m, _)| score(m))
        .ok_or_else(|| "The camera offers no supported video format".into())
}

/// Lists the camera's native formats without starting a stream.
pub fn list_modes(device: &DeviceInfo) -> Result<Vec<VideoMode>, String> {
    let id = device.id.clone();
    let name = device.name.clone();
    std::thread::spawn(move || {
        let _mf = MfThread::init()?;
        let activate = enumerate()?
            .into_iter()
            .find(|(info, _)| info.id == id)
            .map(|(_, activate)| activate)
            .ok_or_else(|| format!("{name} is no longer connected"))?;
        // SAFETY: activation and shutdown of a capture source; no streaming.
        unsafe {
            let source: IMFMediaSource = activate
                .ActivateObject()
                .map_err(|e| format!("Could not open {name}: {e}"))?;
            let modes = MFCreateSourceReaderFromMediaSource(&source, None)
                .map_err(|e| e.to_string())
                .and_then(|reader| native_types(&reader));
            let _ = source.Shutdown();
            let _ = activate.ShutdownObject();
            Ok(modes?.into_iter().map(|(m, _)| m).collect())
        }
    })
    .join()
    .map_err(|_| "Format query panicked".to_string())?
}

/// Reads every UVC camera-terminal control Windows maps to
/// `IAMCameraControl` (property ids 0-19 of the KS camera-control set), for
/// `gimbly --probe --controls`. One line per control the camera answers.
/// Then runs `script`: `id=value` sets a control, a bare number waits that
/// many milliseconds; pan, tilt and zoom are read after each step.
pub fn probe_controls(device: &DeviceInfo, script: Vec<String>) -> Result<Vec<String>, String> {
    let id = device.id.clone();
    std::thread::spawn(move || {
        let _mf = MfThread::init()?;
        let activate = enumerate()?
            .into_iter()
            .find(|(info, _)| info.id == id)
            .map(|(_, activate)| activate)
            .ok_or("The camera is no longer connected")?;
        // SAFETY: activation and shutdown of a capture source; all
        // out-pointers are valid locals.
        unsafe {
            let source: IMFMediaSource = activate.ActivateObject().map_err(|e| e.to_string())?;
            let mut lines = Vec::new();
            if let Ok(control) = source.cast::<IAMCameraControl>() {
                for property in 0..20 {
                    let (mut min, mut max, mut step, mut default, mut flags) = (0, 0, 0, 0, 0);
                    let range = control.GetRange(
                        property,
                        &mut min,
                        &mut max,
                        &mut step,
                        &mut default,
                        &mut flags,
                    );
                    let (mut value, mut value_flags) = (0, 0);
                    let current = control.Get(property, &mut value, &mut value_flags);
                    lines.push(match (range, current) {
                        (Err(e), Err(_)) => format!("control {property}: unsupported ({e})"),
                        _ => format!(
                            "control {property}: range {min}..={max} step {step} default {default} \
                             flags {flags:#x}, value {value} flags {value_flags:#x}"
                        ),
                    });
                }
                let mut xu = Xu::open(&source);
                lines.push(format!(
                    "vendor extension unit: {:?}",
                    xu.as_ref().map(|x| x.node)
                ));
                let start = Instant::now();
                for step in &script {
                    // Vendor steps: `move` starts manual movement, `stop`
                    // ends it, `turn:PITCH:YAW:MS` holds a speed, resent
                    // every 100 ms like the official app does.
                    let words: Vec<&str> = step.split(':').collect();
                    let vendor_step = match (xu.as_mut(), words.as_slice()) {
                        (Some(xu), ["move"]) => Some(xu.send(vendor::manual_move(true))),
                        (Some(xu), ["stop"]) => Some(
                            xu.send(vendor::speed(0.0, 0.0))
                                .and_then(|_| xu.send(vendor::manual_move(false))),
                        ),
                        (Some(xu), ["turn", pitch, yaw, ms]) => {
                            let (Ok(pitch), Ok(yaw), Ok(ms)) =
                                (pitch.parse(), yaw.parse(), ms.parse::<u64>())
                            else {
                                return Err(format!("bad step {step:?}"));
                            };
                            let until = Instant::now() + std::time::Duration::from_millis(ms);
                            let mut result = Ok(());
                            while result.is_ok() && Instant::now() < until {
                                result = xu.send(vendor::speed(pitch, yaw));
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                            Some(result)
                        }
                        _ => None,
                    };
                    let result = match vendor_step {
                        Some(r) => r,
                        None => match step.split_once('=') {
                            Some((id, value)) => {
                                let (Ok(id), Ok(value)) = (id.parse(), value.parse()) else {
                                    return Err(format!("bad step {step:?}"));
                                };
                                control
                                    .Set(id, value, CameraControl_Flags_Manual.0)
                                    .map_err(|e| e.to_string())
                            }
                            None => {
                                let ms = step.parse().map_err(|_| format!("bad step {step:?}"))?;
                                std::thread::sleep(std::time::Duration::from_millis(ms));
                                Ok(())
                            }
                        },
                    };
                    let read = |p| {
                        let (mut v, mut f) = (0, 0);
                        control
                            .Get(p, &mut v, &mut f)
                            .map(|_| v)
                            .unwrap_or(i32::MIN)
                    };
                    lines.push(format!(
                        "{:>6} ms {step}: {result:?}; pan {} tilt {} zoom {}",
                        start.elapsed().as_millis(),
                        read(0),
                        read(1),
                        read(3)
                    ));
                }
            } else {
                lines.push("no IAMCameraControl".into());
            }
            let _ = source.Shutdown();
            let _ = activate.ShutdownObject();
            Ok(lines)
        }
    })
    .join()
    .map_err(|_| "Control query panicked".to_string())?
}

/// Geometry of the NV12 buffers the reader produces.
#[derive(Clone, Copy, Debug)]
struct OutputGeometry {
    width: usize,
    height: usize,
    alloc_height: usize,
    range: YuvRange,
}

fn output_geometry(reader: &IMFSourceReader) -> Result<OutputGeometry, String> {
    // SAFETY: attribute reads on the current output type.
    unsafe {
        let current = reader
            .GetCurrentMediaType(STREAM)
            .map_err(|e| e.to_string())?;
        let (w, h) = frame_size(&current).ok_or("Output type has no frame size")?;
        let mut height = h as usize;
        let mut aperture = [0u8; std::mem::size_of::<MFVideoArea>()];
        if current
            .GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut aperture, None)
            .is_ok()
        {
            let area: MFVideoArea = std::ptr::read_unaligned(aperture.as_ptr().cast());
            if area.Area.cy > 0 && (area.Area.cy as usize) < height {
                height = area.Area.cy as usize;
            }
        }
        let range = match current.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) {
            Ok(r) if r as i32 == MFNominalRange_0_255.0 => YuvRange::Full,
            _ => YuvRange::Limited,
        };
        Ok(OutputGeometry {
            width: w as usize & !1,
            height: height & !1,
            alloc_height: h as usize,
            range,
        })
    }
}

fn set_nv12_output(reader: &IMFSourceReader, width: u32, height: u32) -> Result<(), String> {
    // SAFETY: building and applying a media type with valid attributes.
    unsafe {
        let make = |with_color: bool| -> windows::core::Result<IMFMediaType> {
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            t.SetUINT64(&MF_MT_FRAME_SIZE, (width as u64) << 32 | height as u64)?;
            if with_color {
                // Applications reading the virtual camera assume video-range
                // BT.709 for HD NV12, so ask the converter for exactly that.
                t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
                t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
            }
            Ok(t)
        };
        let with_color = make(true).map_err(|e| e.to_string())?;
        if reader
            .SetCurrentMediaType(STREAM, None, &with_color)
            .is_ok()
        {
            return Ok(());
        }
        let plain = make(false).map_err(|e| e.to_string())?;
        reader
            .SetCurrentMediaType(STREAM, None, &plain)
            .map_err(|e| format!("Could not convert the camera stream to NV12: {e}"))
    }
}

/// Reads and applies the UVC zoom control.
struct Zoom {
    control: IAMCameraControl,
    range: ZoomRange,
}

impl Zoom {
    fn open(source: &IMFMediaSource) -> Option<Self> {
        let control: IAMCameraControl = source.cast().ok()?;
        let (mut min, mut max, mut step, mut default, mut flags) = (0, 0, 0, 0, 0);
        // SAFETY: all out-pointers are valid locals.
        unsafe {
            control
                .GetRange(
                    CameraControl_Zoom.0,
                    &mut min,
                    &mut max,
                    &mut step,
                    &mut default,
                    &mut flags,
                )
                .ok()?;
        }
        (max > min).then_some(Self {
            control,
            range: ZoomRange {
                min,
                max,
                step: step.max(1),
                default,
            },
        })
    }

    fn get(&self) -> Option<i32> {
        let (mut value, mut flags) = (0, 0);
        // SAFETY: out-pointers are valid locals.
        unsafe {
            self.control
                .Get(CameraControl_Zoom.0, &mut value, &mut flags)
        }
        .ok()?;
        Some(value)
    }

    fn set(&self, value: i32) -> Result<i32, String> {
        let value = self.range.snap(value);
        // SAFETY: value is inside the range the device reported.
        unsafe {
            self.control
                .Set(CameraControl_Zoom.0, value, CameraControl_Flags_Manual.0)
        }
        .map_err(|e| format!("Zoom failed: {e}"))?;
        Ok(self.get().unwrap_or(value))
    }
}

/// The OBSBOT extension unit, reached through the kernel-streaming property
/// interface the Media Foundation source exposes.
struct Xu {
    control: IKsControl,
    node: u32,
    sender: vendor::Sender,
}

impl Xu {
    /// Finds the extension unit node that answers the vendor GUID.
    fn open(source: &IMFMediaSource) -> Option<Self> {
        let control: IKsControl = source.cast().ok()?;
        let topology: IKsTopologyInfo = source.cast().ok()?;
        // SAFETY: plain topology queries.
        let nodes = unsafe { topology.NumNodes() }.ok()?;
        (0..nodes)
            .filter(|&n| unsafe { topology.get_NodeType(n) }.ok() == Some(KSNODETYPE_DEV_SPECIFIC))
            .map(|node| Self {
                control: control.clone(),
                node,
                sender: vendor::Sender::default(),
            })
            .find(|xu| xu.get(vendor::SELECTOR_COMMAND).is_ok())
    }

    fn property(&self, selector: u32, flags: u32, data: &mut [u8]) -> Result<u32, String> {
        let mut property = KSP_NODE {
            NodeId: self.node,
            ..Default::default()
        };
        property.Property.Anonymous.Anonymous = KSIDENTIFIER_0_0 {
            Set: GUID::from_u128(vendor::XU_GUID),
            Id: selector,
            Flags: flags | KSPROPERTY_TYPE_TOPOLOGY,
        };
        let mut returned = 0u32;
        // SAFETY: `property` is a valid KSP_NODE, whose first member is the
        // KSIDENTIFIER; `data` is valid for its length.
        unsafe {
            self.control.KsProperty(
                &property.Property,
                std::mem::size_of::<KSP_NODE>() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut returned,
            )
        }
        .map_err(|e| format!("Camera control failed: {e}"))?;
        Ok(returned)
    }

    fn send(&mut self, message: vendor::Message) -> Result<(), String> {
        let mut block = self.sender.number(message);
        self.property(vendor::SELECTOR_COMMAND, KSPROPERTY_TYPE_SET, &mut block)
            .map(|_| ())
    }

    fn get(&self, selector: u32) -> Result<[u8; vendor::MESSAGE_LEN], String> {
        let mut block = [0u8; vendor::MESSAGE_LEN];
        self.property(selector, KSPROPERTY_TYPE_GET, &mut block)?;
        Ok(block)
    }
}

/// Locks a sample's buffer and exposes it as NV12 for the duration of `f`.
fn with_nv12<R>(
    sample: &IMFSample,
    geometry: OutputGeometry,
    f: impl FnOnce(&Nv12View<'_>) -> R,
) -> Result<Option<R>, String> {
    // SAFETY: the buffer stays locked while `data` is borrowed and is
    // unlocked on every path before returning.
    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| e.to_string())?;
        if let Ok(buffer_2d) = buffer.cast::<IMF2DBuffer>() {
            let mut scanline0 = std::ptr::null_mut();
            let mut pitch = 0i32;
            buffer_2d
                .Lock2D(&mut scanline0, &mut pitch)
                .map_err(|e| e.to_string())?;
            let result = if pitch > 0 && !scanline0.is_null() {
                let pitch = pitch as usize;
                let len = pitch * geometry.alloc_height * 3 / 2;
                let data = std::slice::from_raw_parts(scanline0, len);
                Nv12View::with_stride(
                    data,
                    geometry.width,
                    geometry.height,
                    pitch,
                    geometry.alloc_height,
                )
                .map(|v| f(&v))
            } else {
                None
            };
            let _ = buffer_2d.Unlock2D();
            return Ok(result);
        }
        let mut data = std::ptr::null_mut();
        let mut len = 0u32;
        buffer
            .Lock(&mut data, None, Some(&mut len))
            .map_err(|e| e.to_string())?;
        let result = if data.is_null() {
            None
        } else {
            let slice = std::slice::from_raw_parts(data, len as usize);
            Nv12View::with_stride(
                slice,
                geometry.width,
                geometry.height,
                geometry.width,
                geometry.alloc_height,
            )
            .map(|v| f(&v))
        };
        let _ = buffer.Unlock();
        Ok(result)
    }
}

pub fn run_capture(
    device: &DeviceInfo,
    config: CaptureConfig,
    worker: &Worker,
) -> Result<(), String> {
    let _mf = MfThread::init()?;
    let activate = enumerate()?
        .into_iter()
        .find(|(info, _)| info.id == device.id)
        .map(|(_, activate)| activate)
        .ok_or_else(|| format!("{} is no longer connected", device.name))?;

    // SAFETY: standard activation of a capture source; the source is shut
    // down at the end of this function.
    let source: IMFMediaSource = unsafe { activate.ActivateObject() }.map_err(|e| {
        format!(
            "Could not open {}: {e}. Close other apps that use the camera and try again.",
            device.name
        )
    })?;
    let result = capture_loop(&source, device, config, worker);
    // SAFETY: releases the device so other apps can use it.
    unsafe {
        let _ = source.Shutdown();
        let _ = activate.ShutdownObject();
    }
    result
}

fn capture_loop(
    source: &IMFMediaSource,
    device: &DeviceInfo,
    config: CaptureConfig,
    worker: &Worker,
) -> Result<(), String> {
    // SAFETY: plain Media Foundation setup calls with valid arguments.
    let reader = unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1).map_err(|e| e.to_string())?;
        let attributes = attributes.ok_or("MFCreateAttributes returned nothing")?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .map_err(|e| e.to_string())?;
        MFCreateSourceReaderFromMediaSource(source, &attributes).map_err(|e| e.to_string())?
    };

    let types = native_types(&reader)?;
    worker.shared.status().modes = types.iter().map(|(m, _)| m.clone()).collect();
    let (mode, native) = choose_native_type(types, config)?;
    let (width, height) = (mode.width, mode.height);
    let mut fps = mode.fps;
    if (mode.fps - config.fps as f32).abs() >= 0.5
        && mode.min_fps <= config.fps as f32
        && config.fps as f32 <= mode.max_fps
    {
        // The type covers a range; ask for the configured rate inside it.
        // SAFETY: plain attribute write on our copy of the media type.
        if unsafe { native.SetUINT64(&MF_MT_FRAME_RATE, (config.fps as u64) << 32 | 1) }.is_ok() {
            fps = config.fps as f32;
        }
    }
    // SAFETY: `native` came from this reader.
    unsafe { reader.SetCurrentMediaType(STREAM, None, &native) }.map_err(|e| {
        format!(
            "Camera rejected format {} {width}x{height}: {e}",
            mode.codec
        )
    })?;
    set_nv12_output(&reader, width, height)?;
    let mut geometry = output_geometry(&reader)?;

    let zoom = Zoom::open(source);
    let mut xu = device.is_obsbot().then(|| Xu::open(source)).flatten();
    {
        let mut status = worker.shared.status();
        status.format = Some(format!("{} {width}x{height} @ {fps:.0} fps", mode.codec));
        status.output = Some(config.output_size(width, height));
        status.zoom_range = zoom.as_ref().map(|z| z.range);
        status.set_zoom(zoom.as_ref().and_then(Zoom::get));
        // Only the Tiny 3 Lite was checked against the official app so far.
        let verified = device.usb_product_id == Some(TINY_3_LITE);
        status.vendor_control = xu.is_some() && verified;
        status.vendor_control_untested = device.is_obsbot() && !status.vendor_control;
    }

    let mut virtual_camera: Option<SharedMemory> = None;
    let mut fps_window = (Instant::now(), 0u32);
    let mut first_frame = true;
    let mut zoom_polled = Instant::now();
    let mut snapshot_pending = false;
    let mut transform = config.transform;
    let mut turned_buf = Vec::new();
    let mut sleep_buf: Option<((usize, usize), Vec<u8>)> = None;

    while !worker.should_stop() {
        // Only the newest zoom request matters, e.g. while a slider is dragged.
        let mut zoom_request = None;
        while let Ok(command) = worker.commands.try_recv() {
            match command {
                Command::SetZoom(value) => zoom_request = Some(value),
                Command::SetVirtualCamera(enabled) => {
                    if let Some(shm) = virtual_camera.take() {
                        shm.region().mark_stopped();
                    }
                    let mut status = worker.shared.status();
                    if enabled {
                        match SharedMemory::create() {
                            Ok(shm) => virtual_camera = Some(shm),
                            Err(e) => {
                                status.error = Some(format!("Virtual camera output failed: {e}"))
                            }
                        }
                    }
                    status.virtual_camera = virtual_camera.is_some();
                }
                Command::Snapshot => snapshot_pending = true,
                Command::SetTransform(t) => transform = t,
                Command::SetSleep(_) => {
                    worker.shared.status().error = Some("Sleep is not implemented yet".into())
                }
                Command::Gimbal(GimbalCommand::ZoomRatio(ratio)) if zoom.is_some() => {
                    zoom_request = zoom.as_ref().map(|z| z.range.value_for_ratio(ratio))
                }
                Command::Gimbal(command) => {
                    let result = match (xu.as_mut(), vendor::for_gimbal(&command)) {
                        (Some(xu), Some(message)) => xu.send(message),
                        (None, _) => Err("This camera does not accept gimbal commands".into()),
                        (_, None) => Err(format!("{command:?} is not implemented yet")),
                    };
                    if let Err(e) = result {
                        worker.shared.status().error = Some(e);
                    }
                }
            }
        }
        if let Some(value) = zoom_request {
            let result = match zoom.as_ref() {
                Some(z) => z.set(value),
                None => Err("This camera does not report a zoom control".into()),
            };
            let mut status = worker.shared.status();
            match result {
                Ok(v) => status.set_zoom(Some(v)),
                Err(e) => status.error = Some(e),
            }
            zoom_polled = Instant::now();
        } else if let Some(z) = zoom
            .as_ref()
            .filter(|_| zoom_polled.elapsed().as_millis() >= 250)
        {
            // The camera zooms smoothly; keep its reported position current.
            worker.shared.status().set_zoom(z.get());
            zoom_polled = Instant::now();
        }

        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        // SAFETY: synchronous read with valid out-pointers.
        unsafe { reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .map_err(|e| format!("Camera stream stopped: {e}"))?;
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err("The camera reported a stream error (was it unplugged?)".into());
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Err("The camera stream ended".into());
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            geometry = output_geometry(&reader)?;
        }
        let Some(sample) = sample else { continue };

        let asleep = {
            let status = worker.shared.status();
            status.sleeping && status.vendor_control
        };
        // The preview window draws its own placeholder while the camera sleeps.
        let want_preview = !asleep && worker.shared.frame().is_none();
        let shm = virtual_camera.as_ref();
        let want_snapshot = std::mem::take(&mut snapshot_pending);
        let needed = shm.is_some() || want_preview || want_snapshot;
        let frames = with_nv12(&sample, geometry, |full| {
            let cropped = config.portrait.then(|| full.crop_to_portrait()).flatten();
            let view = cropped.as_ref().unwrap_or(full);
            // Rotate/mirror once so the preview, snapshots and the virtual
            // camera all get the same picture.
            let turned = (needed && !transform.is_identity())
                .then(|| gimbly_vcam_shm::pixel::transform_into(view, transform, &mut turned_buf));
            let view = turned.as_ref().unwrap_or(view);
            // A sleeping camera keeps streaming the vendor's still picture;
            // video apps and snapshots get our own instead.
            let placeholder = if asleep {
                let size = (view.width, view.height);
                if sleep_buf.as_ref().is_none_or(|(built, _)| *built != size) {
                    let icon = crate::branding::window_icon();
                    let frame = preview::sleep_frame(
                        size.0,
                        size.1,
                        &icon.rgba,
                        icon.width as usize,
                        geometry.range,
                    );
                    sleep_buf = Some((size, frame));
                }
                sleep_buf
                    .as_ref()
                    .and_then(|(_, frame)| Nv12View::packed(frame, size.0, size.1))
            } else {
                None
            };
            let view = placeholder.as_ref().unwrap_or(view);
            if let Some(shm) = shm {
                shm.region().publish(view, now_ms());
            }
            (
                want_preview.then(|| preview::convert(view, geometry.range)),
                want_snapshot.then(|| preview::convert_full(view, geometry.range)),
            )
        })?;
        let (preview, snapshot) = frames.unwrap_or_default();
        // A frame that could not be read keeps the request for the next one.
        snapshot_pending = want_snapshot && snapshot.is_none();

        if let Some(frame) = snapshot {
            *worker.shared.snapshot() = Some(frame);
            (worker.repaint)();
        }
        if let Some(frame) = preview {
            *worker.shared.frame() = Some(frame);
            (worker.repaint)();
        }

        fps_window.1 += 1;
        let elapsed = fps_window.0.elapsed().as_secs_f32();
        if first_frame || elapsed >= 1.0 {
            let mut status = worker.shared.status();
            status.state = SessionState::Streaming;
            if !first_frame {
                status.measured_fps = fps_window.1 as f32 / elapsed;
                fps_window = (Instant::now(), 0);
            }
            first_frame = false;
        }
    }

    if let Some(shm) = virtual_camera.take() {
        shm.region().mark_stopped();
    }
    Ok(())
}
