//! Camera discovery, capture and control.
//!
//! The shared part of this module defines the session API used by the UI. The
//! OS-specific backends provide Media Foundation, V4L2 and AVFoundation.

mod control;
mod frames;
mod preview;
mod queue;
mod sleep;
mod standby;
mod vendor;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as backend;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use self::linux as backend;
#[cfg(target_os = "linux")]
pub(crate) use self::linux::set_nonblocking;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use self::macos as backend;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
use self::unsupported as backend;

pub use gimbly_vcam_shm::Transform;
pub use preview::{PreviewFrame, YuvRange};

use queue::{Receiver, Sender, channel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// USB vendor ID used by OBSBOT (Remo Tech) cameras.
pub const OBSBOT_USB_VENDOR_ID: u16 = 0x3564;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    /// Stable OS identifier used to reopen the device (symbolic link on
    /// Windows).
    pub id: String,
    pub usb_vendor_id: Option<u16>,
    pub usb_product_id: Option<u16>,
}

impl DeviceInfo {
    pub fn is_obsbot(&self) -> bool {
        self.usb_vendor_id == Some(OBSBOT_USB_VENDOR_ID)
    }
}

/// Range reported by the camera for the UVC zoom (absolute) control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoomRange {
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub default: i32,
}

impl ZoomRange {
    /// Clamps `value` into the range and onto the step grid.
    pub fn snap(&self, value: i32) -> i32 {
        let clamped = value.clamp(self.min, self.max);
        if self.step <= 1 {
            return clamped;
        }
        let (min, max, step) = (
            i64::from(self.min),
            i64::from(self.max),
            i64::from(self.step),
        );
        let steps = (i64::from(clamped) - min + step / 2) / step;
        (min + steps * step).min(max) as i32
    }
}

/// One native capture format reported by the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoMode {
    /// FOURCC of the pixel format, e.g. "NV12" or "MJPG".
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub min_fps: f32,
    pub max_fps: f32,
    /// Whether Gimbly can capture this format.
    pub supported: bool,
}

impl VideoMode {
    /// Formats that need no decoding rank higher.
    pub fn codec_rank(&self) -> u8 {
        match self.codec.as_str() {
            "NV12" => 3,
            "YUY2" => 2,
            "MJPG" => 1,
            _ => 0,
        }
    }

    /// True when the camera can deliver `width`x`height` at `fps`.
    pub fn covers(&self, width: u32, height: u32, fps: u32) -> bool {
        let fps = fps as f32;
        self.supported
            && self.width == width
            && self.height == height
            && ((self.fps - fps).abs() < 0.5 || (self.min_fps <= fps && fps <= self.max_fps))
    }
}

/// What a capture session asks the camera for and how it shapes the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Crop the centre 9:16 of the picture for vertical output.
    pub portrait: bool,
    /// Rotation and mirror for the preview, snapshots and virtual camera;
    /// changed live with [`Command::SetTransform`].
    pub transform: Transform,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            portrait: false,
            transform: Transform::default(),
        }
    }
}

impl CaptureConfig {
    /// Size of the published picture for a capture of `width`x`height`.
    pub fn output_size(&self, width: u32, height: u32) -> (u32, u32) {
        if self.portrait {
            (portrait_width(height).min(width), height)
        } else {
            (width, height)
        }
    }

    /// Standard video-app sizes for the portrait crop. The preview and
    /// snapshots keep the actual cropped dimensions.
    pub fn virtual_size(&self, (width, height): (u32, u32)) -> (u32, u32) {
        if self.portrait {
            if height >= 1080 {
                (1080, 1920)
            } else {
                (720, 1280)
            }
        } else {
            (width, height)
        }
    }
}

/// Even width of a 9:16 picture with the given height.
pub fn portrait_width(height: u32) -> u32 {
    (height * 9 / 16) & !1
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionState {
    #[default]
    Opening,
    Streaming,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct SessionStatus {
    pub state: SessionState,
    /// Human-readable device format, e.g. "MJPG 1920x1080 @ 30 fps".
    pub format: Option<String>,
    /// Size of the picture shown in the preview and sent to the virtual
    /// camera (differs from the capture size in portrait mode).
    pub output: Option<(u32, u32)>,
    /// Native formats the camera reported when the session started.
    pub modes: Vec<VideoMode>,
    pub measured_fps: f32,
    pub zoom_range: Option<ZoomRange>,
    pub zoom: Option<i32>,
    /// The camera accepts OBSBOT vendor commands such as sleep.
    pub vendor_control: bool,
    /// An OBSBOT camera whose vendor commands are not verified yet.
    pub vendor_control_untested: bool,
    pub sleeping: bool,
    /// Changes on every sleep or wake so the preview can reject old video.
    pub video_epoch: u64,
    /// Zoom ratio the camera reports through vendor control.
    pub zoom_ratio: Option<f32>,
    /// Field of view the camera reports; `None` after a manual zoom.
    pub fov: Option<Fov>,
    /// Stored preset positions, in the camera's order. `None` until read.
    pub presets: Option<Vec<Preset>>,
    pub virtual_camera: bool,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    /// Zoom through the standard UVC control (cameras without OBSBOT vendor
    /// control).
    SetZoom(i32),
    SetVirtualCamera(bool),
    /// Convert the next frame at full size for [`Session::take_snapshot`].
    Snapshot,
    SetTransform(Transform),
    /// Put the camera to sleep (`true`) or wake it up.
    SetSleep(bool),
    /// An OBSBOT vendor command (gimbal, zoom ratio, presets).
    Gimbal(GimbalCommand),
}

/// Commands for OBSBOT cameras with a gimbal, carried out off the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum GimbalCommand {
    /// A joystick drag started (`true`) or ended.
    Drag(bool),
    /// Turn at these speeds (see `crate::gimbal`); zero stops.
    Speed {
        pitch: f32,
        yaw: f32,
    },
    /// Return to the initial position.
    Reset,
    /// Zoom to a ratio between 1.0 and [`ZOOM_RATIO_MAX`].
    ZoomRatio(f32),
    SetFov(Fov),
    TriggerPreset(u8),
    /// Store the current position as a new preset in the first free slot.
    AddPreset,
    /// Replace preset `id` with the current position, keeping its name.
    UpdatePreset {
        id: u8,
    },
    DeletePreset(u8),
    RenamePreset {
        id: u8,
        name: String,
    },
}

/// The three fixed fields of view of the View and Gimbal panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fov {
    Wide,
    Medium,
    Narrow,
}

impl Fov {
    pub const ALL: [Fov; 3] = [Fov::Wide, Fov::Medium, Fov::Narrow];
}

/// Largest zoom ratio of the Tiny 3 Lite.
pub const ZOOM_RATIO_MAX: f32 = 4.0;
/// Number of preset slots on the Tiny 3 Lite.
pub const PRESET_SLOTS: usize = 3;
/// Bytes available for a preset name in one vendor message.
pub const PRESET_NAME_MAX: usize = 40;

/// A stored gimbal position as listed in the Preset section.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u8,
    pub name: String,
    /// Angles and zoom the camera stored, when it answered.
    pub position: Option<PresetPosition>,
}

/// A stored gimbal position. Angles in degrees, zoom as a ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresetPosition {
    pub id: u8,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub zoom: f32,
}

/// State shared between the capture thread and the UI.
#[derive(Default)]
pub struct Shared {
    // When taking both locks, take status before frame so the slot and its
    // video epoch always change together.
    frame: Mutex<Option<PreviewFrame>>,
    snapshot: Mutex<Option<PreviewFrame>>,
    status: Mutex<SessionStatus>,
}

impl Shared {
    fn status(&self) -> MutexGuard<'_, SessionStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn frame(&self) -> MutexGuard<'_, Option<PreviewFrame>> {
        self.frame.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_sleeping(&self, sleeping: bool) -> bool {
        let mut status = self.status();
        if status.sleeping == sleeping {
            return false;
        }
        status.sleeping = sleeping;
        status.video_epoch = status.video_epoch.wrapping_add(1);
        self.frame().take();
        true
    }

    fn publish_preview(&self, epoch: u64, frame: PreviewFrame) -> bool {
        let status = self.status();
        if status.video_epoch != epoch || status.sleeping {
            return false;
        }
        *self.frame() = Some(frame);
        true
    }

    fn snapshot(&self) -> MutexGuard<'_, Option<PreviewFrame>> {
        self.snapshot.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fail(&self, message: String) {
        let mut status = self.status();
        status.state = SessionState::Failed;
        status.error = Some(message);
    }
}

/// Context handed to the platform capture loop.
pub struct Worker {
    pub shared: Arc<Shared>,
    commands: Receiver,
    pub stop: Arc<AtomicBool>,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Worker {
    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

fn forward_control(worker: &Worker, control: Option<&Sender>, command: Command) {
    let error = match control {
        Some(control) if control.send(command) => return,
        Some(_) => "Camera control queue is full",
        None => "This camera does not support the requested control",
    };
    worker.shared.status().error = Some(error.into());
    (worker.repaint)();
}

fn mode_score(m: &VideoMode, config: CaptureConfig) -> (bool, bool, i64, i64, u8) {
    let target = config.fps as f32;
    let exact_size = m.width == config.width && m.height == config.height;
    let area = u64::from(m.width) * u64::from(m.height);
    let target_area = u64::from(config.width) * u64::from(config.height);
    let area_gap = -(area.abs_diff(target_area).min(i64::MAX as u64) as i64);
    let fps_ok = (m.fps - target).abs() < 0.5 || (m.min_fps <= target && target <= m.max_fps);
    let fps_gap = -((m.fps - target).abs() * 1000.0) as i64;
    let rank = if config.fps > 30 && m.codec == "MJPG" {
        u8::MAX
    } else {
        m.codec_rank()
    };
    (exact_size, fps_ok, area_gap, fps_gap, rank)
}

// The UI can replace a session immediately. The next capture worker waits
// off-thread for the old camera handle (and virtual output) to be released.
static CAPTURE_BUSY: Mutex<bool> = Mutex::new(false);
static CAPTURE_CHANGED: Condvar = Condvar::new();

struct CapturePermit;

impl CapturePermit {
    fn wait(worker: &Worker) -> Option<Self> {
        let busy = CAPTURE_BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let mut busy = CAPTURE_CHANGED
            .wait_while(busy, |busy| *busy && !worker.should_stop())
            .unwrap_or_else(|e| e.into_inner());
        if worker.should_stop() {
            return None;
        }
        *busy = true;
        Some(Self)
    }
}

impl Drop for CapturePermit {
    fn drop(&mut self) {
        *CAPTURE_BUSY.lock().unwrap_or_else(|e| e.into_inner()) = false;
        CAPTURE_CHANGED.notify_all();
    }
}

/// A running capture session on one device. Dropping it stops the capture
/// thread and releases the camera.
pub struct Session {
    shared: Arc<Shared>,
    commands: Sender,
    stop: Arc<AtomicBool>,
}

impl Session {
    pub fn start(
        device: DeviceInfo,
        config: CaptureConfig,
        virtual_camera: bool,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        if virtual_camera {
            let _ = tx.send(Command::SetVirtualCamera(true));
        }
        let worker = Worker {
            shared: shared.clone(),
            commands: rx,
            stop: stop.clone(),
            repaint: Arc::new(repaint),
        };
        let thread = std::thread::Builder::new()
            .name("camera-capture".into())
            .spawn(move || {
                let Some(_permit) = CapturePermit::wait(&worker) else {
                    return;
                };
                let shared = worker.shared.clone();
                if let Err(message) = backend::run_capture(&device, config, &worker) {
                    shared.fail(message);
                } else {
                    shared.status().state = SessionState::Stopped;
                }
                (worker.repaint)();
            });
        if thread.is_err() {
            shared.fail("Could not start the capture thread".into());
        }
        Self {
            shared,
            commands: tx,
            stop,
        }
    }

    pub fn send(&self, command: Command) {
        if !self.commands.send(command) {
            let mut status = self.shared.status();
            if status.state != SessionState::Failed {
                status.error = Some("The camera could not accept another command".into());
            }
        }
    }

    pub fn status(&self) -> SessionStatus {
        self.shared.status().clone()
    }

    /// Reads the status and takes a frame from the same video epoch.
    pub fn status_and_preview(&self, take_frame: bool) -> (SessionStatus, Option<PreviewFrame>) {
        let mut status = self.shared.status();
        let frame = take_frame.then(|| self.shared.frame().take()).flatten();
        let current = status.clone();
        if status.state != SessionState::Failed {
            status.error = None;
        }
        (current, frame)
    }

    /// Takes the newest preview frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<PreviewFrame> {
        self.shared.frame().take()
    }

    /// Takes the full-size frame requested with [`Command::Snapshot`].
    pub fn take_snapshot(&self) -> Option<PreviewFrame> {
        self.shared.snapshot().take()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _busy = CAPTURE_BUSY.lock().unwrap_or_else(|e| e.into_inner());
        self.stop.store(true, Ordering::Relaxed);
        CAPTURE_CHANGED.notify_all();
    }
}

/// Lists video capture devices, OBSBOT cameras first.
pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    let mut devices = backend::list_devices()?;
    devices.sort_by_key(|d| !d.is_obsbot());
    Ok(devices)
}

/// Lists the native formats of `device` without streaming. Fails while
/// another app streams the camera on some systems.
pub fn list_modes(device: &DeviceInfo) -> Result<Vec<VideoMode>, String> {
    backend::list_modes(device)
}

/// Lists the camera-terminal controls the camera answers, one line each.
pub fn probe_controls(device: &DeviceInfo, script: Vec<String>) -> Result<Vec<String>, String> {
    backend::probe_controls(device, script)
}

/// Extracts the USB vendor and product IDs from an OS device path such as
/// `\\?\usb#vid_3564&pid_ff04&mi_00#...`.
#[cfg(any(windows, test))]
pub fn parse_usb_ids(path: &str) -> (Option<u16>, Option<u16>) {
    let lower = path.to_ascii_lowercase();
    let hex_after = |key: &str| {
        let start = lower.find(key)? + key.len();
        let digits = lower.get(start..start + 4)?;
        u16::from_str_radix(digits, 16).ok()
    };
    (hex_after("vid_"), hex_after("pid_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_and_resume_discard_queued_and_in_flight_preview_frames() {
        let shared = Shared::default();
        let frame = || PreviewFrame {
            size: [1, 1],
            pixels: vec![eframe::egui::Color32::RED],
        };
        let before_sleep = shared.status().video_epoch;
        assert!(shared.publish_preview(before_sleep, frame()));
        assert!(shared.set_sleeping(true));
        let asleep = shared.status().video_epoch;
        assert!(shared.frame().is_none());
        assert!(!shared.publish_preview(asleep, frame()));

        assert!(shared.set_sleeping(false));
        let awake = shared.status().video_epoch;
        assert!(!shared.publish_preview(before_sleep, frame()));
        assert!(!shared.publish_preview(asleep, frame()));
        assert!(shared.frame().is_none());
        assert!(shared.publish_preview(awake, frame()));
        assert!(!shared.set_sleeping(false));
        assert!(shared.frame().is_some());
    }

    #[test]
    fn coalesce_keeps_newest_values_in_order() {
        use GimbalCommand::*;
        let speed = |yaw| Command::Gimbal(Speed { pitch: 0.0, yaw });
        let zoom = |r| Command::Gimbal(ZoomRatio(r));
        let batch = vec![
            Command::Gimbal(Drag(true)),
            speed(1.0),
            zoom(1.5),
            speed(2.0),
            zoom(2.0),
            zoom(2.5),
            Command::Gimbal(Drag(false)),
            zoom(3.0),
            Command::SetSleep(true),
            zoom(3.5),
            zoom(4.0),
        ];
        assert_eq!(
            {
                let (tx, rx) = channel();
                for command in batch {
                    assert!(tx.send(command));
                }
                Vec::from(rx.drain())
            },
            vec![
                Command::Gimbal(Drag(true)),
                speed(2.0),
                zoom(2.5),
                Command::Gimbal(Drag(false)),
                zoom(3.0),
                Command::SetSleep(true),
                zoom(4.0),
            ]
        );
    }

    #[test]
    fn parses_usb_ids_from_symbolic_link() {
        let link = r"\\?\usb#vid_3564&pid_ff04&mi_00#7&245ef964&1&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global";
        assert_eq!(parse_usb_ids(link), (Some(0x3564), Some(0xff04)));
        assert_eq!(parse_usb_ids(r"\\?\root#camera#0000#{...}"), (None, None));
        assert_eq!(parse_usb_ids("vid_12"), (None, None));
    }

    #[test]
    fn zoom_snaps_to_steps() {
        let range = ZoomRange {
            min: 100,
            max: 400,
            step: 10,
            default: 100,
        };
        assert_eq!(range.snap(0), 100);
        assert_eq!(range.snap(1000), 400);
        assert_eq!(range.snap(124), 120);
        assert_eq!(range.snap(125), 130);
        let unit = ZoomRange { step: 1, ..range };
        assert_eq!(unit.snap(123), 123);
        let wide = ZoomRange {
            min: i32::MIN,
            max: i32::MAX,
            step: 2,
            default: 0,
        };
        assert_eq!(wide.snap(i32::MAX), i32::MAX);
    }
}
