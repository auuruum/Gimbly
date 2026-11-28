//! Camera discovery, capture and control.
//!
//! The shared part of this module defines the session API used by the UI. The
//! OS-specific backends provide Media Foundation, V4L2 and AVFoundation.

mod control;
mod frames;
mod mjpeg;
mod models;
mod preview;
mod queue;
mod resume;
mod serial;
mod sleep;
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
pub use models::{Feature, Model};
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

    /// The model when Gimbly drives this camera's vendor controls; other
    /// cameras get video only.
    pub fn model(&self) -> Option<&'static Model> {
        Model::find(self.usb_product_id?).filter(|_| self.is_obsbot())
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
    /// Keep the camera's own JPEG frames for High-fps Recording
    /// ([`Command::Record`]); the session then needs an MJPEG mode.
    pub compressed: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            portrait: false,
            transform: Transform::default(),
            compressed: false,
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
    /// The OBSBOT model Gimbly drives; `None` for other cameras.
    pub model: Option<&'static Model>,
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
    /// AI mode the camera reports running; `None` until read.
    pub ai_mode: Option<AiMode>,
    /// Mode Gimbly last turned Human Tracking on in. The camera reports the
    /// same state for both modes, so tracking Gimbly did not start (such as
    /// the Locked Target gesture) shows as Single Mode.
    pub tracking_mode: Option<TrackingMode>,
    /// Auto Zoom framing the camera reports; `None` until read.
    pub auto_zoom: Option<AutoZoom>,
    /// Tracking speed the camera reports; `None` until read.
    pub tracking_speed: Option<TrackingSpeed>,
    /// Voice Tracking Range the camera reports; `None` until read.
    pub voice_range: Option<VoiceRange>,
    /// Sound Source Assisted Tracking the camera reports; `None` until read.
    pub sound_source: Option<bool>,
    /// HDR the camera reports; `None` until read.
    pub hdr: Option<bool>,
    /// Frame rate the camera streams at; `None` until read.
    pub frame_rate: Option<u8>,
    /// Focus settings the camera reports; `None` until read.
    pub focus: Option<Focus>,
    /// Auto Exposure, the manual shutter and ISO, compensation and the ISO
    /// limits; `None` until read.
    pub exposure: Option<Exposure>,
    /// What Auto Exposure meters on; `None` until read.
    pub exposure_mode: Option<FocusMode>,
    /// The manual exposure curve points; `None` until read.
    pub exposure_curve: Option<Vec<[u8; 2]>>,
    /// The Anti-Flicker setting the camera reports; `None` until read.
    pub anti_flicker: Option<AntiFlicker>,
    /// White balance settings the camera reports; `None` until read.
    pub white_balance: Option<WhiteBalance>,
    /// The Image section the camera reports; `None` until read.
    pub image: Option<ImageAdjust>,
    /// Device Sleep settings the camera reports; `None` until read.
    pub device_sleep: Option<DeviceSleep>,
    /// The Locked Target gesture the camera reports; `None` until read.
    pub locked_target: Option<bool>,
    /// Voice Control settings the camera reports; `None` until read.
    pub voice_control: Option<VoiceControl>,
    /// Audio settings the camera reports; `None` until read.
    pub audio: Option<Audio>,
    /// The hand Hand Tracking follows; `None` until read.
    pub hand: Option<Hand>,
    /// The Hand Tracking range being edited. `Some` exactly while the camera
    /// is in its range-edit state.
    pub hand_range: Option<TrackingRange>,
    /// A return position outside the range, waiting for the user to widen
    /// the range to it (see [`TrackingRange::including_return`]).
    pub hand_range_proposal: Option<TrackingRange>,
    /// The Zone Tracking switch the camera reports; `None` until read.
    pub zone_tracking: Option<bool>,
    /// The Zone Tracking settings being edited. `Some` exactly while the
    /// camera is in its zone-edit state.
    pub zone: Option<Zone>,
    /// A change to the zone range waiting for the user's OK: `true` widens
    /// the range to a new return position, `false` moves the return
    /// position inside new limits.
    pub zone_proposal: Option<(TrackingRange, bool)>,
    /// Object tracking runs, framed like this; `None` when it does not.
    pub object_framing: Option<ObjectFraming>,
    /// The box of the tracked target, streamed by the camera; `None` when
    /// nothing is tracked or the stream is unavailable.
    pub target: Option<TargetBox>,
    /// The camera's serial port, which carries [`Self::target`], could not
    /// be opened for lack of permission (Linux). Cleared once it opens.
    pub serial_denied: Option<SerialDenied>,
    /// The AI Composition settings, read when the composition mode opens.
    pub composition: Option<Composition>,
    /// Stored preset positions, in the camera's order. `None` until read.
    pub presets: Option<Vec<Preset>>,
    /// High-fps Recording runs in this session.
    pub recording: Option<RecordingStatus>,
    /// The virtual camera has received at least one frame in this session.
    pub virtual_camera: bool,
    pub virtual_camera_error: Option<String>,
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
    /// Start High-fps Recording in a [`CaptureConfig::compressed`] session.
    /// The recording ends with the session.
    Record(crate::record::Spec),
    /// Pause (`true`) or resume the recording.
    PauseRecording(bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordingStatus {
    /// Real time recorded, without pauses.
    pub elapsed: std::time::Duration,
    pub paused: bool,
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
    /// Return to the initial position and zoom to 1.0x, as the Hand
    /// Tracking range panel's Reset does.
    ResetWithZoom,
    /// Turn to absolute angles in degrees, in the camera's sign; `None`
    /// keeps that axis.
    MoveTo {
        pitch: Option<f32>,
        yaw: Option<f32>,
    },
    /// Zoom to a ratio between 1.0 and [`ZOOM_RATIO_MAX`].
    ZoomRatio(f32),
    SetFov(Fov),
    /// Turn Human Tracking on in a mode, or off with `None`.
    SetHumanTracking(Option<TrackingMode>),
    SetAutoZoom(AutoZoom),
    SetTrackingSpeed(TrackingSpeed),
    /// Start Voice or Hand Tracking, or stop the running AI mode with `Off`.
    SetAiMode(AiMode),
    SetVoiceRange(VoiceRange),
    SetSoundSource(bool),
    SetHdr(bool),
    /// Auto Focus on, or off at the manual focus position.
    SetAutoFocus(bool),
    SetFocusMode(FocusMode),
    /// Manual focus position, 0 to [`FOCUS_MAX`].
    SetFocusPosition(u8),
    /// Auto Exposure on, or off at the camera's stored manual shutter.
    SetAutoExposure(bool),
    /// What Auto Exposure meters on (the same Global/Face choice as focus).
    SetExposureMode(FocusMode),
    /// Manual shutter in the camera's units of 100 µs (see [`SHUTTER_SPEEDS`]).
    SetShutter(u32),
    /// Manual ISO, 100 to [`ISO_MAX`].
    SetIso(u16),
    /// Exposure compensation, 0 to [`COMPENSATION_MAX`] (thirds of a stop; 9 is 0 EV).
    SetCompensation(u8),
    /// The ISO range Auto Exposure stays in, from [`ISO_LIMITS`].
    SetIsoLimits(u16, u16),
    /// Manual exposure curve points between the fixed ends (0, 0) and (255, 255).
    SetExposureCurve(Vec<[u8; 2]>),
    SetAntiFlicker(AntiFlicker),
    /// The whole White Balance section; the camera takes it in one message.
    SetWhiteBalance(WhiteBalance),
    /// The Image section; only the values that changed are written.
    SetImage(ImageAdjust),
    /// Auto Sleep and its time in seconds; negative while Auto Sleep is off.
    SetSleepTime(i16),
    /// The picture shown while asleep, below [`SLEEP_BACKGROUNDS`].
    SetSleepBackground(u8),
    SetSleepMirror(bool),
    SetGesture(Gesture, bool),
    /// The ratio the Zoom gesture zooms to, in [`GESTURE_ZOOM`].
    SetGestureZoom(f32),
    SetVoiceCommand(VoiceCommand, bool),
    /// "Hi, Tiny" also wakes the camera and turns it to the voice.
    SetVoiceLocator(bool),
    /// The language the camera listens for: English or Chinese.
    SetVoiceEnglish(bool),
    /// The voice Zoom Factor in steps of [`VOICE_ZOOM_STEP`], 0 to [`VOICE_ZOOM_MAX`].
    SetVoiceZoom(u8),
    SetAudioInput(AudioInput),
    SetAudioMode(AudioMode),
    /// Keep the microphone working while the camera sleeps.
    SetSleepMic(bool),
    /// Turn the microphone on or off; the camera restarts.
    SetMicrophone(bool),
    /// Start pairing Vox Wireless Mic transmitter 1 or 2, or end it with `None`.
    PairVox(Option<u8>),
    SetHand(Hand),
    /// Read the Hand Tracking range and put the camera in its edit state.
    OpenHandRange,
    /// Take one range value from the current gimbal position (and zoom).
    UpdateHandRange(RangeField),
    /// Put the four limits (`limits`) or the return position back to the
    /// defaults in the panel; nothing is written until Done.
    ResetHandRange {
        limits: bool,
    },
    /// Answer the question in [`Status::hand_range_proposal`]: `true`
    /// widens the range to take the new return position.
    ConfirmHandRange(bool),
    /// Leave the edit state, writing the edited range when `save` is set.
    CloseHandRange {
        save: bool,
    },
    SetZoneTracking(bool),
    /// Read the Zone Tracking settings and put the camera in its edit state.
    OpenZone,
    /// Take one zone range value from the current gimbal position (and zoom).
    UpdateZone(RangeField),
    /// Put the limits (`limits`, the camera's defaults) or the return
    /// position back in the panel; nothing is written until Done.
    ResetZone {
        limits: bool,
    },
    /// Answer the question in [`SessionStatus::zone_proposal`].
    ConfirmZone(bool),
    SetZoneAutoTarget(bool),
    /// Store the current view as a Non-tracking Area in the first free slot.
    AddArea,
    /// Replace an area's view with the current one.
    UpdateArea(u8),
    RenameArea {
        id: u8,
        name: String,
    },
    DeleteArea(u8),
    /// Leave the edit state, writing the edited settings when `save` is set.
    CloseZone {
        save: bool,
    },
    /// Track the person at this point (fractions of the camera picture),
    /// as a double-click in the preview does.
    SelectPoint {
        x: f32,
        y: f32,
    },
    /// Start object tracking on this box.
    SelectObject(TargetBox),
    /// Stop tracking the current target.
    CancelTarget,
    SetObjectFraming(ObjectFraming),
    /// Read the AI Composition settings into [`SessionStatus::composition`].
    ReadComposition,
    /// Move the subject's place in the picture; see [`Composition`].
    SetComposition {
        x: f32,
        y: f32,
    },
    SetFaceFraming(bool),
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

/// The two Human Tracking buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingMode {
    Single,
    Group,
}

/// How Auto Zoom frames the tracked person (the Auto Zoom buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoZoom {
    Off,
    FullBody,
    UpperBody,
    CloseUp,
    Headless,
    LowerBody,
}

impl AutoZoom {
    pub const ALL: [AutoZoom; 6] = [
        Self::Off,
        Self::FullBody,
        Self::UpperBody,
        Self::CloseUp,
        Self::Headless,
        Self::LowerBody,
    ];

    /// The official app disables these in Group Mode.
    pub fn single_only(self) -> bool {
        matches!(self, Self::CloseUp | Self::Headless | Self::LowerBody)
    }
}

/// How fast AI tracking turns the gimbal (the Tracking Speed row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingSpeed {
    Slow,
    Standard,
    Fast,
}

impl TrackingSpeed {
    pub const ALL: [TrackingSpeed; 3] = [Self::Slow, Self::Standard, Self::Fast];
}

/// How object tracking frames the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFraming {
    Normal,
    CloseUp,
}

/// A serial port the user may not open: its path and owning group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialDenied {
    pub path: String,
    pub group: Option<String>,
}

/// A box in fractions of the camera picture, from its top left corner,
/// before Gimbly's rotation, mirror and portrait crop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl TargetBox {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }
}

/// Where Human Tracking places the person: the offset from the picture
/// centre in fractions of the picture, within ±[`COMPOSITION_LIMIT`]
/// (positive is right and down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Composition {
    pub x: f32,
    pub y: f32,
    /// Face Framing: the camera picks the horizontal place itself.
    pub face_framing: bool,
}

pub use vendor::clamp_composition;

/// The AI mode the camera runs; only one runs at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMode {
    Off,
    Human,
    Hand,
    Voice,
}

/// The highest manual focus position (UVC focus range of the Tiny 3 Lite).
pub const FOCUS_MAX: u8 = 100;

/// Manual shutter speeds the official app offers on the Tiny 3 Lite at
/// 60 fps, slowest first: the denominator shown (1/x s) and the value sent,
/// in units of 100 µs.
pub const SHUTTER_SPEEDS: [(u16, u32); 20] = [
    (60, 167),
    (80, 125),
    (100, 100),
    (120, 83),
    (160, 63),
    (200, 50),
    (240, 42),
    (320, 31),
    (400, 25),
    (500, 20),
    (640, 16),
    (800, 13),
    (1000, 10),
    (1250, 8),
    (1600, 6),
    (2000, 5),
    (2500, 4),
    (3200, 3),
    (5000, 2),
    (6400, 1),
];

/// The highest manual ISO (the UVC gain is ISO / 100).
pub const ISO_MAX: u16 = 6400;
/// The steps of the Auto Exposure ISO limits.
pub const ISO_LIMITS: [u16; 8] = [100, 200, 400, 800, 1600, 3200, 6400, 12800];
/// The highest compensation value (+3 EV); 9 is 0 EV, each step a third of a stop.
pub const COMPENSATION_MAX: u8 = 18;
pub const COMPENSATION_ZERO: u8 = 9;

/// Exposure settings read from the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exposure {
    pub auto: bool,
    /// Manual shutter in units of 100 µs, kept while Auto Exposure is on.
    pub shutter: u32,
    /// Manual ISO, kept while Auto Exposure is on.
    pub iso: u16,
    pub compensation: u8,
    /// Auto Exposure's ISO range (lower, upper).
    pub iso_limits: (u16, u16),
}

/// The manual color temperatures in kelvin, in steps of [`WB_TEMPERATURE_STEP`].
pub const WB_TEMPERATURE: std::ops::RangeInclusive<u16> = 2000..=10000;
pub const WB_TEMPERATURE_STEP: u16 = 100;
/// The White Balance Shift runs from -7 to 7 in quarters: 0 to 56, 28 is 0.
pub const WB_SHIFT_MAX: u8 = 56;
pub const WB_SHIFT_ZERO: u8 = 28;

/// White balance settings read from the camera. The manual values are kept
/// while Auto WB is on, and the shift while it is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WhiteBalance {
    pub auto: bool,
    /// Manual color temperature in kelvin, in [`WB_TEMPERATURE`].
    pub temperature: u16,
    /// Manual by red and blue gains instead of the temperature.
    pub gains: bool,
    pub red: u8,
    pub blue: u8,
    /// Auto WB's shift: B-A (blue to amber) and G-M (green to magenta),
    /// each 0 to [`WB_SHIFT_MAX`].
    pub shift: [u8; 2],
}

/// The Image section's values all run from 0 to this; [`IMAGE_DEFAULT`] is the middle.
pub const IMAGE_MAX: u8 = 100;
pub const IMAGE_DEFAULT: ImageAdjust = ImageAdjust {
    contrast: 50,
    saturation: 50,
    sharpness: 50,
    hue: 50,
};

/// Contrast, saturation, sharpness and hue, each 0 to [`IMAGE_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageAdjust {
    pub contrast: u8,
    pub saturation: u8,
    pub sharpness: u8,
    pub hue: u8,
}

impl ImageAdjust {
    /// The values in the order the section shows them.
    pub fn values(self) -> [u8; 4] {
        [self.contrast, self.saturation, self.sharpness, self.hue]
    }

    pub fn from_values([contrast, saturation, sharpness, hue]: [u8; 4]) -> Self {
        Self {
            contrast,
            saturation,
            sharpness,
            hue,
        }
    }
}

/// The Sleep Time choices in seconds.
pub const SLEEP_TIMES: [i16; 3] = [30, 120, 600];
/// The number of built-in sleep pictures.
pub const SLEEP_BACKGROUNDS: u8 = 4;

/// The user's own picture shown while the camera sleeps, instead of the
/// Sleep Background tile. Shared with every capture session.
static CUSTOM_SLEEP: Mutex<Option<Arc<eframe::egui::IconData>>> = Mutex::new(None);

pub fn set_custom_sleep_picture(picture: Option<Arc<eframe::egui::IconData>>) {
    *CUSTOM_SLEEP.lock().unwrap_or_else(|e| e.into_inner()) = picture;
}

fn custom_sleep_picture() -> Option<Arc<eframe::egui::IconData>> {
    CUSTOM_SLEEP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The Device Sleep section as the camera reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceSleep {
    /// Seconds without a video stream before the camera sleeps; negative
    /// (the same time) while Auto Sleep is off.
    pub time: i16,
    /// The picture the camera streams while asleep, below [`SLEEP_BACKGROUNDS`].
    pub background: u8,
    pub mirror: bool,
}

impl DeviceSleep {
    pub fn auto_sleep(self) -> bool {
        self.time > 0
    }
}

/// The Gesture Control switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    LockedTarget,
    Zoom,
    DynamicZoom,
    DirectionFlip,
}

/// The Zoom gesture's ratio range and default.
pub const GESTURE_ZOOM: std::ops::RangeInclusive<f32> = 1.0..=4.0;
pub const GESTURE_ZOOM_DEFAULT: f32 = 2.0;

/// The Voice Control commands in the official app's order. The value is
/// the camera's key and the bit in the status block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceCommand {
    HiTiny = 0,
    SleepTiny = 1,
    TrackMe = 2,
    UnlockMe = 3,
    ZoomIn = 4,
    ZoomOut = 5,
    Positions = 6,
}

impl VoiceCommand {
    pub const ALL: [Self; 7] = [
        Self::HiTiny,
        Self::SleepTiny,
        Self::TrackMe,
        Self::UnlockMe,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::Positions,
    ];
}

/// The voice Zoom Factor is 1.0 plus this many steps of [`VOICE_ZOOM_STEP`].
pub const VOICE_ZOOM_MAX: u8 = 100;
pub const VOICE_ZOOM_STEP: f32 = 0.03;

/// The Voice Control section as the camera reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceControl {
    pub english: bool,
    /// One bit per [`VoiceCommand`].
    pub commands: u8,
    pub locator: bool,
    /// Steps above 1.0x, 0 to [`VOICE_ZOOM_MAX`].
    pub zoom: u8,
}

impl VoiceControl {
    pub fn on(self, command: VoiceCommand) -> bool {
        self.commands & (1 << command as u8) != 0
    }

    pub fn set(&mut self, command: VoiceCommand, on: bool) {
        let bit = 1 << command as u8;
        self.commands = if on {
            self.commands | bit
        } else {
            self.commands & !bit
        };
    }

    pub fn zoom_ratio(self) -> f32 {
        1.0 + f32::from(self.zoom.min(VOICE_ZOOM_MAX)) * VOICE_ZOOM_STEP
    }
}

/// Where the camera's audio comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioInput {
    Camera,
    /// OBSBOT's Vox Wireless Mic receiver.
    Vox,
}

/// How the built-in microphones are processed. The value is the camera's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioMode {
    SmartOmni = 0,
    Spatial = 1,
    Directional = 2,
    DualDirectional = 4,
    Pure = 5,
}

impl AudioMode {
    /// In the official app's order.
    pub const ALL: [Self; 5] = [
        Self::Pure,
        Self::Spatial,
        Self::SmartOmni,
        Self::Directional,
        Self::DualDirectional,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Pure => "Pure Audio Mode",
            Self::Spatial => "Spatial Audio Mode",
            Self::SmartOmni => "Smart Omni Mode",
            Self::Directional => "Directional Mode",
            Self::DualDirectional => "Dual-Directional Mode",
        }
    }
}

/// The Audio section as the camera reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Audio {
    pub input: AudioInput,
    pub mode: AudioMode,
    /// Microphone Status During Sleep.
    pub sleep_mic: bool,
    /// The microphone is on (Disable Microphone is off).
    pub microphone: bool,
}

/// Anti-Flicker: the mains frequency exposure avoids beating with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AntiFlicker {
    Off,
    Hz50,
    Hz60,
}

/// What Auto Focus looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMode {
    /// The whole picture.
    Global,
    /// Faces first.
    Face,
}

/// The Focus settings in the status block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focus {
    pub auto: bool,
    pub mode: FocusMode,
    /// Manual focus position, kept while Auto Focus is on.
    pub position: u8,
}

/// Which directions Voice Tracking listens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceRange {
    Omni,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

/// The Hand Tracking or Zone Tracking range as the camera stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackingRange {
    /// Degrees in the camera's sign: yaw min, yaw max, pitch min, pitch max.
    pub limits: [f32; 4],
    /// Where the gimbal returns when the target leaves the range, in the
    /// 24-byte preset record layout.
    pub record: [u8; 24],
}

impl TrackingRange {
    /// The Hand Tracking panel's defaults; the camera reported these before
    /// any edit.
    pub const DEFAULT_LIMITS: [f32; 4] = [-45.0, 45.0, -30.0, 30.0];

    /// This range with its return position moved just inside the limits,
    /// or `None` when it already lies within them. The zoom is kept.
    pub fn return_within(&self) -> Option<TrackingRange> {
        let (yaw, pitch, ratio) = self.return_position();
        let [yaw_min, yaw_max, pitch_min, pitch_max] = self.limits;
        // max then min, not clamp: limits read from the camera may be crossed.
        let inside = (
            yaw.max(yaw_min).min(yaw_max),
            pitch.max(pitch_min).min(pitch_max),
        );
        if inside == (yaw, pitch) {
            return None;
        }
        let hundredths = |degrees: f32| (degrees * 100.0).round() as i16;
        let angles = (hundredths(inside.0), hundredths(inside.1));
        let record = vendor::set_return_position(&self.record, angles, ratio)?;
        Some(TrackingRange { record, ..*self })
    }

    /// This range widened just enough to hold its return position, or
    /// `None` when the position already lies within it.
    pub fn including_return(&self) -> Option<TrackingRange> {
        let (yaw, pitch, _) = self.return_position();
        let [yaw_min, yaw_max, pitch_min, pitch_max] = self.limits;
        let limits = [
            yaw_min.min(yaw),
            yaw_max.max(yaw),
            pitch_min.min(pitch),
            pitch_max.max(pitch),
        ];
        (limits != self.limits).then_some(TrackingRange { limits, ..*self })
    }

    /// Yaw and pitch in degrees and the zoom ratio of the return position.
    pub fn return_position(&self) -> (f32, f32, f32) {
        vendor::return_position(&self.record)
    }
}

/// A value of a tracking range that Update can set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeField {
    /// Index into [`TrackingRange::limits`].
    Limit(usize),
    Return,
}

/// The Zone Tracking settings, read when its panel opens and written by
/// Done.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub range: TrackingRange,
    /// The return record the camera gives as its default; the return
    /// position's reset icon puts it back.
    pub default_record: [u8; 24],
    /// "Auto-tracking new target"; written as soon as it changes.
    pub auto_target: bool,
    /// Non-tracking areas, by id.
    pub areas: Vec<Area>,
    /// Yaw and pitch of the camera's initial position, where the
    /// Non-tracking Area Reset turns; `None` when it could not be read.
    pub initial: Option<(f32, f32)>,
}

/// A Non-tracking Area: a stored view the frame stays fixed at.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub id: u8,
    pub name: String,
    /// The 24-byte preset record layout, like [`TrackingRange::record`].
    pub record: [u8; 24],
}

impl Area {
    /// Yaw and pitch in degrees and the zoom ratio.
    pub fn position(&self) -> (f32, f32, f32) {
        vendor::return_position(&self.record)
    }
}

/// Number of Non-tracking Area slots on the Tiny 3 Lite.
pub const ZONE_AREAS: usize = 3;

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
        if status.video_epoch != epoch {
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
        status.virtual_camera = false;
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
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    pub fn start(
        device: DeviceInfo,
        config: CaptureConfig,
        virtual_camera: bool,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        shared.status().model = device.model();
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
                    let mut status = shared.status();
                    status.state = SessionState::Stopped;
                    status.virtual_camera = false;
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
            thread: thread.ok(),
        }
    }

    /// Stops the capture and waits up to `limit` for the camera thread to
    /// end, which also finishes a running recording. Dropping a session
    /// does not wait.
    pub fn close(self, limit: std::time::Duration) {
        let until = std::time::Instant::now() + limit;
        {
            let _busy = CAPTURE_BUSY.lock().unwrap_or_else(|e| e.into_inner());
            self.stop.store(true, Ordering::Relaxed);
            CAPTURE_CHANGED.notify_all();
        }
        while self.thread.as_ref().is_some_and(|t| !t.is_finished())
            && std::time::Instant::now() < until
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
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
        status.virtual_camera_error = None;
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
    fn vendor_controls_only_for_listed_obsbot_models() {
        let device = |vendor, product| DeviceInfo {
            name: String::new(),
            id: String::new(),
            usb_vendor_id: vendor,
            usb_product_id: product,
        };
        assert_eq!(
            device(Some(OBSBOT_USB_VENDOR_ID), Some(0xff04))
                .model()
                .map(|m| m.name),
            Some("Tiny 3 Lite")
        );
        // Another OBSBOT model, a non-OBSBOT camera with the same product ID,
        // and a camera without USB IDs get video only.
        assert_eq!(
            device(Some(OBSBOT_USB_VENDOR_ID), Some(0x0001)).model(),
            None
        );
        assert_eq!(device(Some(0x046d), Some(0xff04)).model(), None);
        assert_eq!(device(None, None).model(), None);
    }

    #[test]
    fn return_outside_the_range_widens_only_the_crossed_limits() {
        // Observed 2026-11-22: return P -55.60° T 30.20° with the default
        // range turned the right limit into -55.6° and the top into 30.2°.
        let inside =
            vendor::set_return_position(&vendor::RETURN_RECORD_NONE, (4490, 3000), 1.42).unwrap();
        let mut range = TrackingRange {
            limits: TrackingRange::DEFAULT_LIMITS,
            record: inside,
        };
        assert_eq!(range.including_return(), None);
        range.record = vendor::set_return_position(&range.record, (-5560, -3020), 1.42).unwrap();
        let wider = range.including_return().unwrap();
        assert_eq!(wider.limits, [-55.6, 45.0, -30.2, 30.0]);
        assert_eq!(wider.record, range.record);
        assert_eq!(wider.including_return(), None);
    }

    #[test]
    fn zone_limit_moves_the_return_position_inside() {
        // Observed 2026-11-24: the right limit updated to 13.4° with return
        // P 0.58° T 0° Z 1.00 gave return P 13.40°, T and Z unchanged.
        let record = vendor::set_return_position(&[0; 24], (58, 0), 1.0).unwrap();
        let range = TrackingRange {
            limits: [13.4, 100.0, -30.0, 30.0],
            record,
        };
        let moved = range.return_within().unwrap();
        assert_eq!(moved.return_position(), (13.4, 0.0, 1.0));
        assert_eq!(moved.limits, range.limits);
        assert_eq!(moved.return_within(), None);
        // Crossed limits from a bad read must not panic.
        let crossed = TrackingRange {
            limits: [10.0, -10.0, 5.0, -5.0],
            record,
        };
        assert!(crossed.return_within().is_some());
    }

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
        // The sleeping camera's own picture is shown like any other frame.
        assert!(shared.publish_preview(asleep, frame()));

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
