//! Shared vendor protocol and command worker. Transports implement only UVC
//! reads and writes. Only this worker waits for the camera to answer.

use super::queue::Receiver;
use super::sleep::SleepReadback;
use super::{
    AiMode, AntiFlicker, Area, AutoZoom, COMPENSATION_MAX, Command, Composition, Exposure,
    FOCUS_MAX, Feature, Focus, GimbalCommand, Hand, IMAGE_MAX, ISO_MAX, ImageAdjust, ObjectFraming,
    PRESET_SLOTS, Preset, RangeField, Shared, TrackingMode, TrackingRange, TrackingSpeed,
    VoiceRange, WhiteBalance, ZONE_AREAS, Zone, serial, vendor,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) trait Transport {
    fn read(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<usize, String>;
    fn write(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<(), String>;
    /// Standard UVC Auto Focus on the camera terminal (`CT_FOCUS_AUTO_CONTROL`).
    fn set_auto_focus(&mut self, _on: bool) -> Result<(), String> {
        Err("Focus is not supported on this system".into())
    }
    /// Standard UVC focus position (`CT_FOCUS_ABSOLUTE_CONTROL`).
    fn set_focus_position(&mut self, _position: u8) -> Result<(), String> {
        Err("Focus is not supported on this system".into())
    }
    /// Reads a standard UVC processing unit control.
    fn get_processing(&mut self, _control: Processing) -> Result<i32, String> {
        Err("Exposure is not supported on this system".into())
    }
    /// Writes a standard UVC processing unit control.
    fn set_processing(&mut self, _control: Processing, _value: i32) -> Result<(), String> {
        Err("Exposure is not supported on this system".into())
    }
}

/// The UVC processing unit controls the Exposure and Image sections use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Processing {
    /// `PU_BACKLIGHT_COMPENSATION_CONTROL` (1): exposure compensation, 0 to 18.
    Backlight,
    /// `PU_GAIN_CONTROL` (4): manual ISO / 100.
    Gain,
    /// `PU_POWER_LINE_FREQUENCY_CONTROL` (5): Anti-Flicker, 0 off, 1 50 Hz, 2 60 Hz.
    PowerLine,
    /// `PU_CONTRAST_CONTROL` (3), 0 to 100.
    Contrast,
    /// `PU_HUE_CONTROL` (6), 0 to 100.
    Hue,
    /// `PU_SATURATION_CONTROL` (7), 0 to 100.
    Saturation,
    /// `PU_SHARPNESS_CONTROL` (8), 0 to 100.
    Sharpness,
}

/// The Image section's controls in [`ImageAdjust::values`] order.
const IMAGE_CONTROLS: [Processing; 4] = [
    Processing::Contrast,
    Processing::Saturation,
    Processing::Sharpness,
    Processing::Hue,
];

/// An Image value read from the camera, or `None` past the range.
fn image_value(read: i32) -> Option<u8> {
    u8::try_from(read).ok().filter(|&v| v <= IMAGE_MAX)
}

impl Processing {
    /// The UVC control selector.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn selector(self) -> u8 {
        match self {
            Self::Backlight => 1,
            Self::Gain => 4,
            Self::PowerLine => 5,
            Self::Contrast => 3,
            Self::Hue => 6,
            Self::Saturation => 7,
            Self::Sharpness => 8,
        }
    }
}

pub(super) struct Channel<T> {
    transport: T,
    sender: vendor::Sender,
    last_write: Option<Instant>,
}

impl<T: Transport> Channel<T> {
    pub(super) fn open(transport: T) -> Result<Self, String> {
        let channel = Self {
            transport,
            sender: vendor::Sender::default(),
            last_write: None,
        };
        channel.get(vendor::SELECTOR_COMMAND)?;
        Ok(channel)
    }

    pub(super) fn initialize(&mut self, shared: &Shared) -> ZoomRefresh {
        let view = self.read_view();
        shared.set_sleeping(view.is_some_and(|view| view.sleeping));
        let ratio = self.read_zoom().ok();
        let presets = self.presets().ok();
        let tracking_speed = self.read_tracking_speed().ok();
        let hand = self.read_hand().ok();
        let zone_tracking = self.read_zone_tracking().ok();
        let exposure = self.read_exposure().ok();
        let exposure_curve = self.read_exposure_curve().ok();
        let white_balance = self.read_white_balance().ok();
        let image = self.read_image().ok();
        let mut status = shared.status();
        status.ai_mode = view.and_then(|view| view.ai_mode);
        status.auto_zoom = view.and_then(|view| view.auto_zoom);
        status.voice_range = view.and_then(|view| view.voice_range);
        status.sound_source = view.and_then(|view| view.sound_source);
        status.hdr = view.and_then(|view| view.hdr);
        status.frame_rate = view.and_then(|view| view.frame_rate);
        status.focus = view.and_then(|view| view.focus);
        status.exposure_mode = view.and_then(|view| view.exposure_mode);
        status.anti_flicker = view.and_then(|view| view.anti_flicker);
        status.exposure = exposure;
        status.exposure_curve = exposure_curve;
        status.white_balance = white_balance;
        status.image = image;
        status.device_sleep = view.and_then(|view| view.device_sleep);
        status.locked_target = view.and_then(|view| view.locked_target);
        status.voice_control = view.and_then(|view| view.voice_control);
        status.audio = view.and_then(|view| view.audio);
        status.object_framing = view.and_then(|view| view.object_framing);
        status.tracking_speed = tracking_speed;
        status.hand = hand;
        status.zone_tracking = zone_tracking;
        status.zoom_ratio = ratio;
        status.presets = presets;
        status.vendor_control = true;
        status.vendor_control_untested = false;
        ratio
            .is_none()
            .then(|| read_zoom_in(Duration::from_secs(1)))
            .flatten()
    }

    pub(super) fn send(&mut self, mut message: vendor::Message) -> Result<(), String> {
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
        let result = self.transport.write(selector, block);
        self.last_write = Some(Instant::now());
        result
    }

    pub(super) fn get(&self, selector: u32) -> Result<([u8; vendor::MESSAGE_LEN], usize), String> {
        let mut block = [0u8; vendor::MESSAGE_LEN];
        let len = self.transport.read(selector, &mut block)?;
        if len > block.len() {
            return Err("Camera returned an oversized extension unit block".into());
        }
        Ok((block, len))
    }

    pub(super) fn request(&mut self, command: [u8; 2], data: Vec<u8>) -> Result<Vec<u8>, String> {
        self.request_from(vendor::GIMBAL, command, data)
    }

    fn request_from(
        &mut self,
        destination: u8,
        command: [u8; 2],
        data: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        self.exchange(vendor::Message::new(destination, command, data).requesting_reply())
    }

    /// Sends `request` and waits for its reply's data.
    fn exchange(&mut self, mut request: vendor::Message) -> Result<Vec<u8>, String> {
        let command = request.command;
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

    /// Sleep and AI state from one read of the status block.
    pub(super) fn read_view(&self) -> Option<View> {
        let (block, len) = self.get(vendor::SELECTOR_VIEW).ok()?;
        let block = &block[..len];
        Some(View {
            sleeping: vendor::sleeping(block)?,
            ai_mode: vendor::read_ai_mode(block),
            auto_zoom: vendor::read_auto_zoom(block),
            voice_range: vendor::read_voice_range(block),
            sound_source: vendor::read_sound_source(block),
            hdr: vendor::read_hdr(block),
            frame_rate: vendor::read_frame_rate(block),
            focus: vendor::read_focus(block),
            exposure_mode: vendor::read_exposure_mode(block),
            anti_flicker: vendor::read_anti_flicker(block),
            object_framing: vendor::read_object_framing(block),
            device_sleep: vendor::read_device_sleep(block),
            locked_target: vendor::read_locked_target(block),
            voice_control: vendor::read_voice_control(block),
            audio: vendor::read_audio(block),
        })
    }

    /// Auto Exposure and the shutter (vendor), manual ISO and compensation
    /// (UVC), and the ISO limits (vendor).
    pub(super) fn read_exposure(&mut self) -> Result<Exposure, String> {
        let state = self.request_from(vendor::CAMERA, vendor::READ_EXPOSURE, Vec::new())?;
        let (auto, shutter) =
            vendor::read_exposure(&state).ok_or("Camera returned an unknown exposure state")?;
        let limits = self.request_from(vendor::CAMERA, vendor::READ_ISO_LIMITS, Vec::new())?;
        let iso_limits =
            vendor::read_iso_limits(&limits).ok_or("Camera returned unknown ISO limits")?;
        let gain = self.transport.get_processing(Processing::Gain)?;
        let iso = u16::try_from(gain.clamp(1, i32::from(ISO_MAX / 100)) * 100).unwrap_or(100);
        let compensation = self.transport.get_processing(Processing::Backlight)?;
        let compensation =
            u8::try_from(compensation.clamp(0, i32::from(COMPENSATION_MAX))).unwrap_or(0);
        Ok(Exposure {
            auto,
            shutter,
            iso,
            compensation,
            iso_limits,
        })
    }

    pub(super) fn read_white_balance(&mut self) -> Result<WhiteBalance, String> {
        let data = self.request_from(vendor::CAMERA, vendor::READ_WHITE_BALANCE, Vec::new())?;
        vendor::read_white_balance(&data)
            .ok_or_else(|| "Camera returned an unknown white balance".into())
    }

    /// Contrast, saturation, sharpness and hue (UVC). A sleeping camera
    /// answers garbage (`0x63c0`), so values past the range are refused.
    pub(super) fn read_image(&mut self) -> Result<ImageAdjust, String> {
        let mut values = [0; 4];
        for (value, control) in values.iter_mut().zip(IMAGE_CONTROLS) {
            let read = self.transport.get_processing(control)?;
            *value = image_value(read).ok_or("Camera returned an unknown image value")?;
        }
        Ok(ImageAdjust::from_values(values))
    }

    fn read_exposure_curve(&self) -> Result<Vec<[u8; 2]>, String> {
        let (block, len) = self.get(vendor::SELECTOR_CURVE)?;
        vendor::read_exposure_curve(&block[..len])
            .ok_or_else(|| "Camera returned an invalid exposure curve".into())
    }

    /// Sets the camera's target report for the serial port (see
    /// [`serial`]) the way the official app does: once when it connects,
    /// then after every tracking change. Best effort: the box is optional.
    fn target_report(&mut self, tracking: bool, connecting: bool) {
        if connecting {
            let (destination, command) = vendor::TARGET_STREAM_START;
            let _ = self.request_from(destination, command, Vec::new());
        }
        let Ok(reply) = self.exchange(vendor::read_report_settings()) else {
            return;
        };
        if let Some(message) = vendor::target_report(&reply, tracking) {
            let _ = self.send(message);
        }
    }

    fn read_setting(&mut self, key: u32) -> Result<Vec<u8>, String> {
        self.request(vendor::READ_SETTING, vendor::read_setting_request(key))
    }

    fn read_composition(&mut self) -> Result<Composition, String> {
        let invalid = || "Camera returned an invalid composition setting".to_string();
        let face = self.read_setting(vendor::SETTING_FACE_FRAMING)?;
        let x = self.read_setting(vendor::SETTING_COMPOSITION_X)?;
        let y = self.read_setting(vendor::SETTING_COMPOSITION_Y)?;
        Ok(Composition {
            x: vendor::read_composition(&x).ok_or_else(invalid)?,
            y: vendor::read_composition(&y).ok_or_else(invalid)?,
            face_framing: vendor::read_flag(&face).ok_or_else(invalid)?,
        })
    }

    fn read_hand(&mut self) -> Result<Hand, String> {
        let data = self.request(
            vendor::READ_HAND_SETTING,
            vendor::hand_setting_key(vendor::HAND_SIDE),
        )?;
        vendor::read_hand(&data).ok_or_else(|| "Camera returned an unknown hand".into())
    }

    /// Reads the Hand Tracking range, then enters the range-edit state.
    fn open_hand_range(&mut self, shared: &Shared) -> Result<(), String> {
        // The official app reads twice, first with data `ff`; the replies matched.
        let _ = self.request(vendor::READ_RETURN_POSITION, vec![0xff]);
        let record =
            vendor::read_return_record(&self.request(vendor::READ_RETURN_POSITION, Vec::new())?)
                .ok_or("Camera returned an invalid return position")?;
        let mut limits = [0.0; 4];
        for (key, limit) in (0u32..).zip(&mut limits) {
            let data = self.request(vendor::READ_HAND_SETTING, vendor::hand_setting_key(key))?;
            *limit =
                vendor::read_hand_limit(&data).ok_or("Camera returned an invalid range limit")?;
        }
        // The official app then reads keys 0-3 again with one more `00` byte
        // and gets the same replies; Gimbly skips that.
        self.set(vendor::SELECTOR_VIEW, &mut vendor::hand_range_edit(true))?;
        shared.status().hand_range = Some(TrackingRange { limits, record });
        self.send(vendor::hand_range_gimbal(true))
    }

    fn update_hand_range(&mut self, shared: &Shared, field: RangeField) -> Result<(), String> {
        let mut range = shared
            .status()
            .hand_range
            .ok_or("The Hand Tracking range is not open")?;
        let current = self.request(vendor::READ_POSITION, Vec::new())?;
        let angles =
            vendor::current_angles(&current).ok_or("Camera returned an invalid gimbal position")?;
        match field {
            RangeField::Limit(key) => {
                let (yaw, pitch) = angles;
                let value = f32::from(if key < 2 { yaw } else { pitch }) / 100.0;
                let mut limits = range.limits;
                *limits.get_mut(key).ok_or("Unknown range limit")? = value;
                if !vendor::hand_limits_valid(&limits) {
                    return Err("Keep at least 10° between opposite limits".into());
                }
                range.limits = limits;
            }
            RangeField::Return => {
                let ratio = self.read_zoom()?;
                range.record = vendor::set_return_position(&range.record, angles, ratio)
                    .ok_or("Camera returned an invalid zoom ratio")?;
                // Like the official app, ask before widening the range to it.
                if let Some(wider) = range.including_return() {
                    shared.status().hand_range_proposal = Some(wider);
                    return Ok(());
                }
            }
        }
        shared.status().hand_range = Some(range);
        Ok(())
    }

    /// The panel's two reset icons; they only change the values on screen.
    fn reset_hand_range(shared: &Shared, limits: bool) {
        let mut status = shared.status();
        if let Some(range) = &mut status.hand_range {
            if limits {
                range.limits = TrackingRange::DEFAULT_LIMITS;
            } else {
                range.record = vendor::RETURN_RECORD_NONE;
            }
        }
    }

    fn confirm_hand_range(shared: &Shared, widen: bool) {
        let mut status = shared.status();
        let proposal = status.hand_range_proposal.take();
        if widen && status.hand_range.is_some() {
            status.hand_range = proposal.or(status.hand_range);
        }
    }

    /// Leaves the range-edit state, writing the range when `save` is set.
    fn close_hand_range(&mut self, shared: &Shared, save: bool) -> Result<(), String> {
        let Some(range) = shared.status().hand_range else {
            return Ok(());
        };
        let mut close = || {
            self.set(vendor::SELECTOR_VIEW, &mut vendor::hand_range_edit(false))?;
            vendor::hand_range_close(save.then_some(&range))
                .into_iter()
                .try_for_each(|message| self.send(message))
        };
        let result = close();
        // Cleared only now: exiting waits for this.
        let mut status = shared.status();
        status.hand_range = None;
        status.hand_range_proposal = None;
        result
    }

    fn read_zone_tracking(&mut self) -> Result<bool, String> {
        let data = self.request(vendor::READ_SETTING, vendor::read_zone_tracking_request())?;
        vendor::read_zone_tracking(&data)
            .ok_or_else(|| "Camera returned an unknown Zone Tracking state".into())
    }

    fn read_zone_limits(&mut self, default: bool) -> Result<[f32; 4], String> {
        let mut limits = [0.0; 4];
        for (index, limit) in (0u32..).zip(&mut limits) {
            let data = self.request(
                vendor::READ_SETTING,
                vendor::read_zone_limit_request(index, default),
            )?;
            *limit =
                vendor::read_hand_limit(&data).ok_or("Camera returned an invalid range limit")?;
        }
        Ok(limits)
    }

    /// The current gimbal angles in hundredths of a degree, and the zoom ratio.
    fn current_view(&mut self) -> Result<((i16, i16), f32), String> {
        let current = self.request(vendor::READ_POSITION, Vec::new())?;
        let angles =
            vendor::current_angles(&current).ok_or("Camera returned an invalid gimbal position")?;
        Ok((angles, self.read_zoom()?))
    }

    /// Reads the Zone Tracking settings while entering the zone-edit state.
    fn open_zone(&mut self, shared: &Shared) -> Result<(), String> {
        let [enter, pause] = vendor::zone_edit(true);
        self.send(enter)?;
        // Just after connecting the camera sometimes left these reads
        // unanswered for a moment (2026-11-24); one retry covers it.
        let opened = self
            .read_zone(pause.clone())
            .or_else(|_| self.read_zone(pause));
        match opened {
            Ok(zone) => {
                shared.status().zone = Some(zone);
                Ok(())
            }
            Err(error) => {
                // Without a panel to close, leave the edit state now.
                for message in vendor::zone_edit(false) {
                    let _ = self.send(message);
                }
                Err(error)
            }
        }
    }

    /// The reads between [`vendor::zone_edit`]'s two messages, `pause`
    /// sent where the official app sent it.
    fn read_zone(&mut self, pause: vendor::Message) -> Result<Zone, String> {
        // Like the official app: the default record (data `ff`), then the stored one.
        // The first read once went unanswered; the default is known anyway.
        let default_record = self
            .request(vendor::READ_ZONE_RETURN, vec![0xff])
            .ok()
            .and_then(|data| vendor::read_return_record(&data))
            .unwrap_or(vendor::ZONE_RETURN_DEFAULT);
        let record =
            vendor::read_return_record(&self.request(vendor::READ_ZONE_RETURN, Vec::new())?)
                .ok_or("Camera returned an invalid return position")?;
        self.send(pause)?;
        let limits = self.read_zone_limits(false)?;
        let auto_target = self.request(
            vendor::READ_SETTING,
            vendor::read_zone_auto_target_request(),
        )?;
        let auto_target =
            vendor::read_flag(&auto_target).ok_or("Camera returned an unknown setting")?;
        // An area list that cannot be read must fail: Done would delete the areas.
        let ids = vendor::read_area_ids(&self.request(vendor::LIST_AREAS, Vec::new())?)
            .ok_or("Camera returned an invalid area list")?;
        let initial = self
            .request(vendor::READ_INITIAL_POSITION, Vec::new())
            .ok()
            .and_then(|data| vendor::read_initial_position(&data));
        let mut areas = Vec::new();
        for id in ids {
            let name = self.request(vendor::READ_AREA_NAME, vendor::preset_id(id))?;
            let name = vendor::read_preset_name(&name)
                .filter(|name| vendor::area_name_valid(name))
                .unwrap_or_else(|| format!("Area{}", id + 1));
            let record = vendor::read_return_record(
                &self.request(vendor::READ_AREA, vendor::preset_id(id))?,
            )
            .ok_or("Camera returned an invalid area")?;
            areas.push(Area { id, name, record });
        }
        Ok(Zone {
            range: TrackingRange { limits, record },
            default_record,
            auto_target,
            areas,
            initial,
        })
    }

    fn update_zone(&mut self, shared: &Shared, field: RangeField) -> Result<(), String> {
        let current = shared.status().zone.as_ref().map(|zone| zone.range);
        let mut range = current.ok_or("The Zone Tracking panel is not open")?;
        let (angles, ratio) = self.current_view()?;
        // Like the official app, ask before a change moves the other values.
        let proposal = match field {
            RangeField::Limit(key) => {
                let (yaw, pitch) = angles;
                let value = f32::from(if key < 2 { yaw } else { pitch }) / 100.0;
                *range.limits.get_mut(key).ok_or("Unknown range limit")? = value;
                if !vendor::hand_limits_valid(&range.limits) {
                    return Err("Keep at least 10° between opposite limits".into());
                }
                range.return_within().map(|moved| (moved, false))
            }
            RangeField::Return => {
                range.record = vendor::set_return_position(&range.record, angles, ratio)
                    .ok_or("Camera returned an invalid zoom ratio")?;
                range.including_return().map(|wider| (wider, true))
            }
        };
        let mut status = shared.status();
        match (proposal, &mut status.zone) {
            (Some(proposal), _) => status.zone_proposal = Some(proposal),
            (None, Some(zone)) => zone.range = range,
            (None, None) => {}
        }
        Ok(())
    }

    /// The panel's two reset icons; nothing is written until Done.
    fn reset_zone(&mut self, shared: &Shared, limits: bool) -> Result<(), String> {
        // The official app reads the default limits from the camera.
        let defaults = if limits {
            Some(self.read_zone_limits(true)?)
        } else {
            None
        };
        if let Some(zone) = &mut shared.status().zone {
            match defaults {
                Some(limits) => zone.range.limits = limits,
                None => zone.range.record = zone.default_record,
            }
        }
        Ok(())
    }

    /// Area edits; they stay in the panel until Done.
    fn change_area(&mut self, shared: &Shared, command: GimbalCommand) -> Result<(), String> {
        let view = match command {
            GimbalCommand::AddArea | GimbalCommand::UpdateArea(_) => Some(self.current_view()?),
            _ => None,
        };
        let mut status = shared.status();
        let zone = status
            .zone
            .as_mut()
            .ok_or("The Zone Tracking panel is not open")?;
        let invalid = "Camera returned an invalid view";
        match (command, view) {
            (GimbalCommand::AddArea, Some((angles, ratio))) => {
                let id = (0..ZONE_AREAS as u8)
                    .find(|id| zone.areas.iter().all(|area| area.id != *id))
                    .ok_or("All Non-tracking Area slots are occupied")?;
                let record = vendor::area_record(id, angles, ratio).ok_or(invalid)?;
                let name = format!("Area{}", id + 1);
                zone.areas.push(Area { id, name, record });
                zone.areas.sort_by_key(|area| area.id);
            }
            (GimbalCommand::UpdateArea(id), Some((angles, ratio))) => {
                let area = zone.areas.iter_mut().find(|area| area.id == id);
                area.ok_or("No such area")?.record =
                    vendor::area_record(id, angles, ratio).ok_or(invalid)?;
            }
            (GimbalCommand::RenameArea { id, name }, _) => {
                if !vendor::area_name_valid(&name) {
                    return Err("Use 1 to 40 printable ASCII characters for a name".into());
                }
                if let Some(area) = zone.areas.iter_mut().find(|area| area.id == id) {
                    area.name = name;
                }
            }
            (GimbalCommand::DeleteArea(id), _) => zone.areas.retain(|area| area.id != id),
            _ => return Err("Unsupported area command".into()),
        }
        Ok(())
    }

    /// Leaves the zone-edit state, writing the settings when `save` is set.
    fn close_zone(&mut self, shared: &Shared, save: bool) -> Result<(), String> {
        let Some(zone) = shared.status().zone.clone() else {
            return Ok(());
        };
        let [leave, unpause] = vendor::zone_edit(false);
        let close = || {
            if save {
                vendor::zone_save(&zone)?
                    .into_iter()
                    .try_for_each(|message| self.send(message))?;
            }
            self.send(leave)?;
            // The official app's Cancel sent only `leave`.
            if save { self.send(unpause) } else { Ok(()) }
        };
        let result = close();
        // Cleared only now: exiting waits for this.
        let mut status = shared.status();
        status.zone = None;
        status.zone_proposal = None;
        result
    }

    pub(super) fn read_zoom(&mut self) -> Result<f32, String> {
        vendor::read_zoom(&self.request(vendor::READ_ZOOM, Vec::new())?)
            .ok_or_else(|| "Camera returned an invalid zoom ratio".into())
    }

    fn read_tracking_speed(&mut self) -> Result<TrackingSpeed, String> {
        let data = self.request(vendor::READ_SETTING, vendor::read_tracking_speed_request())?;
        vendor::read_tracking_speed(&data)
            .ok_or_else(|| "Camera returned an unknown tracking speed".into())
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

    pub(super) fn presets(&mut self) -> Result<Vec<Preset>, String> {
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
            GimbalCommand::SetHumanTracking(mode) => {
                let zoom = shared.status().auto_zoom.unwrap_or(AutoZoom::Off);
                for mut block in vendor::human_tracking(mode, zoom) {
                    self.set(vendor::SELECTOR_VIEW, &mut block)?;
                }
                let mut status = shared.status();
                status.ai_mode = Some(if mode.is_some() {
                    AiMode::Human
                } else {
                    AiMode::Off
                });
                status.tracking_mode = mode;
                // Auto Zoom moves the zoom once tracking starts.
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::SetAutoZoom(zoom) => {
                let tracking = {
                    let status = shared.status();
                    if status.auto_zoom == Some(zoom) {
                        return Ok(());
                    }
                    (status.ai_mode == Some(AiMode::Human)).then_some(status.tracking_mode)
                };
                for mut block in vendor::auto_zoom(zoom, tracking) {
                    self.set(vendor::SELECTOR_VIEW, &mut block)?;
                }
                shared.status().auto_zoom = Some(zoom);
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::SetTrackingSpeed(speed) => {
                if shared.status().tracking_speed == Some(speed) {
                    return Ok(());
                }
                for message in vendor::tracking_speed(speed) {
                    self.send(message)?;
                }
                shared.status().tracking_speed = Some(speed);
            }
            GimbalCommand::SetAiMode(mode) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::ai_mode(mode))?;
                let hand = (mode == AiMode::Hand)
                    .then(|| self.read_hand().ok())
                    .flatten();
                let mut status = shared.status();
                status.ai_mode = Some(mode);
                status.tracking_mode = None;
                status.hand = hand.or(status.hand);
            }
            GimbalCommand::SetVoiceRange(range) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::voice_range(range))?;
                shared.status().voice_range = Some(range);
            }
            GimbalCommand::SetSoundSource(on) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::sound_source(on))?;
                shared.status().sound_source = Some(on);
            }
            GimbalCommand::SetHdr(on) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::hdr(on))?;
                shared.status().hdr = Some(on);
            }
            GimbalCommand::SetAutoFocus(on) => {
                self.transport.set_auto_focus(on)?;
                let mut focus = shared.status().focus;
                // Like the official app: turning it off also sends the manual position.
                if !on && let Some(focus) = focus {
                    self.transport.set_focus_position(focus.position)?;
                }
                if let Some(focus) = &mut focus {
                    focus.auto = on;
                }
                shared.status().focus = focus;
            }
            GimbalCommand::SetFocusMode(mode) => {
                self.send(vendor::focus_mode(mode))?;
                if let Some(focus) = &mut shared.status().focus {
                    focus.mode = mode;
                }
            }
            GimbalCommand::SetFocusPosition(position) => {
                let position = position.min(FOCUS_MAX);
                self.transport.set_focus_position(position)?;
                if let Some(focus) = &mut shared.status().focus {
                    focus.position = position;
                }
            }
            GimbalCommand::SetAutoExposure(on) => {
                let shutter = if on {
                    self.send(vendor::exposure(true, 1))?;
                    None
                } else {
                    // Like the official app: read the stored shutter and send it back.
                    self.send(vendor::before_manual_exposure())?;
                    let state =
                        self.request_from(vendor::CAMERA, vendor::READ_EXPOSURE, Vec::new())?;
                    let (_, shutter) = vendor::read_exposure(&state)
                        .ok_or("Camera returned an unknown exposure state")?;
                    self.send(vendor::exposure(false, shutter))?;
                    Some(shutter)
                };
                if let Some(exposure) = &mut shared.status().exposure {
                    exposure.auto = on;
                    exposure.shutter = shutter.unwrap_or(exposure.shutter);
                }
            }
            GimbalCommand::SetExposureMode(mode) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::exposure_mode(mode))?;
                shared.status().exposure_mode = Some(mode);
            }
            GimbalCommand::SetShutter(shutter) => {
                let shutter = shutter.clamp(1, 10_000);
                self.send(vendor::exposure(false, shutter))?;
                if let Some(exposure) = &mut shared.status().exposure {
                    exposure.shutter = shutter;
                }
            }
            GimbalCommand::SetIso(iso) => {
                let iso = iso.clamp(100, ISO_MAX) / 100 * 100;
                self.transport
                    .set_processing(Processing::Gain, i32::from(iso / 100))?;
                if let Some(exposure) = &mut shared.status().exposure {
                    exposure.iso = iso;
                }
            }
            GimbalCommand::SetCompensation(value) => {
                let value = value.min(COMPENSATION_MAX);
                self.transport
                    .set_processing(Processing::Backlight, i32::from(value))?;
                if let Some(exposure) = &mut shared.status().exposure {
                    exposure.compensation = value;
                }
            }
            GimbalCommand::SetIsoLimits(lower, upper) => {
                if !(super::ISO_LIMITS.contains(&lower)
                    && super::ISO_LIMITS.contains(&upper)
                    && lower < upper)
                {
                    return Err("Invalid ISO limits".into());
                }
                self.send(vendor::iso_limits(lower, upper))?;
                if let Some(exposure) = &mut shared.status().exposure {
                    exposure.iso_limits = (lower, upper);
                }
            }
            GimbalCommand::SetExposureCurve(points) => {
                let points = &points[..points.len().min(vendor::CURVE_POINTS_MAX)];
                self.set(vendor::SELECTOR_VIEW, &mut vendor::exposure_curve(points))?;
                shared.status().exposure_curve = Some(points.to_vec());
            }
            GimbalCommand::SetAntiFlicker(flicker) => {
                let value = match flicker {
                    AntiFlicker::Off => 0,
                    AntiFlicker::Hz50 => 1,
                    AntiFlicker::Hz60 => 2,
                };
                self.transport
                    .set_processing(Processing::PowerLine, value)?;
                shared.status().anti_flicker = Some(flicker);
            }
            GimbalCommand::SetSleepTime(seconds) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::sleep_time(seconds))?;
                if let Some(sleep) = &mut shared.status().device_sleep {
                    sleep.time = seconds;
                }
            }
            GimbalCommand::SetSleepBackground(index) => {
                if index >= super::SLEEP_BACKGROUNDS {
                    return Err(format!("Unknown sleep background {index}"));
                }
                self.set(vendor::SELECTOR_VIEW, &mut vendor::sleep_background(index))?;
                if let Some(sleep) = &mut shared.status().device_sleep {
                    sleep.background = index;
                }
            }
            GimbalCommand::SetSleepMirror(on) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::sleep_mirror(on))?;
                if let Some(sleep) = &mut shared.status().device_sleep {
                    sleep.mirror = on;
                }
            }
            GimbalCommand::SetGesture(gesture, on) => {
                self.send(vendor::gesture(gesture, on))?;
                if gesture == super::Gesture::LockedTarget {
                    shared.status().locked_target = Some(on);
                }
            }
            GimbalCommand::SetGestureZoom(ratio) => self.send(vendor::gesture_zoom(ratio))?,
            GimbalCommand::SetVoiceCommand(command, on) => {
                self.set(
                    vendor::SELECTOR_VIEW,
                    &mut vendor::voice_command(command, on),
                )?;
                if let Some(voice) = &mut shared.status().voice_control {
                    voice.set(command, on);
                }
            }
            GimbalCommand::SetVoiceLocator(on) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::voice_locator(on))?;
                if let Some(voice) = &mut shared.status().voice_control {
                    voice.locator = on;
                }
            }
            GimbalCommand::SetVoiceEnglish(english) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::voice_language(english))?;
                if let Some(voice) = &mut shared.status().voice_control {
                    voice.english = english;
                }
            }
            GimbalCommand::SetVoiceZoom(steps) => {
                let steps = steps.min(super::VOICE_ZOOM_MAX);
                self.set(vendor::SELECTOR_VIEW, &mut vendor::voice_zoom(steps))?;
                if let Some(voice) = &mut shared.status().voice_control {
                    voice.zoom = steps;
                }
            }
            GimbalCommand::SetAudioInput(input) => {
                self.send(vendor::audio_input(input))?;
                if let Some(audio) = &mut shared.status().audio {
                    audio.input = input;
                }
            }
            GimbalCommand::SetAudioMode(mode) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::audio_mode(mode))?;
                if let Some(audio) = &mut shared.status().audio {
                    audio.mode = mode;
                }
            }
            GimbalCommand::SetSleepMic(on) => {
                self.set(vendor::SELECTOR_VIEW, &mut vendor::sleep_mic(on))?;
                if let Some(audio) = &mut shared.status().audio {
                    audio.sleep_mic = on;
                }
            }
            GimbalCommand::SetMicrophone(on) => {
                // The camera restarts and comes back as a new device.
                self.set(vendor::SELECTOR_VIEW, &mut vendor::microphone(on))?;
                if let Some(audio) = &mut shared.status().audio {
                    audio.microphone = on;
                }
            }
            GimbalCommand::PairVox(transmitter) => {
                self.exchange(vendor::vox_pairing(transmitter))?;
            }
            GimbalCommand::SetImage(image) => {
                let new = image.values().map(|v| v.min(IMAGE_MAX));
                for (i, control) in IMAGE_CONTROLS.into_iter().enumerate() {
                    let old = shared.status().image.map(ImageAdjust::values);
                    // Write only what changed, so one slider sends one request.
                    if old.is_some_and(|old| old[i] == new[i]) {
                        continue;
                    }
                    self.transport.set_processing(control, i32::from(new[i]))?;
                    let mut values = old.unwrap_or(new);
                    values[i] = new[i];
                    shared.status().image = Some(ImageAdjust::from_values(values));
                }
            }
            GimbalCommand::SetWhiteBalance(wb) => {
                let message = vendor::white_balance(&wb);
                // The clamped values that were sent.
                let sent = vendor::read_white_balance(&message.data);
                self.send(message)?;
                shared.status().white_balance = sent;
            }
            GimbalCommand::SetHand(hand) => {
                self.send(vendor::hand(hand))?;
                shared.status().hand = Some(hand);
            }
            GimbalCommand::SelectPoint { x, y } => {
                self.send(vendor::select_point(x, y))?;
                // The official app showed Single Mode after a double-click.
                let mut status = shared.status();
                status.ai_mode = Some(AiMode::Human);
                status.tracking_mode = Some(TrackingMode::Single);
                status.object_framing = None;
            }
            GimbalCommand::SelectObject(target) => {
                self.send(vendor::select_object(target))?;
                let mut status = shared.status();
                status.ai_mode = Some(AiMode::Human);
                status.tracking_mode = None;
                status.object_framing = Some(ObjectFraming::Normal);
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::CancelTarget => {
                self.send(vendor::cancel_target())?;
                let mut status = shared.status();
                status.ai_mode = Some(AiMode::Off);
                status.tracking_mode = None;
                status.object_framing = None;
            }
            GimbalCommand::SetObjectFraming(framing) => {
                self.send(vendor::object_framing(framing))?;
                shared.status().object_framing = Some(framing);
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::ReadComposition => {
                let composition = self.read_composition();
                shared.status().composition = composition.as_ref().ok().copied();
                composition?;
            }
            GimbalCommand::SetComposition { x, y } => {
                for message in vendor::composition(x, y) {
                    self.send(message)?;
                }
                let mut status = shared.status();
                let composition = status.composition.get_or_insert_default();
                composition.x = super::clamp_composition(x);
                composition.y = super::clamp_composition(y);
            }
            GimbalCommand::SetFaceFraming(on) => {
                self.send(vendor::face_framing(on))?;
                shared
                    .status()
                    .composition
                    .get_or_insert_default()
                    .face_framing = on;
            }
            GimbalCommand::OpenHandRange => self.open_hand_range(shared)?,
            GimbalCommand::UpdateHandRange(field) => self.update_hand_range(shared, field)?,
            GimbalCommand::ResetHandRange { limits } => Self::reset_hand_range(shared, limits),
            GimbalCommand::ConfirmHandRange(widen) => Self::confirm_hand_range(shared, widen),
            GimbalCommand::CloseHandRange { save } => {
                self.close_hand_range(shared, save)?;
                // The close sequence zooms back to 1.0x.
                *zoom_refresh = follow_zoom();
            }
            GimbalCommand::SetZoneTracking(on) => {
                self.send(vendor::zone_tracking(on))?;
                shared.status().zone_tracking = Some(on);
            }
            GimbalCommand::OpenZone => self.open_zone(shared)?,
            GimbalCommand::UpdateZone(field) => self.update_zone(shared, field)?,
            GimbalCommand::ResetZone { limits } => self.reset_zone(shared, limits)?,
            GimbalCommand::ConfirmZone(accept) => {
                let mut status = shared.status();
                let proposal = status.zone_proposal.take();
                if let (true, Some((range, _)), Some(zone)) = (accept, proposal, &mut status.zone) {
                    zone.range = range;
                }
            }
            GimbalCommand::SetZoneAutoTarget(on) => {
                self.send(vendor::zone_auto_target(on))?;
                if let Some(zone) = &mut shared.status().zone {
                    zone.auto_target = on;
                }
            }
            GimbalCommand::AddArea
            | GimbalCommand::UpdateArea(_)
            | GimbalCommand::RenameArea { .. }
            | GimbalCommand::DeleteArea(_) => self.change_area(shared, command)?,
            GimbalCommand::CloseZone { save } => self.close_zone(shared, save)?,
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
                // The plain Reset zoomed out to 1.0x too (2026-11-25), so
                // every reset reads the zoom back.
                if matches!(
                    command,
                    GimbalCommand::TriggerPreset(_)
                        | GimbalCommand::Reset
                        | GimbalCommand::ResetWithZoom
                ) {
                    // The official app showed Wide after the 1.0x reset.
                    shared.status().fov =
                        (command == GimbalCommand::ResetWithZoom).then_some(super::Fov::Wide);
                    *zoom_refresh = follow_zoom();
                }
            }
        }
        Ok(())
    }
}

/// Next zoom read, and until when reads repeat every [`ZOOM_FOLLOW_EVERY`].
pub(super) type ZoomRefresh = Option<(Instant, Instant)>;

/// The camera zooms smoothly after a field of view or preset change; reading
/// the ratio this often lets the slider move along with it.
const ZOOM_FOLLOW_EVERY: Duration = Duration::from_millis(200);
/// Long enough for the camera to settle after a field of view change.
const ZOOM_FOLLOW_FOR: Duration = Duration::from_millis(2500);

/// How long the status block may lag a tracking write (it showed the new
/// state within 2 s in the 2026-11-21 capture).
const TRACKING_SETTLE: Duration = Duration::from_millis(2500);

/// Human Tracking that runs in a mode Gimbly did not set (the Locked Target
/// palm gesture, another app) shows as Single Mode, as in OBSBOT Center: the
/// camera reports on/off only.
fn assume_single_mode(status: &mut super::SessionStatus) {
    if status.ai_mode == Some(AiMode::Human)
        && status.object_framing.is_none()
        && status.tracking_mode.is_none()
    {
        status.tracking_mode = Some(TrackingMode::Single);
    }
}

pub(super) fn read_zoom_in(delay: Duration) -> ZoomRefresh {
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
/// The tracked target's box is read from the camera's serial port on one
/// more thread; `device_id` identifies the camera for it.
pub(super) fn control_loop(
    xu: Channel<impl Transport>,
    commands: &Receiver,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
    zoom_refresh: ZoomRefresh,
    device_id: &str,
) {
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let stop = &stop;
        let started = std::thread::Builder::new()
            .name("camera-target".into())
            .spawn_scoped(scope, move || {
                serial::target_stream(device_id, shared, repaint, stop)
            });
        if started.is_err() {
            shared.status().error = Some("Could not start the tracking box reader".into());
        }
        commands_loop(xu, commands, shared, repaint, zoom_refresh);
        stop.store(true, Ordering::Relaxed);
    });
}

fn commands_loop(
    mut xu: Channel<impl Transport>,
    commands: &Receiver,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
    mut zoom_refresh: ZoomRefresh,
) {
    let mut sleep_polled = Instant::now();
    let mut sleep_readback = SleepReadback::default();
    let mut tracking_sent: Option<Instant> = None;
    let mut preset_retries = 0;
    let mut dragging = false;
    let mut stopped = false;
    // The tracking state last given to the target report; `None` until set
    // and after a wake.
    let mut report: Option<bool> = None;
    loop {
        let mut wake = sleep_polled + Duration::from_secs(1);
        if let Some((next, _)) = zoom_refresh {
            wake = wake.min(next);
        }
        let Ok(batch) = commands.wait(wake.saturating_duration_since(Instant::now())) else {
            break;
        };
        for command in batch {
            match command {
                Command::SetSleep(sleep) => {
                    let result = xu.send(vendor::sleep(sleep));
                    match result {
                        Ok(()) => {
                            shared.set_sleeping(sleep);
                            sleep_polled = Instant::now();
                            sleep_readback.sent(sleep, sleep_polled);
                            preset_retries = 0;
                            zoom_refresh = if sleep {
                                None
                            } else {
                                read_zoom_in(Duration::from_secs(3))
                            };
                        }
                        Err(error) => shared.status().error = Some(error),
                    }
                    repaint();
                }
                Command::Gimbal(command) => {
                    let (next_dragging, next_stopped) = match command {
                        GimbalCommand::Drag(active) => (active, !active),
                        GimbalCommand::Speed { pitch, yaw } => {
                            (dragging, pitch == 0.0 && yaw == 0.0)
                        }
                        _ => (dragging, stopped),
                    };
                    if matches!(command, GimbalCommand::Drag(false)) && !stopped {
                        let _ = xu.send(vendor::speed(0.0, 0.0));
                    }
                    if matches!(
                        command,
                        GimbalCommand::SetHumanTracking(_)
                            | GimbalCommand::SetAutoZoom(_)
                            | GimbalCommand::SetAiMode(_)
                            | GimbalCommand::SetVoiceRange(_)
                            | GimbalCommand::SetSoundSource(_)
                            | GimbalCommand::SetHdr(_)
                            | GimbalCommand::SetAutoFocus(_)
                            | GimbalCommand::SetFocusMode(_)
                            | GimbalCommand::SetFocusPosition(_)
                            | GimbalCommand::SetExposureMode(_)
                            | GimbalCommand::SetAntiFlicker(_)
                            | GimbalCommand::SetSleepTime(_)
                            | GimbalCommand::SetSleepBackground(_)
                            | GimbalCommand::SetSleepMirror(_)
                            | GimbalCommand::SetGesture(..)
                            | GimbalCommand::SetVoiceCommand(..)
                            | GimbalCommand::SetVoiceLocator(_)
                            | GimbalCommand::SetVoiceEnglish(_)
                            | GimbalCommand::SetVoiceZoom(_)
                            | GimbalCommand::SetAudioInput(_)
                            | GimbalCommand::SetAudioMode(_)
                            | GimbalCommand::SetSleepMic(_)
                            | GimbalCommand::SetMicrophone(_)
                            | GimbalCommand::SelectPoint { .. }
                            | GimbalCommand::SelectObject(_)
                            | GimbalCommand::CancelTarget
                            | GimbalCommand::SetObjectFraming(_)
                    ) {
                        tracking_sent = Some(Instant::now());
                    }
                    match xu.gimbal(command, shared, &mut zoom_refresh) {
                        Ok(()) => {
                            dragging = next_dragging;
                            stopped = next_stopped;
                        }
                        Err(e) => shared.status().error = Some(e),
                    }
                    repaint();
                }
                _ => {}
            }
        }
        if sleep_polled.elapsed() >= Duration::from_secs(1) {
            let view = xu.read_view();
            // The status block can still show the old state just after a write.
            if let Some(view) = view
                && tracking_sent.is_none_or(|sent| sent.elapsed() >= TRACKING_SETTLE)
            {
                let mut status = shared.status();
                let before = (
                    status.ai_mode,
                    status.auto_zoom,
                    status.voice_range,
                    status.sound_source,
                    status.hdr,
                    status.frame_rate,
                    status.focus,
                    status.exposure_mode,
                    status.anti_flicker,
                    status.object_framing,
                    status.device_sleep,
                    status.locked_target,
                );
                let before_audio = (status.voice_control, status.audio);
                let tracking_mode = status.tracking_mode;
                // Unknown values (such as the switching value 6) keep the old state.
                status.ai_mode = view.ai_mode.or(status.ai_mode);
                status.auto_zoom = view.auto_zoom.or(status.auto_zoom);
                status.voice_range = view.voice_range.or(status.voice_range);
                status.sound_source = view.sound_source.or(status.sound_source);
                status.hdr = view.hdr.or(status.hdr);
                status.frame_rate = view.frame_rate.or(status.frame_rate);
                status.focus = view.focus.or(status.focus);
                status.exposure_mode = view.exposure_mode.or(status.exposure_mode);
                status.anti_flicker = view.anti_flicker.or(status.anti_flicker);
                status.device_sleep = view.device_sleep.or(status.device_sleep);
                status.locked_target = view.locked_target.or(status.locked_target);
                status.voice_control = view.voice_control.or(status.voice_control);
                status.audio = view.audio.or(status.audio);
                // Another app may turn tracking on later in a mode we cannot read.
                if view.ai_mode.is_some_and(|mode| mode != AiMode::Human) {
                    status.tracking_mode = None;
                }
                if view.ai_mode.is_some() {
                    status.object_framing = view.object_framing;
                }
                if status.object_framing.is_some() {
                    status.tracking_mode = None;
                }
                assume_single_mode(&mut status);
                let changed = before
                    != (
                        status.ai_mode,
                        status.auto_zoom,
                        status.voice_range,
                        status.sound_source,
                        status.hdr,
                        status.frame_rate,
                        status.focus,
                        status.exposure_mode,
                        status.anti_flicker,
                        status.object_framing,
                        status.device_sleep,
                        status.locked_target,
                    )
                    || before_audio != (status.voice_control, status.audio)
                    || tracking_mode != status.tracking_mode;
                drop(status);
                if changed {
                    repaint();
                }
            }
            if let Some(sleeping) =
                view.and_then(|view| sleep_readback.observe(view.sleeping, Instant::now()))
                && shared.set_sleeping(sleeping)
            {
                preset_retries = 0;
                report = None;
                zoom_refresh = if sleeping {
                    None
                } else {
                    read_zoom_in(Duration::from_secs(2))
                };
                repaint();
            }
            sleep_polled = Instant::now();
            // Opening or waking the stream can briefly interrupt replies.
            let (retry_presets, retry_speed, retry_hand, retry_zone, retry_exposure) = {
                let status = shared.status();
                if status.presets.is_some()
                    && status.tracking_speed.is_some()
                    && status.hand.is_some()
                    && status.zone_tracking.is_some()
                    && status.exposure.is_some()
                    && status.exposure_curve.is_some()
                    && status.white_balance.is_some()
                    && status.image.is_some()
                {
                    preset_retries = 0;
                }
                let retry = !status.sleeping && preset_retries < 3;
                (
                    retry && status.presets.is_none(),
                    retry && status.tracking_speed.is_none(),
                    retry && status.hand.is_none(),
                    retry && status.zone_tracking.is_none(),
                    retry
                        && (status.exposure.is_none()
                            || status.exposure_curve.is_none()
                            || status.white_balance.is_none()
                            || status.image.is_none()),
                )
            };
            if retry_presets || retry_speed || retry_hand || retry_zone || retry_exposure {
                preset_retries += 1;
            }
            if retry_exposure {
                if let Ok(exposure) = xu.read_exposure() {
                    shared.status().exposure = Some(exposure);
                    repaint();
                }
                if let Ok(curve) = xu.read_exposure_curve() {
                    shared.status().exposure_curve = Some(curve);
                    repaint();
                }
                if let Ok(wb) = xu.read_white_balance() {
                    shared.status().white_balance = Some(wb);
                    repaint();
                }
                if let Ok(image) = xu.read_image() {
                    shared.status().image = Some(image);
                    repaint();
                }
            }
            if retry_zone && let Ok(on) = xu.read_zone_tracking() {
                shared.status().zone_tracking = Some(on);
                repaint();
            }
            if retry_hand && let Ok(hand) = xu.read_hand() {
                shared.status().hand = Some(hand);
                repaint();
            }
            if retry_presets && let Ok(presets) = xu.presets() {
                shared.status().presets = Some(presets);
                repaint();
            }
            if retry_speed && let Ok(speed) = xu.read_tracking_speed() {
                shared.status().tracking_speed = Some(speed);
                repaint();
            }
            // Gestures, auto zoom and other apps zoom without telling us.
            if zoom_refresh.is_none()
                && !shared.status().sleeping
                && let Ok(ratio) = xu.read_zoom()
            {
                let mut status = shared.status();
                if status.zoom_ratio != Some(ratio) {
                    if status.zoom_ratio.is_some() {
                        status.fov = None;
                    }
                    status.zoom_ratio = Some(ratio);
                    drop(status);
                    repaint();
                }
            }
        }
        let (tracking, sleeping) = {
            let status = shared.status();
            (status.ai_mode == Some(AiMode::Human), status.sleeping)
        };
        if !sleeping && report != Some(tracking) {
            xu.target_report(tracking, report.is_none());
            report = Some(tracking);
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
    if dragging {
        let _ = xu.send(vendor::speed(0.0, 0.0));
        let _ = xu.send(vendor::manual_move(false));
    }
    // Never leave the camera in the range-edit state (best effort: it may
    // be gone already).
    let _ = xu.close_hand_range(shared, false);
    let _ = xu.close_zone(shared, false);
}

/// The status block fields Gimbly reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
    pub sleeping: bool,
    pub ai_mode: Option<AiMode>,
    pub auto_zoom: Option<AutoZoom>,
    pub voice_range: Option<VoiceRange>,
    pub sound_source: Option<bool>,
    pub hdr: Option<bool>,
    pub frame_rate: Option<u8>,
    pub focus: Option<Focus>,
    pub exposure_mode: Option<super::FocusMode>,
    pub anti_flicker: Option<AntiFlicker>,
    pub object_framing: Option<ObjectFraming>,
    pub device_sleep: Option<super::DeviceSleep>,
    pub locked_target: Option<bool>,
    pub voice_control: Option<super::VoiceControl>,
    pub audio: Option<super::Audio>,
}

/// Finds the control interface and unit by GUID in a USB descriptor stream.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn extension_unit(descriptors: &[u8]) -> Option<(u8, u8)> {
    const GUID: [u8; 16] = [
        0x91, 0x72, 0x1e, 0x9a, 0x43, 0x68, 0x83, 0x46, 0x6d, 0x92, 0x39, 0xbc, 0x79, 0x06, 0xee,
        0x49,
    ];
    video_control_entities(descriptors, |descriptor| match descriptor {
        [_, 0x24, 6, unit, ..] if descriptor.get(4..20) == Some(&GUID) => Some(*unit),
        _ => None,
    })
}

#[cfg(target_os = "macos")]
pub(super) fn zoom_terminal(descriptors: &[u8]) -> Option<(u8, u8)> {
    video_control_entities(descriptors, |descriptor| match descriptor {
        [_, 0x24, 2, terminal, 1, 2, ..]
            if descriptor.get(14).is_some_and(|&size| size >= 2)
                && descriptor.get(16).is_some_and(|flags| flags & 2 != 0) =>
        {
            Some(*terminal)
        }
        _ => None,
    })
}

/// The camera input terminal (focus and the other camera controls).
#[cfg(target_os = "macos")]
pub(super) fn camera_terminal(descriptors: &[u8]) -> Option<(u8, u8)> {
    video_control_entities(descriptors, |descriptor| match descriptor {
        [_, 0x24, 2, terminal, 1, 2, ..] => Some(*terminal),
        _ => None,
    })
}

/// The processing unit (exposure, gain and the other picture controls).
#[cfg(target_os = "macos")]
pub(super) fn processing_unit(descriptors: &[u8]) -> Option<(u8, u8)> {
    video_control_entities(descriptors, |descriptor| match descriptor {
        [_, 0x24, 5, unit, ..] => Some(*unit),
        _ => None,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn video_control_entities(
    mut descriptors: &[u8],
    entity: impl Fn(&[u8]) -> Option<u8>,
) -> Option<(u8, u8)> {
    let mut interface = None;
    while let Some(&length) = descriptors.first() {
        if length < 2 {
            break;
        }
        let (descriptor, tail) = descriptors.split_at_checked(length as usize)?;
        match descriptor {
            [_, 4, number, 0, _, 0x0e, 1, ..] => interface = Some(*number),
            [_, 4, ..] => interface = None,
            _ => {
                if let Some(interface) = interface
                    && let Some(id) = entity(descriptor)
                {
                    return Some((interface, id));
                }
            }
        }
        descriptors = tail;
    }
    None
}

pub(super) trait AbsoluteZoom {
    fn get(&self) -> Option<i32>;
    fn set(&mut self, value: i32) -> Result<i32, String>;
}

pub(super) fn zoom_loop(
    mut zoom: impl AbsoluteZoom,
    commands: &Receiver,
    shared: &Shared,
    repaint: &(dyn Fn() + Send + Sync),
) {
    while let Ok(batch) = commands.wait(Duration::from_millis(250)) {
        let mut changed = false;
        for command in batch {
            if let Command::SetZoom(value) = command {
                let result = zoom.set(value);
                let mut status = shared.status();
                match result {
                    Ok(v) => status.zoom = Some(v),
                    Err(error) => status.error = Some(error),
                }
                changed = true;
            } else {
                shared.status().error =
                    Some("This camera does not support the requested control".into());
                changed = true;
            }
        }
        let value = zoom.get();
        let mut status = shared.status();
        changed |= status.zoom != value;
        status.zoom = value;
        drop(status);
        if changed {
            repaint();
        }
    }
}

/// What the camera answers for each feature, using reads only. Run it with
/// `gimbly --probe --controls report` when adding a model.
pub(super) fn report(xu: &mut Channel<impl Transport>) -> Vec<String> {
    let view = xu.read_view();
    let mut lines = vec![format!(
        "status block (selector 6): {}",
        if view.is_some() {
            "recognized"
        } else {
            "not recognized"
        }
    )];
    for feature in Feature::ALL {
        let answer = match feature {
            Feature::Gimbal => format!(
                "{:?}",
                xu.request(vendor::READ_POSITION, Vec::new())
                    .map(|data| vendor::current_angles(&data))
            ),
            Feature::Zoom => format!("{:?}", xu.read_zoom()),
            Feature::FieldOfView => "no readback; try Wide, Medium and Narrow".into(),
            Feature::Presets => format!("{:?}", xu.presets().map(|p| p.len())),
            Feature::Sleep => format!("sleeping {:?}", view.map(|v| v.sleeping)),
            Feature::HumanTracking => format!("{:?}", view.map(|v| v.ai_mode)),
            Feature::AutoZoom => format!("{:?}", view.map(|v| v.auto_zoom)),
            Feature::TrackingSpeed => format!("{:?}", xu.read_tracking_speed()),
            Feature::SoundSource => format!("{:?}", view.map(|v| v.sound_source)),
            Feature::Hdr => format!(
                "{:?} at {:?} fps",
                view.map(|v| v.hdr),
                view.map(|v| v.frame_rate)
            ),
            Feature::Focus => format!("{:?}", view.map(|v| v.focus)),
            Feature::Exposure => format!(
                "{:?}, mode {:?}, anti-flicker {:?}, curve {:?}",
                xu.read_exposure(),
                view.map(|v| v.exposure_mode),
                view.map(|v| v.anti_flicker),
                xu.read_exposure_curve()
            ),
            Feature::WhiteBalance => format!("{:?}", xu.read_white_balance()),
            Feature::Image => format!("{:?}", xu.read_image()),
            Feature::DeviceSleep => format!("{:?}", view.map(|v| v.device_sleep)),
            Feature::VoiceControl => format!("{:?}", view.map(|v| v.voice_control)),
            Feature::Audio => format!("{:?}", view.map(|v| v.audio)),
            Feature::Gestures => format!(
                "Locked Target {:?} (the other gestures are not reported)",
                view.map(|v| v.locked_target)
            ),
            Feature::ObjectTracking => format!(
                "{:?} (None unless an object is tracked)",
                view.map(|v| v.object_framing)
            ),
            Feature::Composition => format!("{:?}", xu.read_composition()),
            Feature::VoiceTracking => format!("{:?}", view.map(|v| v.voice_range)),
            Feature::HandTracking => format!("{:?}", xu.read_hand()),
            Feature::ZoneTracking => format!("switch {:?}", xu.read_zone_tracking()),
        };
        lines.push(format!("{}: {answer}", feature.name()));
    }
    lines
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn probe(
    xu: &mut Channel<impl Transport>,
    steps: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for step in steps {
        let words: Vec<_> = step.split(':').collect();
        match words.as_slice() {
            ["status"] => lines.push(format!(
                "selector 6: {:?}, vendor zoom: {:?}, presets: {:?}",
                xu.get(vendor::SELECTOR_VIEW),
                xu.read_zoom(),
                xu.presets()
            )),
            ["report"] => lines.extend(report(xu)),
            ["ptz"] => lines.push(format!(
                "current position bytes: {:?}",
                xu.request(vendor::READ_POSITION, Vec::new())?
            )),
            ["move"] => xu.send(vendor::manual_move(true))?,
            ["stop"] => {
                xu.send(vendor::speed(0.0, 0.0))?;
                xu.send(vendor::manual_move(false))?;
            }
            ["turn", pitch, yaw, ms] => {
                let (Ok(pitch), Ok(yaw), Ok(ms)) =
                    (pitch.parse::<f32>(), yaw.parse::<f32>(), ms.parse::<u64>())
                else {
                    return Err(format!("Bad control step {step:?}"));
                };
                if !pitch.is_finite() || !yaw.is_finite() || ms > 60_000 {
                    return Err(format!("Bad control step {step:?}"));
                }
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(ms) {
                    xu.send(vendor::speed(pitch, yaw))?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                xu.send(vendor::speed(0.0, 0.0))?;
            }
            [ms] => {
                let ms = ms
                    .parse::<u64>()
                    .ok()
                    .filter(|ms| *ms <= 60_000)
                    .ok_or_else(|| format!("Bad control step {step:?}"))?;
                std::thread::sleep(Duration::from_millis(ms));
            }
            _ => return Err(format!("Bad control step {step:?}")),
        }
    }
    Ok(lines)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::extension_unit;

    #[test]
    fn unknown_tracking_mode_shows_as_single() {
        use super::{AiMode, ObjectFraming, TrackingMode, assume_single_mode};
        let mut status = crate::camera::SessionStatus {
            ai_mode: Some(AiMode::Human),
            ..Default::default()
        };
        assume_single_mode(&mut status);
        assert_eq!(status.tracking_mode, Some(TrackingMode::Single));
        // Group Mode set by Gimbly stays.
        status.tracking_mode = Some(TrackingMode::Group);
        assume_single_mode(&mut status);
        assert_eq!(status.tracking_mode, Some(TrackingMode::Group));
        // Object tracking and other AI modes show no mode.
        for (ai_mode, framing) in [
            (AiMode::Human, Some(ObjectFraming::Normal)),
            (AiMode::Hand, None),
            (AiMode::Off, None),
        ] {
            status.ai_mode = Some(ai_mode);
            status.object_framing = framing;
            status.tracking_mode = None;
            assume_single_mode(&mut status);
            assert_eq!(status.tracking_mode, None);
        }
    }

    #[test]
    fn image_values_out_of_range_are_refused() {
        use super::image_value;
        assert_eq!(image_value(0x32), Some(50));
        assert_eq!(image_value(0), Some(0));
        assert_eq!(image_value(100), Some(100));
        // What a sleeping camera answered for all four.
        assert_eq!(image_value(0x63c0), None);
        assert_eq!(image_value(101), None);
        assert_eq!(image_value(-1), None);
    }

    #[test]
    fn matches_extension_guid_only_in_video_control_interface() {
        let mut descriptors = vec![9, 4, 7, 0, 0, 0x0e, 1, 0, 0];
        descriptors.extend_from_slice(&[
            20, 0x24, 6, 2, 0x91, 0x72, 0x1e, 0x9a, 0x43, 0x68, 0x83, 0x46, 0x6d, 0x92, 0x39, 0xbc,
            0x79, 0x06, 0xee, 0x49,
        ]);
        assert_eq!(extension_unit(&descriptors), Some((7, 2)));
        assert_eq!(extension_unit(&descriptors[..28]), None);
        descriptors[6] = 2;
        assert_eq!(extension_unit(&descriptors), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finds_the_camera_terminal() {
        use super::camera_terminal;
        // Interface 0, then a camera input terminal (id 1, type 0x0201).
        let mut descriptors = vec![9, 4, 0, 0, 0, 0x0e, 1, 0, 0];
        descriptors.extend_from_slice(&[
            18, 0x24, 2, 1, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0x2a, 0x0a, 0x02,
        ]);
        assert_eq!(camera_terminal(&descriptors), Some((0, 1)));
        // An input terminal of another type (0x0202) is not the camera.
        descriptors[13] = 2;
        assert_eq!(camera_terminal(&descriptors), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finds_the_processing_unit() {
        use super::processing_unit;
        // Interface 0, then a processing unit (id 3, source 1).
        let mut descriptors = vec![9, 4, 0, 0, 0, 0x0e, 1, 0, 0];
        descriptors.extend_from_slice(&[13, 0x24, 5, 3, 1, 0, 0, 3, 0x5f, 0x17, 0, 0, 0]);
        assert_eq!(processing_unit(&descriptors), Some((0, 3)));
        descriptors[11] = 6;
        assert_eq!(processing_unit(&descriptors), None);
    }
}
