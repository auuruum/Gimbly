//! Shared vendor protocol and command worker. Transports implement only UVC
//! reads and writes. Only this worker waits for the camera to answer.

use super::queue::Receiver;
use super::sleep::SleepReadback;
use super::{Command, GimbalCommand, PRESET_SLOTS, Preset, Shared, TrackingSpeed, vendor};
use std::time::{Duration, Instant};

pub(super) const TINY_3_LITE: u16 = 0xff04;

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
        shared.set_sleeping(view.map(|(sleeping, _)| sleeping).unwrap_or(false));
        let ratio = self.read_zoom().ok();
        let presets = self.presets().ok();
        let tracking_speed = self.read_tracking_speed().ok();
        let mut status = shared.status();
        status.human_tracking = view.and_then(|(_, tracking)| tracking);
        status.tracking_speed = tracking_speed;
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

    /// Sleep and tracking state from one read of the status block.
    pub(super) fn read_view(&self) -> Option<(bool, Option<bool>)> {
        let (block, len) = self.get(vendor::SELECTOR_VIEW).ok()?;
        let block = &block[..len];
        Some((vendor::sleeping(block)?, vendor::tracking(block)))
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
            GimbalCommand::SetHumanTracking(on) => {
                for mut block in vendor::human_tracking(on) {
                    self.set(vendor::SELECTOR_VIEW, &mut block)?;
                }
                shared.status().human_tracking = Some(on);
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
pub(super) fn control_loop(
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
                    if matches!(command, GimbalCommand::SetHumanTracking(_)) {
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
            if let Some(tracking) = view.and_then(|(_, tracking)| tracking)
                && tracking_sent.is_none_or(|sent| sent.elapsed() >= TRACKING_SETTLE)
            {
                let mut status = shared.status();
                if status.human_tracking != Some(tracking) {
                    status.human_tracking = Some(tracking);
                    drop(status);
                    repaint();
                }
            }
            if let Some(sleeping) =
                view.and_then(|(sleeping, _)| sleep_readback.observe(sleeping, Instant::now()))
                && shared.set_sleeping(sleeping)
            {
                preset_retries = 0;
                zoom_refresh = if sleeping {
                    None
                } else {
                    read_zoom_in(Duration::from_secs(2))
                };
                repaint();
            }
            sleep_polled = Instant::now();
            // Opening or waking the stream can briefly interrupt replies.
            let (retry_presets, retry_speed) = {
                let status = shared.status();
                if status.presets.is_some() && status.tracking_speed.is_some() {
                    preset_retries = 0;
                }
                let retry = !status.sleeping && preset_retries < 3;
                (
                    retry && status.presets.is_none(),
                    retry && status.tracking_speed.is_none(),
                )
            };
            if retry_presets || retry_speed {
                preset_retries += 1;
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
