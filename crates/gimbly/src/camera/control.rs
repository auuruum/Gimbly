//! Shared vendor protocol and command worker. Transports implement only UVC
//! reads and writes. Only this worker waits for the camera to answer.

use super::queue::Receiver;
use super::sleep::SleepReadback;
use super::{
    AiMode, Area, AutoZoom, Command, Composition, Feature, GimbalCommand, Hand, ObjectFraming,
    PRESET_SLOTS, Preset, RangeField, Shared, TrackingMode, TrackingRange, TrackingSpeed,
    VoiceRange, ZONE_AREAS, Zone, serial, vendor,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) trait Transport {
    fn read(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<usize, String>;
    fn write(&self, selector: u32, block: &mut [u8; vendor::MESSAGE_LEN]) -> Result<(), String>;
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
        let mut status = shared.status();
        status.ai_mode = view.and_then(|view| view.ai_mode);
        status.auto_zoom = view.and_then(|view| view.auto_zoom);
        status.voice_range = view.and_then(|view| view.voice_range);
        status.sound_source = view.and_then(|view| view.sound_source);
        status.hdr = view.and_then(|view| view.hdr);
        status.frame_rate = view.and_then(|view| view.frame_rate);
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
            object_framing: vendor::read_object_framing(block),
        })
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
                    status.object_framing,
                );
                // Unknown values (such as the switching value 6) keep the old state.
                status.ai_mode = view.ai_mode.or(status.ai_mode);
                status.auto_zoom = view.auto_zoom.or(status.auto_zoom);
                status.voice_range = view.voice_range.or(status.voice_range);
                status.sound_source = view.sound_source.or(status.sound_source);
                status.hdr = view.hdr.or(status.hdr);
                status.frame_rate = view.frame_rate.or(status.frame_rate);
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
                let changed = before
                    != (
                        status.ai_mode,
                        status.auto_zoom,
                        status.voice_range,
                        status.sound_source,
                        status.hdr,
                        status.frame_rate,
                        status.object_framing,
                    );
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
            let (retry_presets, retry_speed, retry_hand, retry_zone) = {
                let status = shared.status();
                if status.presets.is_some()
                    && status.tracking_speed.is_some()
                    && status.hand.is_some()
                    && status.zone_tracking.is_some()
                {
                    preset_retries = 0;
                }
                let retry = !status.sleeping && preset_retries < 3;
                (
                    retry && status.presets.is_none(),
                    retry && status.tracking_speed.is_none(),
                    retry && status.hand.is_none(),
                    retry && status.zone_tracking.is_none(),
                )
            };
            if retry_presets || retry_speed || retry_hand || retry_zone {
                preset_retries += 1;
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
    pub object_framing: Option<ObjectFraming>,
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
}
