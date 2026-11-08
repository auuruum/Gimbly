//! Windows camera backend built on Media Foundation.
//!
//! Capture uses an `IMFSourceReader` with advanced video processing enabled,
//! so Media Foundation decodes the camera's MJPEG stream and hands us NV12.
//! Standard zoom uses `IAMCameraControl`. The Tiny 3 Lite uses vendor
//! controls through the camera's UVC extension unit.

use super::preview::{self, YuvRange};
use super::vendor;
use super::{
    CaptureConfig, Command, DeviceInfo, GimbalCommand, PRESET_SLOTS, Preset, SessionState, Shared,
    VideoMode, Worker, ZoomRange, parse_usb_ids,
};
use gimbly_vcam_shm::{Nv12View, SharedMemory, now_ms};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
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
                        (Some(xu), ["status"]) => {
                            lines.push(format!("selector 6: {:?}", xu.get(vendor::SELECTOR_VIEW)));
                            lines.push(format!("vendor zoom: {:?}", xu.read_zoom()));
                            lines.push(format!("presets: {:?}", xu.presets()));
                            Some(Ok(()))
                        }
                        (Some(xu), ["ptz"]) => {
                            lines.push(format!(
                                "current position bytes: {:?}",
                                xu.request(vendor::READ_POSITION, Vec::new())
                            ));
                            Some(Ok(()))
                        }
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
    last_write: Option<Instant>,
}

// SAFETY: the control comes from a media source opened on a thread in the
// multithreaded apartment and is only used from threads that joined it
// (see `control_loop`); kernel-streaming property calls are free-threaded.
unsafe impl Send for Xu {}

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
                last_write: None,
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

    fn send(&mut self, mut message: vendor::Message) -> Result<(), String> {
        self.sender.number(&mut message);
        self.set(vendor::SELECTOR_COMMAND, &mut message.encode())
    }

    fn set(&mut self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<(), String> {
        // Separate successive writes, as in the capture. The device needs
        // time to consume a SET before another message replaces it.
        if let Some(delay) = self
            .last_write
            .and_then(|last| Duration::from_millis(20).checked_sub(last.elapsed()))
        {
            std::thread::sleep(delay);
        }
        let result = self
            .property(selector, KSPROPERTY_TYPE_SET, block)
            .map(|_| ());
        self.last_write = Some(Instant::now());
        result
    }

    fn get(&self, selector: u32) -> Result<([u8; vendor::MESSAGE_LEN], usize), String> {
        let mut block = [0u8; vendor::MESSAGE_LEN];
        let len = self.property(selector, KSPROPERTY_TYPE_GET, &mut block)? as usize;
        if len > block.len() {
            return Err("Camera returned an oversized extension unit block".into());
        }
        Ok((block, len))
    }

    fn request(&mut self, command: [u8; 2], data: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut request = vendor::Message::new(vendor::GIMBAL, command, data).requesting_reply();
        self.sender.number(&mut request);
        self.set(vendor::SELECTOR_COMMAND, &mut request.encode())?;
        // The XU may still return an older reply before this one is ready.
        std::thread::sleep(Duration::from_millis(40));
        for _ in 0..5 {
            if let Ok((block, len)) = self.get(vendor::SELECTOR_COMMAND)
                && let Some(reply) = vendor::Message::decode(&block[..len])
                && reply.matches_reply(&request)
            {
                return Ok(reply.data);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(format!(
            "No matching camera reply for {:02x} {:02x}",
            command[0], command[1]
        ))
    }

    fn read_sleep(&self) -> Option<bool> {
        let (block, len) = self.get(vendor::SELECTOR_VIEW).ok()?;
        vendor::sleeping(&block[..len])
    }

    fn read_zoom(&mut self) -> Result<f32, String> {
        vendor::read_zoom(&self.request(vendor::READ_ZOOM, Vec::new())?)
            .ok_or_else(|| "Camera returned an invalid zoom ratio".into())
    }

    fn preset_ids(&mut self) -> Result<Vec<u8>, String> {
        let data = self.request(vendor::LIST_PRESETS, Vec::new())?;
        vendor::preset_ids(&data).ok_or("Camera returned an invalid preset list")?;
        Ok(data)
    }

    fn preset_name(&mut self, id: u8) -> Result<String, String> {
        let name = self.request(vendor::READ_PRESET_NAME, vendor::preset_id(id))?;
        vendor::read_preset_name(&name)
            .ok_or_else(|| "Camera returned an invalid preset name".into())
    }

    fn presets(&mut self) -> Result<Vec<Preset>, String> {
        self.preset_ids()?
            .into_iter()
            .map(|id| {
                let mut name = self.preset_name(id)?;
                if name.trim().is_empty() {
                    name = format!("Preset{}", u16::from(id) + 1);
                }
                let mut data = vendor::preset_id(id);
                data.push(0);
                let data = self.request(vendor::READ_PRESET_POSITION, data)?;
                let position = vendor::read_preset_position(&data, id)
                    .ok_or("Camera returned an invalid stored preset position")?;
                Ok(Preset {
                    id,
                    name,
                    position: Some(position),
                })
            })
            .collect()
    }

    fn store_preset(&mut self, id: u8) -> Result<(), String> {
        let current = self.request(vendor::READ_POSITION, Vec::new())?;
        let ratio = self.read_zoom()?;
        let message = vendor::store_preset(id, &current, ratio)
            .ok_or("Camera returned an invalid gimbal position")?;
        self.send(message)
    }

    fn preset_change(&mut self, command: GimbalCommand) -> Result<(), String> {
        match command {
            GimbalCommand::AddPreset => {
                let ids = self.preset_ids()?;
                let id = (0..PRESET_SLOTS as u8)
                    .find(|id| !ids.contains(id))
                    .filter(|_| ids.len() < PRESET_SLOTS)
                    .ok_or("All preset slots are occupied")?;
                let name = vendor::preset_name(id, &format!("Preset{}", id + 1))?;
                self.store_preset(id)?;
                self.send(name)
            }
            GimbalCommand::UpdatePreset { id } => {
                // Writing the position also overwrites the stored name.
                let name = self.preset_name(id)?;
                let name = vendor::preset_name(id, &name)?;
                self.store_preset(id)?;
                self.send(name)
            }
            GimbalCommand::RenamePreset { id, name } => self.send(vendor::preset_name(id, &name)?),
            GimbalCommand::DeletePreset(_) => {
                self.send(vendor::for_gimbal(&command).ok_or("Unsupported preset command")?)
            }
            _ => Err("Unsupported preset command".into()),
        }
    }

    fn gimbal(
        &mut self,
        command: GimbalCommand,
        shared: &Shared,
        zoom_refresh: &mut ZoomRefresh,
    ) -> Result<(), String> {
        match command {
            GimbalCommand::SetFov(fov) => {
                if shared.status().fov == Some(fov) {
                    return Ok(());
                }
                self.set(vendor::SELECTOR_VIEW, &mut vendor::field_of_view(fov))?;
                shared.status().fov = Some(fov);
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::ZoomRatio(ratio) => {
                let (message, ratio) = vendor::zoom(ratio).ok_or("Zoom ratio is out of range")?;
                if shared.status().zoom_ratio == Some(ratio) && shared.status().fov.is_none() {
                    return Ok(());
                }
                self.send(message)?;
                let mut status = shared.status();
                status.zoom_ratio = Some(ratio);
                status.fov = None;
                *zoom_refresh = None;
            }
            GimbalCommand::AddPreset
            | GimbalCommand::UpdatePreset { .. }
            | GimbalCommand::RenamePreset { .. }
            | GimbalCommand::DeletePreset(_) => {
                let change = self.preset_change(command);
                let presets = self.presets();
                shared.status().presets = presets.as_ref().ok().cloned();
                change?;
                presets?;
            }
            _ => {
                self.send(vendor::for_gimbal(&command).ok_or("Unsupported gimbal command")?)?;
                if matches!(command, GimbalCommand::TriggerPreset(_)) {
                    shared.status().fov = None;
                    *zoom_refresh = follow_zoom();
                }
            }
        }
        Ok(())
    }
}

/// Next zoom read, and until when reads repeat every [`ZOOM_FOLLOW_EVERY`].
type ZoomRefresh = Option<(Instant, Instant)>;

/// The camera zooms smoothly after a field of view or preset change; reading
/// the ratio this often lets the slider move along with it.
const ZOOM_FOLLOW_EVERY: Duration = Duration::from_millis(200);
/// Long enough for the camera to settle after a field of view change.
const ZOOM_FOLLOW_FOR: Duration = Duration::from_millis(2500);

fn read_zoom_in(delay: Duration) -> ZoomRefresh {
    let at = Instant::now() + delay;
    Some((at, at))
}

fn follow_zoom() -> ZoomRefresh {
    let now = Instant::now();
    Some((now + ZOOM_FOLLOW_EVERY, now + ZOOM_FOLLOW_FOR))
}

/// Carries out vendor commands and polls the camera's state. Every vendor
/// write and read waits for the camera, so this runs on its own thread and
/// the frame loop never waits on it. Returns when `commands` disconnects.
fn control_loop(
    mut xu: Xu,
    commands: &Receiver<Command>,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
    mut zoom_refresh: ZoomRefresh,
) {
    // The control must be used from the multithreaded apartment.
    let _com = MfThread::init();
    let mut sleep_polled = Instant::now();
    let mut preset_retries = 0;
    loop {
        let mut wake = sleep_polled + Duration::from_secs(1);
        if let Some((next, _)) = zoom_refresh {
            wake = wake.min(next);
        }
        let first = match commands.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Everything that queued up during the last write goes out as one
        // batch, with stale joystick speeds and zoom ratios dropped.
        let batch = first
            .into_iter()
            .chain(std::iter::from_fn(|| commands.try_recv().ok()));
        for command in super::coalesce(batch) {
            match command {
                Command::SetSleep(sleep) => {
                    let result = xu.send(vendor::sleep(sleep));
                    let mut status = shared.status();
                    match result {
                        Ok(()) => {
                            status.sleeping = sleep;
                            sleep_polled = Instant::now();
                            preset_retries = 0;
                            zoom_refresh = if sleep {
                                None
                            } else {
                                read_zoom_in(Duration::from_secs(3))
                            };
                        }
                        Err(error) => status.error = Some(error),
                    }
                    drop(status);
                    repaint();
                }
                Command::Gimbal(command) => {
                    if let Err(e) = xu.gimbal(command, shared, &mut zoom_refresh) {
                        shared.status().error = Some(e);
                    }
                    repaint();
                }
                _ => {}
            }
        }
        if sleep_polled.elapsed() >= Duration::from_secs(1) {
            if let Some(sleeping) = xu.read_sleep() {
                let mut status = shared.status();
                if status.sleeping != sleeping {
                    status.sleeping = sleeping;
                    preset_retries = 0;
                    zoom_refresh = if sleeping {
                        None
                    } else {
                        read_zoom_in(Duration::from_secs(2))
                    };
                    drop(status);
                    repaint();
                }
            }
            sleep_polled = Instant::now();
            // Opening or waking the stream can briefly interrupt replies.
            let retry_presets = {
                let status = shared.status();
                if status.presets.is_some() {
                    preset_retries = 0;
                }
                !status.sleeping && status.presets.is_none() && preset_retries < 3
            };
            if retry_presets {
                preset_retries += 1;
                if let Ok(presets) = xu.presets() {
                    shared.status().presets = Some(presets);
                    repaint();
                }
            }
        }
        if let Some((next, until)) = zoom_refresh.filter(|(next, _)| Instant::now() >= *next) {
            zoom_refresh = (next < until).then(|| {
                let again = Instant::now() + ZOOM_FOLLOW_EVERY;
                (again.min(until), until)
            });
            if let Ok(ratio) = xu.read_zoom() {
                let mut status = shared.status();
                if status.zoom_ratio != Some(ratio) {
                    status.zoom_ratio = Some(ratio);
                    drop(status);
                    repaint();
                }
            }
        }
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
    // The scope joins the vendor control thread before the source shuts down.
    let result = std::thread::scope(|scope| capture_loop(&source, device, config, worker, scope));
    // SAFETY: releases the device so other apps can use it.
    unsafe {
        let _ = source.Shutdown();
        let _ = activate.ShutdownObject();
    }
    result
}

fn capture_loop<'scope, 'env>(
    source: &IMFMediaSource,
    device: &DeviceInfo,
    config: CaptureConfig,
    worker: &'env Worker,
    scope: &'scope std::thread::Scope<'scope, 'env>,
) -> Result<(), String> {
    // Read the initial controls before Media Foundation changes stream types.
    // Only the Tiny 3 Lite has been checked against the official app so far.
    let mut xu = (device.is_obsbot() && device.usb_product_id == Some(TINY_3_LITE))
        .then(|| Xu::open(source))
        .flatten();
    let mut zoom_refresh: ZoomRefresh = None;
    if let Some(xu) = xu.as_mut() {
        let sleeping = xu.read_sleep();
        let ratio = xu.read_zoom().ok();
        let presets = xu.presets().ok();
        if ratio.is_none() {
            zoom_refresh = read_zoom_in(Duration::from_secs(1));
        }
        let mut status = worker.shared.status();
        status.sleeping = sleeping.unwrap_or(false);
        status.zoom_ratio = ratio;
        status.presets = presets;
    }

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

    let zoom = if xu.is_none() {
        Zoom::open(source)
    } else {
        None
    };
    {
        let mut status = worker.shared.status();
        status.format = Some(format!("{} {width}x{height} @ {fps:.0} fps", mode.codec));
        status.output = Some(config.output_size(width, height));
        status.zoom_range = zoom.as_ref().map(|z| z.range);
        status.zoom = zoom.as_ref().and_then(Zoom::get);
        status.vendor_control = xu.is_some();
        status.vendor_control_untested = device.is_obsbot() && !status.vendor_control;
    }

    let control: Option<Sender<Command>> = xu.map(|xu| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let shared: &'env Shared = &worker.shared;
        let repaint: &'env (dyn Fn() + Send + Sync) = &*worker.repaint;
        scope.spawn(move || control_loop(xu, &receiver, shared, repaint, zoom_refresh));
        sender
    });

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
                // Vendor commands wait on the camera; they run on the control thread.
                Command::SetSleep(_) | Command::Gimbal(_) => match &control {
                    Some(control) => {
                        let _ = control.send(command);
                    }
                    None => {
                        worker.shared.status().error = Some(match command {
                            Command::SetSleep(_) => "Sleep is unsupported for this camera".into(),
                            _ => "This camera does not accept gimbal commands".into(),
                        })
                    }
                },
            }
        }
        if let Some(value) = zoom_request {
            let result = match zoom.as_ref() {
                Some(z) => z.set(value),
                None => Err("This camera does not report a zoom control".into()),
            };
            let mut status = worker.shared.status();
            match result {
                Ok(v) => status.zoom = Some(v),
                Err(e) => status.error = Some(e),
            }
            zoom_polled = Instant::now();
        } else if let Some(z) = zoom
            .as_ref()
            .filter(|_| zoom_polled.elapsed().as_millis() >= 250)
        {
            // The camera zooms smoothly; keep its reported position current.
            worker.shared.status().zoom = z.get();
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
