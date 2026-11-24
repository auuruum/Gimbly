//! OBSBOT vendor messages, carried in the camera's UVC extension unit.
//!
//! Learned from USB captures of the official app talking to a Tiny 3 Lite;
//! see `docs/clean-room/2026-11-08-vendor-messages.md`, and
//! `docs/protocol.md` for every message by feature. Every message is
//! written with SET_CUR to [`SELECTOR_COMMAND`] as one [`MESSAGE_LEN`]-byte
//! block; a reply, when the camera sends one, is read back with GET_CUR on
//! the same selector.

//!
//! Layout (multi-byte fields little-endian):
//!
//! | bytes | field |
//! | --- | --- |
//! | 0 | `0xaa` |
//! | 1 | flags |
//! | 2 | sequence number, counted per channel |
//! | 3 | channel |
//! | 4-5 | header length, always 12 |
//! | 6-7 | CRC of bytes 0-11 with this field zeroed |
//! | 8 | source |
//! | 9 | destination |
//! | 10-11 | command |
//! | 12-13 | data length (only with flag `0x20` or `0x40`) |
//! | 14-15 | CRC of bytes 12-15 (this field zeroed) and the data |
//! | 16.. | data, then padding |

use super::{
    AiMode, AutoZoom, Fov, GimbalCommand, Hand, HandRange, ObjectFraming, PRESET_NAME_MAX,
    PRESET_SLOTS, PresetPosition, TargetBox, TrackingMode, TrackingSpeed, VoiceRange,
    ZOOM_RATIO_MAX,
};

/// GUID of the extension unit, as the UVC descriptor stores it.
#[cfg(windows)]
pub const XU_GUID: u128 = 0x9a1e7291_6843_4683_6d92_39bc7906ee49;
/// Extension unit control that takes messages and returns replies.
pub const SELECTOR_COMMAND: u32 = 2;
/// Extension unit control for field of view and camera status.
pub const SELECTOR_VIEW: u32 = 6;
/// Every message is padded to this size.
pub const MESSAGE_LEN: usize = 60;
const HEADER_LEN: usize = 16;
/// Largest data part that fits in one message.
pub const DATA_MAX: usize = MESSAGE_LEN - HEADER_LEN;

const FLAG_DATA: u8 = 0x20;
/// Data in protobuf form; it carries a length like [`FLAG_DATA`].
const FLAG_PROTOBUF: u8 = 0x40;
const FLAG_NO_REPLY: u8 = 0x04;
const FLAG_REPLY: u8 = 0x08;
/// The address the official app sends from.
pub const HOST: u8 = 0x0a;
/// The gimbal's address.
pub const GIMBAL: u8 = 0x04;
const CAMERA: u8 = 0x02;

/// Begins (data `0`) or ends (data `1`) manual gimbal movement.
pub const CMD_MANUAL_MOVE: [u8; 2] = [0x44, 0x02];
/// Gimbal speed: three `f32`, roll (always 0), pitch and yaw.
pub const CMD_SPEED: [u8; 2] = [0x84, 0x64];

pub const READ_ZOOM: [u8; 2] = [0x04, 0x68];
pub const READ_POSITION: [u8; 2] = [0x04, 0x66];
pub const LIST_PRESETS: [u8; 2] = [0x04, 0x3d];
pub const READ_PRESET_NAME: [u8; 2] = [0xc4, 0x3c];
pub const READ_PRESET_POSITION: [u8; 2] = [0x44, 0x3a];
/// Reads (`84 54`) and writes (`44 54`) a numbered gimbal setting; see
/// `docs/clean-room/2026-11-21-tracking-speed.md`.
pub const READ_SETTING: [u8; 2] = [0x84, 0x54];
const WRITE_SETTING: [u8; 2] = [0x44, 0x54];
/// Setting the official app writes as 0 before every speed change (meaning
/// unknown).
const SETTING_BEFORE_SPEED: u32 = 4;
const SETTING_TRACKING_SPEED: u32 = 5;
/// Reads (`84 20`) and writes (`44 20`) a Hand Tracking setting: keys 0–3
/// the range, key 4 the hand; see `docs/clean-room/2026-11-22-ai-mode.md`.
pub const READ_HAND_SETTING: [u8; 2] = [0x84, 0x20];
const WRITE_HAND_SETTING: [u8; 2] = [0x44, 0x20];
pub const HAND_SIDE: u32 = 4;
/// Reads (`04 25`) and writes (`c4 24`) the Hand Tracking return position.
/// Composition settings (`READ_SETTING` / `WRITE_SETTING` keys); see
/// `docs/clean-room/2026-11-22-preview-tracking.md`.
pub const SETTING_FACE_FRAMING: u32 = 0x0e;
pub const SETTING_COMPOSITION_X: u32 = 0x0f;
pub const SETTING_COMPOSITION_Y: u32 = 0x11;
/// How far the official app lets the composition lines move from the centre.
pub const COMPOSITION_LIMIT: f32 = 0.16;
const SELECT_TARGET: [u8; 2] = [0x84, 0x06];
const OBJECT_FRAMING: [u8; 2] = [0xc4, 0x06];

pub const READ_RETURN_POSITION: [u8; 2] = [0x04, 0x25];
const WRITE_RETURN_POSITION: [u8; 2] = [0xc4, 0x24];

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub flags: u8,
    pub seq: u8,
    pub channel: u8,
    pub source: u8,
    pub destination: u8,
    pub command: [u8; 2],
    pub data: Vec<u8>,
}

/// CRC-16 with the reflected 0x8005 polynomial, initial value and final XOR
/// 0xffff.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &b in bytes {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn data_crc(data: &[u8]) -> u16 {
    if data.is_empty() {
        return 0;
    }
    let mut covered = Vec::with_capacity(data.len() + 4);
    covered.extend_from_slice(&(data.len() as u16).to_le_bytes());
    covered.extend_from_slice(&[0, 0]);
    covered.extend_from_slice(data);
    crc16(&covered)
}

impl Message {
    /// A message from the host to `destination`.
    pub fn new(destination: u8, command: [u8; 2], data: Vec<u8>) -> Self {
        Self {
            flags: 0x01 | FLAG_NO_REPLY | if data.is_empty() { 0 } else { FLAG_DATA },
            seq: 0,
            channel: 0,
            source: HOST,
            destination,
            command,
            data,
        }
    }

    /// A message whose data is in protobuf form (flag `0x40`).
    pub fn protobuf(destination: u8, command: [u8; 2], data: Vec<u8>) -> Self {
        Self {
            flags: 0x01 | FLAG_NO_REPLY | FLAG_PROTOBUF,
            ..Self::new(destination, command, data)
        }
    }

    pub fn requesting_reply(mut self) -> Self {
        self.flags &= !FLAG_NO_REPLY;
        self
    }

    pub fn matches_reply(&self, request: &Self) -> bool {
        self.flags & FLAG_REPLY != 0
            && self.seq == request.seq
            && self.channel == request.channel
            && self.source == request.destination
            && self.destination == request.source
            && self.command == request.command
    }

    /// The padded wire form; data beyond [`DATA_MAX`] bytes is cut off.
    pub fn encode(&self) -> [u8; MESSAGE_LEN] {
        let data = &self.data[..self.data.len().min(DATA_MAX)];
        let mut m = [0u8; MESSAGE_LEN];
        m[..6].copy_from_slice(&[0xaa, self.flags, self.seq, self.channel, 12, 0]);
        m[8..12].copy_from_slice(&[
            self.source,
            self.destination,
            self.command[0],
            self.command[1],
        ]);
        m[12..14].copy_from_slice(&(data.len() as u16).to_le_bytes());
        m[14..16].copy_from_slice(&data_crc(data).to_le_bytes());
        m[HEADER_LEN..HEADER_LEN + data.len()].copy_from_slice(data);
        let header = crc16(&m[..12]);
        m[6..8].copy_from_slice(&header.to_le_bytes());
        m
    }

    /// Parses a block read from the camera; `None` when it is not a whole,
    /// intact message.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let header: &[u8; 12] = bytes.get(..12)?.try_into().ok()?;
        if header[0] != 0xaa || header[4..6] != [12, 0] {
            return None;
        }
        let mut zeroed = [0u8; 12];
        zeroed.copy_from_slice(header);
        zeroed[6..8].fill(0);
        if crc16(&zeroed) != u16::from_le_bytes([header[6], header[7]]) {
            return None;
        }
        let data = if header[1] & (FLAG_DATA | FLAG_PROTOBUF) != 0 {
            let &[lo, hi, crc_lo, crc_hi] = bytes.get(12..HEADER_LEN)? else {
                return None;
            };
            let len = u16::from_le_bytes([lo, hi]) as usize;
            let data = bytes.get(HEADER_LEN..HEADER_LEN + len)?;
            if data_crc(data) != u16::from_le_bytes([crc_lo, crc_hi]) {
                return None;
            }
            data.to_vec()
        } else {
            Vec::new()
        };
        Some(Self {
            flags: header[1],
            seq: header[2],
            channel: header[3],
            source: header[8],
            destination: header[9],
            command: [header[10], header[11]],
            data,
        })
    }
}

/// Numbers outgoing messages the way the official app does: one running
/// sequence byte per channel.
#[derive(Default)]
pub struct Sender {
    seq: u8,
}

impl Sender {
    /// The channel Gimbly sends on (the official app used 1, 2, 12 and 13).
    const CHANNEL: u8 = 1;

    /// Stamps `message` with the next sequence number.
    pub fn number(&mut self, message: &mut Message) {
        (message.seq, message.channel) = (self.seq, Self::CHANNEL);
        self.seq = self.seq.wrapping_add(1);
    }
}

/// The message for a gimbal command, or `None` when that command is not
/// supported yet.
pub fn for_gimbal(command: &GimbalCommand) -> Option<Message> {
    use GimbalCommand::*;
    match *command {
        Drag(active) => Some(manual_move(active)),
        Speed { pitch, yaw } => Some(speed(pitch, yaw)),
        Reset => Some(Message::new(GIMBAL, [0x04, 0x39], Vec::new())),
        ResetWithZoom => Some(Message::new(GIMBAL, [0x04, 0x39], vec![1])),
        MoveTo { pitch, yaw } => Some(move_to(pitch, yaw)),
        TriggerPreset(id) => {
            let mut data = preset_id(id);
            data.extend([1.0f32; 4].into_iter().flat_map(f32::to_le_bytes));
            Some(Message::new(GIMBAL, [0xc4, 0x39], data))
        }
        DeletePreset(id) => Some(Message::new(GIMBAL, [0x84, 0x39], preset_id(id))),
        _ => None,
    }
}

/// Turns to absolute angles in degrees; the official app sends -1000 for
/// the roll and for an axis it keeps.
pub fn move_to(pitch: Option<f32>, yaw: Option<f32>) -> Message {
    const KEEP: f32 = -1000.0;
    let data = [KEEP, pitch.unwrap_or(KEEP), yaw.unwrap_or(KEEP)]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    Message::new(GIMBAL, [0x44, 0x64], data)
}

/// Starts (`true`) or ends manual gimbal movement.
pub fn manual_move(active: bool) -> Message {
    Message::new(GIMBAL, CMD_MANUAL_MOVE, vec![u8::from(!active)])
}

/// Turns the gimbal at `pitch` and `yaw` (the units the official app sends;
/// see `crate::gimbal`).
pub fn speed(pitch: f32, yaw: f32) -> Message {
    let data = [0.0f32, pitch, yaw]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    Message::new(GIMBAL, CMD_SPEED, data)
}

pub fn sleep(sleeping: bool) -> Message {
    Message::new(
        CAMERA,
        [0xc2, 0xa0],
        u32::from(sleeping).to_le_bytes().to_vec(),
    )
}

pub fn field_of_view(fov: Fov) -> [u8; MESSAGE_LEN] {
    let mut data = [0; MESSAGE_LEN];
    data[..3].copy_from_slice(&[
        4,
        1,
        match fov {
            Fov::Wide => 0,
            Fov::Medium => 1,
            Fov::Narrow => 2,
        },
    ]);
    data
}

/// Selector 6 block of AI tracking command `0x16`: a sub-command and its
/// value; see `docs/clean-room/2026-11-21-group-mode-auto-zoom.md`.
fn ai_block(sub: u8, value: u8) -> [u8; MESSAGE_LEN] {
    let mut data = [0; MESSAGE_LEN];
    data[..4].copy_from_slice(&[0x16, 2, sub, value]);
    data
}

/// Turns Human Tracking on in `mode` with the `zoom` framing, or off with
/// `None`. The start command carries the framing; without it the camera
/// tracks without zooming.
pub fn human_tracking(mode: Option<TrackingMode>, zoom: AutoZoom) -> Vec<[u8; MESSAGE_LEN]> {
    match mode {
        None => vec![ai_block(0, 0)],
        Some(mode) => vec![ai_block(2, auto_zoom_value(zoom)), mode_block(mode)],
    }
}

fn mode_block(mode: TrackingMode) -> [u8; MESSAGE_LEN] {
    ai_block(0x0e, (mode == TrackingMode::Group).into())
}

fn auto_zoom_value(zoom: AutoZoom) -> u8 {
    match zoom {
        AutoZoom::Off => 0,
        AutoZoom::FullBody => 1,
        AutoZoom::UpperBody => 2,
        AutoZoom::CloseUp => 3,
        AutoZoom::Headless => 5,
        AutoZoom::LowerBody => 6,
    }
}

/// Selects the Auto Zoom framing. While tracking runs (`tracking` is
/// `Some`), the official app also restarts tracking with the new framing
/// and repeats the mode; the framing only takes effect then. `Some(None)`
/// means tracking runs in a mode Gimbly did not set, so the mode is left out.
pub fn auto_zoom(zoom: AutoZoom, tracking: Option<Option<TrackingMode>>) -> Vec<[u8; MESSAGE_LEN]> {
    let value = auto_zoom_value(zoom);
    let mut blocks = vec![ai_block(0x0f, value)];
    if let Some(mode) = tracking {
        blocks.push(ai_block(2, value));
        blocks.extend(mode.map(mode_block));
    }
    blocks
}

/// The Auto Zoom framing in the status block; `None` for values never
/// observed.
pub fn read_auto_zoom(data: &[u8]) -> Option<AutoZoom> {
    sleeping(data)?;
    let value = *data.get(42)?;
    AutoZoom::ALL
        .into_iter()
        .find(|&zoom| auto_zoom_value(zoom) == value)
}

fn tracking_speed_value(speed: TrackingSpeed) -> u32 {
    match speed {
        TrackingSpeed::Slow => 0,
        TrackingSpeed::Standard => 2,
        TrackingSpeed::Fast => 3,
    }
}

/// The two writes the official app sends for one Tracking Speed click.
pub fn tracking_speed(speed: TrackingSpeed) -> [Message; 2] {
    let write = |setting: u32, value: u32| {
        let data = [0, setting, value]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        Message::new(GIMBAL, WRITE_SETTING, data)
    };
    [
        write(SETTING_BEFORE_SPEED, 0),
        write(SETTING_TRACKING_SPEED, tracking_speed_value(speed)),
    ]
}

/// Request data for [`READ_SETTING`] that asks for the tracking speed.
pub fn read_tracking_speed_request() -> Vec<u8> {
    [0, SETTING_TRACKING_SPEED]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}

/// The speed in a [`READ_SETTING`] reply; `None` for values never observed
/// (the app shows no speed for `1`).
pub fn read_tracking_speed(data: &[u8]) -> Option<TrackingSpeed> {
    let value = u32::from_le_bytes(data.try_into().ok()?);
    TrackingSpeed::ALL
        .into_iter()
        .find(|&speed| tracking_speed_value(speed) == value)
}

/// Request data for [`READ_SETTING`] that asks for setting `key`.
pub fn read_setting_request(key: u32) -> Vec<u8> {
    [0, key].into_iter().flat_map(u32::to_le_bytes).collect()
}

fn write_setting(key: u32, value: &[u8]) -> Message {
    let mut data = read_setting_request(key);
    data.extend_from_slice(value);
    Message::new(GIMBAL, WRITE_SETTING, data)
}

/// A composition offset kept within [`COMPOSITION_LIMIT`].
pub fn clamp_composition(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-COMPOSITION_LIMIT, COMPOSITION_LIMIT)
    } else {
        0.0
    }
}

/// The two writes the official app sends when a composition line is
/// released: the subject's offset from the centre, in fractions of the
/// picture, horizontal then vertical.
pub fn composition(x: f32, y: f32) -> [Message; 2] {
    [
        write_setting(SETTING_COMPOSITION_X, &clamp_composition(x).to_le_bytes()),
        write_setting(SETTING_COMPOSITION_Y, &clamp_composition(y).to_le_bytes()),
    ]
}

pub fn face_framing(on: bool) -> Message {
    write_setting(SETTING_FACE_FRAMING, &[on.into()])
}

/// A composition offset from a [`READ_SETTING`] reply; `None` unless it is
/// a plausible value.
pub fn read_composition(data: &[u8]) -> Option<f32> {
    let value = f32::from_le_bytes(data.try_into().ok()?);
    (value.is_finite() && value.abs() <= 1.0).then(|| clamp_composition(value))
}

pub fn read_face_framing(data: &[u8]) -> Option<bool> {
    match data {
        [0] => Some(false),
        [1] => Some(true),
        _ => None,
    }
}

fn select(kind: [i16; 4], values: [f32; 4]) -> Message {
    let data = kind
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .chain(values.into_iter().flat_map(f32::to_le_bytes))
        .collect();
    Message::new(GIMBAL, SELECT_TARGET, data)
}

/// A fraction of the picture, kept inside it.
fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// Track the person at `(x, y)`, in fractions of the camera picture. The
/// official app sends this for a double-click.
pub fn select_point(x: f32, y: f32) -> Message {
    // 101, 99 and -2 are copied from the capture; their meaning is unknown.
    select([2, 101, 99, -2], [unit(x), unit(y), 0.0, 0.0])
}

/// Track the object inside `target` (object tracking).
pub fn select_object(target: TargetBox) -> Message {
    let TargetBox { x0, y0, x1, y1 } = target;
    let (x0, y0, x1, y1) = (unit(x0), unit(y0), unit(x1), unit(y1));
    select(
        [3, 2, 0, -2],
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)],
    )
}

/// Stop tracking the current target. This is the double-click cancel form;
/// the official app repeats the last point after it, which we leave zero.
pub fn cancel_target() -> Message {
    select([-1, -1, 0, -2], [0.0; 4])
}

pub fn object_framing(framing: ObjectFraming) -> Message {
    let mut data = vec![0; 8];
    data[0] = match framing {
        ObjectFraming::Normal => 0,
        ObjectFraming::CloseUp => 3,
    };
    Message::new(GIMBAL, OBJECT_FRAMING, data)
}

/// The object framing in status offset 28 while object tracking runs;
/// `None` when it does not (or for values never observed).
pub fn read_object_framing(data: &[u8]) -> Option<ObjectFraming> {
    if read_ai_mode(data)? != AiMode::Human {
        return None;
    }
    match *data.get(28)? {
        0x14 => Some(ObjectFraming::Normal),
        0x17 => Some(ObjectFraming::CloseUp),
        _ => None,
    }
}

/// The 12-byte message the official app writes to the CDC serial port every
/// 500 ms: command `42 ae` to `0x02`, no data.
pub fn target_keepalive() -> [u8; 12] {
    let block = Message::new(CAMERA, [0x42, 0xae], Vec::new()).encode();
    let mut bytes = [0; 12];
    bytes.copy_from_slice(&block[..12]);
    bytes
}

/// Largest serial message data accepted; longer lengths mean a lost sync.
const SERIAL_DATA_MAX: usize = 1024;
/// Bytes kept while waiting for the rest of a message.
const SERIAL_BUFFER_MAX: usize = 4 * (HEADER_LEN + SERIAL_DATA_MAX);

/// Appends serial bytes to `buffer` and returns the whole messages in it,
/// skipping bytes that do not start a valid message.
pub fn split_serial(buffer: &mut Vec<u8>, bytes: &[u8]) -> Vec<Message> {
    buffer.extend_from_slice(bytes);
    let mut messages = Vec::new();
    let mut start = 0;
    while let Some(rest) = buffer.get(start..) {
        let Some(at) = rest.iter().position(|&b| b == 0xaa) else {
            start = buffer.len();
            break;
        };
        start += at;
        let Some(header) = buffer.get(start..start + 12) else {
            break;
        };
        let valid = header[4..6] == [12, 0] && {
            let mut zeroed = [0u8; 12];
            zeroed.copy_from_slice(header);
            zeroed[6..8].fill(0);
            crc16(&zeroed) == u16::from_le_bytes([header[6], header[7]])
        };
        if !valid {
            start += 1;
            continue;
        }
        let len = if header[1] & (FLAG_DATA | FLAG_PROTOBUF) != 0 {
            let Some(&[lo, hi]) = buffer.get(start + 12..start + 14) else {
                break;
            };
            let len = usize::from(u16::from_le_bytes([lo, hi]));
            if len > SERIAL_DATA_MAX {
                start += 1;
                continue;
            }
            HEADER_LEN + len
        } else {
            12
        };
        let Some(frame) = buffer.get(start..start + len) else {
            break;
        };
        match Message::decode(frame) {
            Some(message) => {
                messages.push(message);
                start += len;
            }
            None => start += 1,
        }
    }
    buffer.drain(..start.min(buffer.len()));
    if buffer.len() > SERIAL_BUFFER_MAX {
        buffer.clear();
    }
    messages
}

/// Destination and command the official app sent once when it connected,
/// right before the camera started the target stream (no data; the reply
/// was `u32 0`).
pub const TARGET_STREAM_START: (u8, [u8; 2]) = (CAMERA, [0x42, 0xc4]);

/// Reads the camera's report settings record (protobuf, `82 ad` to `0x02`,
/// data `0a 01 01` as the official app sends it).
pub fn read_report_settings() -> Message {
    Message::protobuf(CAMERA, [0x82, 0xad], vec![0x0a, 0x01, 0x01]).requesting_reply()
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_field(out: &mut Vec<u8>, number: u64, value: &Field<'_>) {
    match value {
        Field::Varint(v) => {
            put_varint(out, number << 3);
            put_varint(out, *v);
        }
        Field::Bytes(bytes) => {
            put_varint(out, number << 3 | 2);
            put_varint(out, bytes.len() as u64);
            out.extend_from_slice(bytes);
        }
        Field::Fixed(bytes) => {
            put_varint(out, number << 3 | if bytes.len() == 8 { 1 } else { 5 });
            out.extend_from_slice(bytes);
        }
    }
}

/// The write (`c2 ad`) that sets the camera's target report, from the reply
/// to [`read_report_settings`]: the same record with field 2.4 = 1 (the
/// stream runs; the camera started it 0.4 s after this) and field 2.1 =
/// `tracking` (the stream carries the target's box). The official app wrote
/// it after every tracking change. `None` when nothing would change or the
/// reply cannot be read.
pub fn target_report(reply: &[u8], tracking: bool) -> Option<Message> {
    let mut record = None;
    protobuf_fields(reply, |number, value| {
        if let (1, Field::Bytes(bytes)) = (number, value) {
            record = Some(bytes);
        }
    })?;
    let record = record?;
    let mut fields = Vec::new();
    protobuf_fields(record, |number, value| fields.push((number, value)))?;
    if !fields
        .iter()
        .any(|(number, value)| *number == 2 && matches!(value, Field::Bytes(_)))
    {
        fields.push((2, Field::Bytes(&[])));
    }
    let mut out = Vec::new();
    for (number, value) in fields {
        let (2, Field::Bytes(inner)) = (number, value) else {
            put_field(&mut out, number, &value);
            continue;
        };
        let mut report = Vec::new();
        if tracking {
            put_field(&mut report, 1, &Field::Varint(1));
        }
        protobuf_fields(inner, |number, value| {
            if number != 1 && number != 4 {
                put_field(&mut report, number, &value);
            }
        })?;
        put_field(&mut report, 4, &Field::Varint(1));
        put_field(&mut out, 2, &Field::Bytes(&report));
    }
    (out != record).then(|| Message::protobuf(CAMERA, [0xc2, 0xad], out))
}

/// The command the camera streams the tracked target with.
pub const TARGET_STREAM: [u8; 2] = [0x02, 0xae];

fn varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// A protobuf field value: a varint, or the bytes of a fixed-size or
/// length-delimited field.
#[derive(Clone, Copy)]
enum Field<'a> {
    Varint(u64),
    /// Length-delimited: bytes, a string or a nested message.
    Bytes(&'a [u8]),
    /// A 32- or 64-bit value (`f32`, `fixed32`, ...).
    Fixed(&'a [u8]),
}

/// Calls `field` with each field number and value; `None` when the input
/// is malformed.
fn protobuf_fields<'a>(mut bytes: &'a [u8], mut field: impl FnMut(u64, Field<'a>)) -> Option<()> {
    while !bytes.is_empty() {
        let key = varint(&mut bytes)?;
        let (size, fixed) = match key & 7 {
            0 => {
                field(key >> 3, Field::Varint(varint(&mut bytes)?));
                continue;
            }
            1 => (8, true),
            2 => (usize::try_from(varint(&mut bytes)?).ok()?, false),
            5 => (4, true),
            _ => return None,
        };
        let (value, rest) = bytes.split_at_checked(size)?;
        bytes = rest;
        field(
            key >> 3,
            if fixed {
                Field::Fixed(value)
            } else {
                Field::Bytes(value)
            },
        );
    }
    Some(())
}

/// The boxes of the tracked targets in a [`TARGET_STREAM`] message, in
/// fractions of the camera picture; `None` when the data is malformed.
pub fn read_targets(data: &[u8]) -> Option<Vec<TargetBox>> {
    let mut targets = Vec::new();
    let mut valid = true;
    protobuf_fields(data, |number, value| {
        let (1, Field::Bytes(entry)) = (number, value) else {
            return;
        };
        let mut corners = [0.0f32; 4];
        let mut has_box = false;
        let mut none = false;
        valid &= protobuf_fields(entry, |number, value| match (number, value) {
            // A cancelled target is reported with id 255 and no box.
            (1, Field::Varint(255)) => none = true,
            (6..=9, Field::Fixed(&[a, b, c, d])) => {
                let value = f32::from_le_bytes([a, b, c, d]);
                if value.is_finite()
                    && let Some(corner) = corners.get_mut((number - 6) as usize)
                {
                    *corner = value.clamp(0.0, 1.0);
                    has_box = true;
                }
            }
            _ => {}
        })
        .is_some();
        let [x0, y0, x1, y1] = corners;
        if has_box && !none && x1 > x0 && y1 > y0 {
            targets.push(TargetBox { x0, y0, x1, y1 });
        }
    })?;
    valid.then_some(targets)
}

pub fn sleeping(data: &[u8]) -> Option<bool> {
    match data {
        [46, 0, state @ 0..=1, ..] if data.len() >= 46 => Some(*state == 1),
        _ => None,
    }
}

fn ai_mode_value(mode: AiMode) -> u8 {
    match mode {
        AiMode::Off => 0,
        AiMode::Human => 2,
        AiMode::Hand => 3,
        AiMode::Voice => 7,
    }
}

/// Starts an AI mode, or stops the running one with [`AiMode::Off`]. Human
/// Tracking also needs its framing; see [`human_tracking`].
pub fn ai_mode(mode: AiMode) -> [u8; MESSAGE_LEN] {
    ai_block(ai_mode_value(mode), 0)
}

/// The running AI mode in the status block; `None` for unknown values and
/// for `6`, which shows for under 2 s while the camera switches modes.
pub fn read_ai_mode(data: &[u8]) -> Option<AiMode> {
    sleeping(data)?;
    let value = *data.get(24)?;
    [AiMode::Off, AiMode::Human, AiMode::Hand, AiMode::Voice]
        .into_iter()
        .find(|&mode| ai_mode_value(mode) == value)
}

fn view_block(bytes: &[u8]) -> [u8; MESSAGE_LEN] {
    let mut data = [0; MESSAGE_LEN];
    data[..bytes.len()].copy_from_slice(bytes);
    data
}

pub fn voice_range(range: VoiceRange) -> [u8; MESSAGE_LEN] {
    view_block(&[0x24, 1, (range == VoiceRange::Forward).into()])
}

/// The Voice Tracking Range in status offset 46: bit 1 is Forward, bit 0
/// Sound Source Assisted Tracking. Other bits were never seen.
pub fn read_voice_range(data: &[u8]) -> Option<VoiceRange> {
    sleeping(data)?;
    match *data.get(46)? {
        0 | 1 => Some(VoiceRange::Omni),
        2 | 3 => Some(VoiceRange::Forward),
        _ => None,
    }
}

pub fn sound_source(on: bool) -> [u8; MESSAGE_LEN] {
    view_block(&[0x25, 1, on.into()])
}

/// Sound Source Assisted Tracking: bit 0 of status offset 46 (see
/// [`read_voice_range`]).
pub fn read_sound_source(data: &[u8]) -> Option<bool> {
    sleeping(data)?;
    match *data.get(46)? {
        value @ 0..=3 => Some(value & 1 == 1),
        _ => None,
    }
}

fn hand_setting(key: u32, value: &[u8]) -> Message {
    let mut data = key.to_le_bytes().to_vec();
    data.extend_from_slice(value);
    Message::new(GIMBAL, WRITE_HAND_SETTING, data)
}

/// Request data for [`READ_HAND_SETTING`].
pub fn hand_setting_key(key: u32) -> Vec<u8> {
    key.to_le_bytes().to_vec()
}

pub fn hand(hand: Hand) -> Message {
    hand_setting(HAND_SIDE, &[(hand == Hand::Left).into()])
}

pub fn read_hand(data: &[u8]) -> Option<Hand> {
    match data {
        [1] => Some(Hand::Left),
        [0] => Some(Hand::Right),
        _ => None,
    }
}

/// One range limit in degrees; `None` unless it is a plausible angle.
pub fn read_hand_limit(data: &[u8]) -> Option<f32> {
    let value = f32::from_le_bytes(data.try_into().ok()?);
    (value.is_finite() && value.abs() <= 360.0).then_some(value)
}

/// The return record the camera reported with none stored: id `ffffffff`,
/// shown as P 0, T 0, Z 1.0.
pub const RETURN_RECORD_NONE: [u8; 24] = [
    0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0, 0, 0, 100, 0, 100, 0, 100, 0, 0, 0,
];

pub fn read_return_record(data: &[u8]) -> Option<[u8; 24]> {
    data.try_into().ok()
}

/// Yaw and pitch in degrees and the zoom ratio of a return record; id
/// `ffffffff` means none is stored, shown as the home position.
pub fn return_position(record: &[u8; 24]) -> (f32, f32, f32) {
    if record[..4] == [0xff; 4] {
        return (0.0, 0.0, 1.0);
    }
    let value = |at: usize| f32::from(i16::from_le_bytes([record[at], record[at + 1]])) / 100.0;
    (value(4), value(6), value(10))
}

/// `record` with a new position, from [`current_angles`] and a zoom ratio.
pub fn set_return_position(
    record: &[u8; 24],
    (yaw, pitch): (i16, i16),
    ratio: f32,
) -> Option<[u8; 24]> {
    let mut record = *record;
    // shortcut: the id a newly set position needs is unknown (Update was
    // not captured); 0 replaces "none stored". Check with a capture.
    if record[..4] == [0xff; 4] {
        record[..4].fill(0);
    }
    record[4..6].copy_from_slice(&yaw.to_le_bytes());
    record[6..8].copy_from_slice(&pitch.to_le_bytes());
    record[10..12].copy_from_slice(&zoom_units(ratio)?.to_le_bytes());
    Some(record)
}

/// The official app keeps opposite limits at least 10° apart.
pub fn hand_limits_valid(limits: &[f32; 4]) -> bool {
    limits[1] - limits[0] >= 10.0 && limits[3] - limits[2] >= 10.0
}

/// Selector 6 block that enters (`true`) or leaves the range-edit state.
pub fn hand_range_edit(editing: bool) -> [u8; MESSAGE_LEN] {
    view_block(&[0x18, 1, editing.into()])
}

/// The gimbal message sent after entering (`true`) or before leaving the
/// range-edit state.
pub fn hand_range_gimbal(editing: bool) -> Message {
    Message::new(GIMBAL, [0xc4, 0x26], vec![u8::from(!editing)])
}

/// Messages that follow [`hand_range_edit`]`(false)`: Done writes `save`,
/// Cancel passes `None`. Both end the edit state and zoom back to 1.0x.
pub fn hand_range_close(save: Option<&HandRange>) -> Vec<Message> {
    let mut messages = Vec::new();
    if let Some(range) = save {
        for (key, limit) in (0u32..).zip(range.limits) {
            messages.push(hand_setting(key, &limit.to_le_bytes()));
        }
        let mut record = range.record;
        // The official app wrote this field (read as 100) as 0; meaning unknown.
        record[20..22].fill(0);
        messages.push(Message::new(GIMBAL, WRITE_RETURN_POSITION, record.to_vec()));
    }
    messages.push(hand_range_gimbal(false));
    let mut zoom = 0x1e_u32.to_le_bytes().to_vec();
    zoom.extend_from_slice(&1.0f32.to_le_bytes());
    messages.push(Message::new(GIMBAL, [0x84, 0x68], zoom));
    messages
}

pub fn zoom(ratio: f32) -> Option<(Message, f32)> {
    let zoom = zoom_units(ratio)?;
    let data = [8, zoom as u32]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    Some((
        Message::new(CAMERA, [0x42, 0x19], data),
        f32::from(zoom) / 100.0,
    ))
}

fn zoom_units(ratio: f32) -> Option<i16> {
    (1.0..=ZOOM_RATIO_MAX)
        .contains(&ratio)
        .then(|| (ratio * 100.0).round() as i16)
}

pub fn read_zoom(data: &[u8]) -> Option<f32> {
    let ratio = f32::from_le_bytes(data.try_into().ok()?);
    zoom_units(ratio).map(|_| ratio)
}

pub fn preset_id(id: u8) -> Vec<u8> {
    u32::from(id).to_le_bytes().to_vec()
}

pub fn preset_ids(data: &[u8]) -> Option<&[u8]> {
    if data.len() > PRESET_SLOTS
        || data
            .iter()
            .enumerate()
            .any(|(i, id)| data[..i].contains(id))
    {
        return None;
    }
    Some(data)
}

pub fn preset_name(id: u8, name: &str) -> Result<Message, String> {
    if name.trim().is_empty()
        || name.len() > PRESET_NAME_MAX
        || !name.bytes().all(|b| (b' '..=b'~').contains(&b))
    {
        return Err(format!(
            "Use 1 to {PRESET_NAME_MAX} printable ASCII characters for a preset name"
        ));
    }
    let mut data = preset_id(id);
    data.extend_from_slice(name.as_bytes());
    Ok(Message::new(GIMBAL, [0x84, 0x3a], data))
}

pub fn read_preset_name(data: &[u8]) -> Option<String> {
    if data.len() > PRESET_NAME_MAX || !data.iter().all(|b| (b' '..=b'~').contains(b)) {
        return None;
    }
    Some(std::str::from_utf8(data).ok()?.to_owned())
}

pub fn read_preset_position(data: &[u8], id: u8) -> Option<PresetPosition> {
    let &[
        id_byte,
        0,
        0,
        0,
        a0,
        a1,
        b0,
        b1,
        roll0,
        roll1,
        zoom0,
        zoom1,
        ..,
    ] = data
    else {
        return None;
    };
    if data.len() != 24 || id_byte != id {
        return None;
    }
    Some(PresetPosition {
        id,
        yaw: f32::from(i16::from_le_bytes([a0, a1])) / 100.0,
        pitch: f32::from(i16::from_le_bytes([b0, b1])) / 100.0,
        roll: f32::from(i16::from_le_bytes([roll0, roll1])) / 100.0,
        zoom: f32::from(i16::from_le_bytes([zoom0, zoom1])) / 100.0,
    })
}

/// Yaw and pitch in hundredths of a degree from a [`READ_POSITION`] reply.
pub fn current_angles(current: &[u8]) -> Option<(i16, i16)> {
    let &[_, _, _, _, _, _, _, _, pitch0, pitch1, yaw0, yaw1, ..] = current else {
        return None;
    };
    if current.len() != 24 {
        return None;
    }
    Some((
        i16::from_le_bytes([yaw0, yaw1]).checked_mul(10)?,
        i16::from_le_bytes([pitch0, pitch1]).checked_mul(10)?,
    ))
}

pub fn store_preset(id: u8, current: &[u8], ratio: f32) -> Option<Message> {
    let (yaw, pitch) = current_angles(current)?;
    let mut data = preset_id(id);
    data.extend(
        [
            yaw,
            pitch,
            0,
            zoom_units(ratio)?,
            -200,
            -200,
            300,
            300,
            0,
            0,
        ]
        .into_iter()
        .flat_map(i16::to_le_bytes),
    );
    Some(Message::new(GIMBAL, [0x44, 0x39], data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    // Blocks the official app sent in the capture of 2026-11-08 (padding
    // after the data replaced with zeros: the app leaves old bytes there).
    const START: &str = "aa253a010c0067ea0a0444020100e63f00";
    const SPEED: &str = "aa253b010c00e6050a0484640c00b5a500000000308f2d3e308f2dbe";
    const STATUS_REPLY: &str = "aa2943010c0039ab040a84540400bfbf02000000";

    fn padded(text: &str) -> Vec<u8> {
        let mut bytes = hex(text);
        bytes.resize(MESSAGE_LEN, 0);
        bytes
    }

    #[test]
    fn encodes_captured_messages() {
        let mut start = manual_move(true);
        (start.seq, start.channel) = (0x3a, 1);
        assert_eq!(start.encode().to_vec(), padded(START));

        let pitch = f32::from_le_bytes([0x30, 0x8f, 0x2d, 0x3e]);
        let mut turn = speed(pitch, -pitch);
        (turn.seq, turn.channel) = (0x3b, 1);
        assert_eq!(turn.encode().to_vec(), padded(SPEED));
    }

    #[test]
    fn decodes_a_reply() {
        let reply = Message::decode(&padded(STATUS_REPLY)).unwrap();
        assert_eq!(reply.flags, 0x29);
        assert_eq!((reply.source, reply.destination), (GIMBAL, HOST));
        assert_eq!(reply.command, [0x84, 0x54]);
        assert_eq!(reply.data, vec![2, 0, 0, 0]);
    }

    #[test]
    fn round_trips_and_rejects_damage() {
        let message = speed(-12.5, 30.0);
        let bytes = message.encode();
        assert_eq!(Message::decode(&bytes), Some(message));
        for i in [0, 3, 7, 10, 14, 20] {
            let mut bad = bytes;
            bad[i] ^= 0x10;
            assert_eq!(Message::decode(&bad), None, "byte {i}");
        }
        assert_eq!(Message::decode(&bytes[..10]), None);
        let mut long = bytes;
        long[12] = 0xff; // claims more data than the block holds
        assert_eq!(Message::decode(&long), None);
    }

    #[test]
    fn empty_data_has_zero_crc() {
        let bytes = Message::new(GIMBAL, [0x04, 0x01], Vec::new()).encode();
        assert_eq!(&bytes[12..16], &[0, 0, 0, 0]);
        assert!(Message::decode(&bytes).is_some());
    }

    #[test]
    fn empty_commands_ignore_the_previous_payload() {
        let captured = padded("aa0512010c003cb90a04043940000000");
        let mut reset = for_gimbal(&GimbalCommand::Reset).unwrap();
        (reset.seq, reset.channel) = (0x12, 1);
        assert_eq!(Message::decode(&captured), Some(reset.clone()));
        assert_eq!(reset.encode()[..12], captured[..12]);
        assert_eq!(Message::decode(&captured[..12]), Some(reset));
    }

    #[test]
    fn matches_the_numbered_reply() {
        let request = Message::decode(&hex("aa01be020c00bbfc0a040466ffffffff")).unwrap();
        let mut reply = Message::decode(&hex(
            "aa29be020c005977040a04661800b9d9dffff801670400004c0172ff0000ffff0000450050d54500",
        ))
        .unwrap();
        assert!(reply.matches_reply(&request));
        reply.seq = reply.seq.wrapping_sub(1);
        assert!(!reply.matches_reply(&request));
        reply.seq = request.seq;
        reply.channel = 1;
        assert!(!reply.matches_reply(&request));
    }

    #[test]
    fn view_commands_match_capture() {
        let mut message = sleep(true);
        (message.seq, message.channel) = (2, 2);
        assert_eq!(
            message.encode().to_vec(),
            padded("aa2502020c00f1d70a02c2a00400bffb01000000")
        );
        let (mut message, ratio) = zoom(2.5).unwrap();
        assert_eq!(ratio, 2.5);
        (message.seq, message.channel) = (0xa3, 1);
        assert_eq!(
            message.encode().to_vec(),
            padded("aa25a3010c0012120a02421908004b5c08000000fa000000")
        );
        for (index, fov) in Fov::ALL.into_iter().enumerate() {
            assert_eq!(
                field_of_view(fov).to_vec(),
                padded(&format!("0401{index:02x}"))
            );
        }
        let reply = Message::decode(&hex("aa29bf020c008976040a046804009fd70000803f")).unwrap();
        assert_eq!(read_zoom(&reply.data), Some(1.0));
        assert_eq!(read_zoom(&f32::NAN.to_le_bytes()), None);
        assert!(zoom(f32::INFINITY).is_none());

        let awake = hex(
            "2e00000100000001000178000001010001000000017f000000004b030000553c000310000000000008000000030101",
        );
        let asleep = hex(
            "2e00010100000001000378000001010001000000017f000000004b030000003c000310000000000008000000030101",
        );
        assert_eq!(sleeping(&awake), Some(false));
        assert_eq!(sleeping(&asleep), Some(true));
        assert_eq!(sleeping(&asleep[..45]), None);
    }

    #[test]
    fn human_tracking_matches_capture() {
        // Selector 6 blocks from the 2026-11-21 captures.
        let blocks = |mode, zoom| {
            human_tracking(mode, zoom)
                .iter()
                .map(|b| b.to_vec())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            blocks(Some(TrackingMode::Single), AutoZoom::Off),
            [padded("160202"), padded("16020e")]
        );
        assert_eq!(
            blocks(Some(TrackingMode::Group), AutoZoom::Off),
            [padded("160202"), padded("16020e01")]
        );
        assert_eq!(
            blocks(Some(TrackingMode::Single), AutoZoom::CloseUp),
            [padded("16020203"), padded("16020e")]
        );
        assert_eq!(blocks(None, AutoZoom::CloseUp), [padded("1602")]);
        let off = hex(
            "2e00000100000001000178000001010001000000017f000000004b030000001e0003100000000000080000000301",
        );
        let on = hex(
            "2e00000100000001000178000001010001000000017f000002004b030000001e0003100000000000080000000301",
        );
        assert_eq!(read_ai_mode(&off), Some(AiMode::Off));
        assert_eq!(read_ai_mode(&on), Some(AiMode::Human));
        assert_eq!(read_ai_mode(&on[..45]), None);
        assert_eq!(read_ai_mode(&padded("aa")), None);
    }

    #[test]
    fn auto_zoom_matches_capture() {
        // Selector 6 writes and status reads from the 2026-11-21 capture.
        let expected = [
            (AutoZoom::Off, "16020f"),
            (AutoZoom::FullBody, "16020f01"),
            (AutoZoom::UpperBody, "16020f02"),
            (AutoZoom::CloseUp, "16020f03"),
            (AutoZoom::Headless, "16020f05"),
            (AutoZoom::LowerBody, "16020f06"),
        ];
        let status = |value: u8| {
            let mut block = hex(
                "2e00000100000001000178000001010001000000017f000000004b030000553c00031000000000000800000003010100000000000000000000000000",
            );
            block[42] = value;
            block
        };
        let blocks = |zoom, tracking| {
            auto_zoom(zoom, tracking)
                .iter()
                .map(|b| b.to_vec())
                .collect::<Vec<_>>()
        };
        for (zoom, block) in expected {
            assert_eq!(blocks(zoom, None), [padded(block)]);
            assert_eq!(read_auto_zoom(&status(auto_zoom_value(zoom))), Some(zoom));
        }
        // While tracking ran in Single Mode (second capture).
        assert_eq!(
            blocks(AutoZoom::CloseUp, Some(Some(TrackingMode::Single))),
            [padded("16020f03"), padded("16020203"), padded("16020e")]
        );
        assert_eq!(
            blocks(AutoZoom::FullBody, Some(Some(TrackingMode::Group))),
            [padded("16020f01"), padded("16020201"), padded("16020e01")]
        );
        assert_eq!(
            blocks(AutoZoom::FullBody, Some(None)),
            [padded("16020f01"), padded("16020201")]
        );
        assert_eq!(read_auto_zoom(&status(4)), None);
        assert_eq!(read_auto_zoom(&status(1)[..42]), None);
        assert_eq!(read_auto_zoom(&padded("aa")), None);
    }

    #[test]
    fn tracking_speed_matches_capture() {
        // Blocks from the 2026-11-21 tracking speed capture (channel 0).
        let encoded = |speed, seqs: [u8; 2]| {
            tracking_speed(speed)
                .into_iter()
                .zip(seqs)
                .map(|(mut message, seq)| {
                    (message.seq, message.channel) = (seq, 0);
                    message.encode().to_vec()
                })
                .collect::<Vec<_>>()
        };
        let before = |seq: &str, crc: &str| {
            padded(&format!(
                "aa25{seq}000c00{crc}0a0444540c0045f9000000000400000000000000"
            ))
        };
        assert_eq!(
            encoded(TrackingSpeed::Slow, [0x35, 0x36]),
            [
                before("35", "da74"),
                padded("aa2536000c002a7b0a0444540c00843500000000050000000000000000")
            ]
        );
        assert_eq!(
            encoded(TrackingSpeed::Fast, [0x47, 0x48]),
            [
                before("47", "7995"),
                padded("aa2548000c0049a50a0444540c00847100000000050000000300000000")
            ]
        );
        assert_eq!(
            encoded(TrackingSpeed::Standard, [0x59, 0x5a]),
            [
                before("59", "19f5"),
                padded("aa255a000c00e9fa0a0444540c00858d00000000050000000200000000")
            ]
        );

        let mut read =
            Message::new(GIMBAL, READ_SETTING, read_tracking_speed_request()).requesting_reply();
        (read.seq, read.channel) = (0x38, 0);
        assert_eq!(
            read.encode().to_vec(),
            padded("aa2138000c000ebe0a04845408007aee0000000005000000")
        );
        let reply = Message::decode(&hex("aa2938000c0047f5040a84540400be0700000000")).unwrap();
        assert!(reply.matches_reply(&read));
        assert_eq!(read_tracking_speed(&reply.data), Some(TrackingSpeed::Slow));
        assert_eq!(
            read_tracking_speed(&[3, 0, 0, 0]),
            Some(TrackingSpeed::Fast)
        );
        assert_eq!(read_tracking_speed(&[1, 0, 0, 0]), None);
        assert_eq!(read_tracking_speed(&[2, 0, 0]), None);
    }

    #[test]
    fn preset_commands_match_capture() {
        let current = hex("dffff801670400004c0172ff0000ffff0000450050d54500");
        let mut store = store_preset(2, &current, 1.0).unwrap();
        (store.seq, store.channel) = (0xc0, 2);
        assert_eq!(
            store.encode().to_vec(),
            padded(
                "aa25c0020c00172a0a0444391800eab20200000074faf80c0000640038ff38ff2c012c0100000000",
            )
        );
        let mut name = preset_name(2, "Preset3").unwrap();
        (name.seq, name.channel) = (0xc1, 2);
        assert_eq!(
            name.encode().to_vec(),
            padded("aa25c1020c0056ee0a04843a0b0084ae0200000050726573657433")
        );
        let mut go = for_gimbal(&GimbalCommand::TriggerPreset(1)).unwrap();
        (go.seq, go.channel) = (0xa6, 2);
        assert_eq!(
            go.encode().to_vec(),
            padded("aa25a6020c00954b0a04c4391400eb2a010000000000803f0000803f0000803f0000803f",)
        );
        let mut delete = for_gimbal(&GimbalCommand::DeletePreset(2)).unwrap();
        (delete.seq, delete.channel) = (0x87, 3);
        assert_eq!(
            delete.encode().to_vec(),
            padded("aa2587030c00f9b40a0484390400bfbf02000000")
        );
        assert!(preset_name(2, &"a".repeat(PRESET_NAME_MAX + 1)).is_err());
        assert!(preset_name(2, "猫").is_err());
        assert!(store_preset(2, &current[..23], 1.0).is_none());
    }

    #[test]
    fn decodes_preset_replies() {
        let list = Message::decode(&hex("aa295b120c00c10a040a043d030057ae000102")).unwrap();
        assert_eq!(preset_ids(&list.data), Some([0, 1, 2].as_slice()));
        assert_eq!(preset_ids(&[0, 0]), None);
        let name = Message::decode(&hex("aa2943030c0010e5040ac43c070059fe50726573657432")).unwrap();
        assert_eq!(read_preset_name(&name.data).as_deref(), Some("Preset2"));
        let stored = Message::decode(&hex(
            "aa29a5020c0018fe040a443a1800600a0100000074faf80c00006400000000006400640064000000",
        ))
        .unwrap();
        let position = read_preset_position(&stored.data, 1).unwrap();
        assert_eq!(
            (position.yaw, position.pitch, position.roll, position.zoom),
            (-14.2, 33.2, 0.0, 1.0)
        );
        assert!(read_preset_position(&stored.data, 2).is_none());
    }

    /// Checks a gimbal write the way the 2026-11-22 capture showed it.
    fn assert_gimbal_write(message: &Message, command: [u8; 2], data: &str) {
        assert_eq!(message.flags, 0x25);
        assert_eq!((message.source, message.destination), (HOST, GIMBAL));
        assert_eq!(message.command, command);
        assert_eq!(message.data, hex(data));
    }

    #[test]
    fn ai_mode_matches_capture() {
        // Selector 6 writes from the 2026-11-22 capture.
        assert_eq!(ai_mode(AiMode::Voice).to_vec(), padded("16020700"));
        assert_eq!(ai_mode(AiMode::Hand).to_vec(), padded("16020300"));
        assert_eq!(ai_mode(AiMode::Off).to_vec(), padded("16020000"));
        assert_eq!(voice_range(VoiceRange::Omni).to_vec(), padded("240100"));
        assert_eq!(voice_range(VoiceRange::Forward).to_vec(), padded("240101"));

        let status = |mode: u8, range: u8| {
            let mut block = hex(
                "2e00000100000001000178000001010001000000017f000000004b030000553c00031000000000000800000003010100000000000000000000000000",
            );
            (block[24], block[46]) = (mode, range);
            block
        };
        for (value, mode) in [
            (0, Some(AiMode::Off)),
            (2, Some(AiMode::Human)),
            (3, Some(AiMode::Hand)),
            (7, Some(AiMode::Voice)),
            (6, None), // switching between modes
            (1, None),
            (0xff, None),
        ] {
            assert_eq!(read_ai_mode(&status(value, 1)), mode, "offset 24 = {value}");
        }
        for (value, range) in [
            (1, Some(VoiceRange::Omni)),
            (3, Some(VoiceRange::Forward)),
            (0, Some(VoiceRange::Omni)),
            (2, Some(VoiceRange::Forward)),
            (4, None),
            (0x81, None),
        ] {
            assert_eq!(
                read_voice_range(&status(0, value)),
                range,
                "offset 46 = {value}"
            );
        }
        assert_eq!(read_voice_range(&status(0, 3)[..46]), None);

        // Selector 6 writes from the 2026-11-24 capture.
        assert_eq!(sound_source(false).to_vec(), padded("250100"));
        assert_eq!(sound_source(true).to_vec(), padded("250101"));
        for (value, on) in [
            (0, Some(false)),
            (1, Some(true)),
            (2, Some(false)),
            (3, Some(true)),
            (4, None),
            (0x81, None),
        ] {
            assert_eq!(
                read_sound_source(&status(0, value)),
                on,
                "offset 46 = {value}"
            );
        }
        assert_eq!(read_sound_source(&status(0, 1)[..46]), None);

        assert_gimbal_write(&hand(Hand::Left), [0x44, 0x20], "0400000001");
        assert_gimbal_write(&hand(Hand::Right), [0x44, 0x20], "0400000000");
        let read =
            Message::new(GIMBAL, READ_HAND_SETTING, hand_setting_key(HAND_SIDE)).requesting_reply();
        assert_eq!((read.flags, read.data.clone()), (0x21, hex("04000000")));
        assert_eq!(read_hand(&[1]), Some(Hand::Left));
        assert_eq!(read_hand(&[0]), Some(Hand::Right));
        assert_eq!(read_hand(&[2]), None);
        assert_eq!(read_hand(&[]), None);
        assert_eq!(read_hand(&[1, 0]), None);
    }

    #[test]
    fn hand_range_panel_moves_match_capture() {
        // Writes from the 2026-11-22 14:26 capture: clicking the limits
        // -45 / 30 / 45 / -30 and the return position P 44.90° T -30.00°.
        let writes = [
            (move_to(None, Some(-45.0)), "00007ac400007ac4000034c2"),
            (move_to(Some(-30.0), None), "00007ac40000f0c100007ac4"),
            (move_to(None, Some(45.0)), "00007ac400007ac400003442"),
            (move_to(Some(30.0), None), "00007ac40000f04100007ac4"),
            (move_to(Some(30.0), Some(44.9)), "00007ac40000f0419a993342"),
        ];
        for (message, data) in writes {
            assert_gimbal_write(&message, [0x44, 0x64], data);
        }
        let move_to_command = GimbalCommand::MoveTo {
            pitch: None,
            yaw: Some(-45.0),
        };
        assert_eq!(
            for_gimbal(&move_to_command),
            Some(move_to(None, Some(-45.0)))
        );
        // The panel's Reset also zoomed back to 1.0x.
        let reset = for_gimbal(&GimbalCommand::ResetWithZoom).unwrap();
        assert_gimbal_write(&reset, [0x04, 0x39], "01");
        assert_eq!(return_position(&RETURN_RECORD_NONE), (0.0, 0.0, 1.0));
    }

    #[test]
    fn hand_range_matches_capture() {
        // Replies and writes from the 2026-11-22 capture.
        let none =
            read_return_record(&hex("ffffffff0000000000006400000000006400640064000000")).unwrap();
        assert_eq!(none, RETURN_RECORD_NONE);
        assert_eq!(return_position(&none), (0.0, 0.0, 1.0));
        assert_eq!(read_return_record(&none[..23]), None);
        let limits = ["000034c2", "00003442", "0000f0c1", "0000f041"]
            .map(|reply| read_hand_limit(&hex(reply)).unwrap());
        assert_eq!(limits, [-45.0, 45.0, -30.0, 30.0]);
        assert_eq!(read_hand_limit(&f32::NAN.to_le_bytes()), None);
        assert_eq!(read_hand_limit(&400.0f32.to_le_bytes()), None);
        assert_eq!(read_hand_limit(&hex("000034")), None);

        assert_eq!(hand_range_edit(true).to_vec(), padded("180101"));
        assert_eq!(hand_range_edit(false).to_vec(), padded("180100"));
        assert_gimbal_write(&hand_range_gimbal(true), [0xc4, 0x26], "00");

        let range = HandRange {
            limits,
            record: none,
        };
        let done = hand_range_close(Some(&range));
        let expected = [
            ([0x44, 0x20], "00000000000034c2"),
            ([0x44, 0x20], "0100000000003442"),
            ([0x44, 0x20], "020000000000f0c1"),
            ([0x44, 0x20], "030000000000f041"),
            (
                [0xc4, 0x24],
                "ffffffff0000000000006400000000006400640000000000",
            ),
            ([0xc4, 0x26], "01"),
            ([0x84, 0x68], "1e0000000000803f"),
        ];
        assert_eq!(done.len(), expected.len());
        for (message, (command, data)) in done.iter().zip(expected) {
            assert_gimbal_write(message, command, data);
        }
        let cancel = hand_range_close(None);
        assert_eq!(cancel, done[5..]);

        // Displayed: left 45 ≥ right -45 + 10 and top 30 ≥ bottom -30 + 10.
        assert!(hand_limits_valid(&limits));
        assert!(hand_limits_valid(&[-5.0, 5.0, -30.0, 30.0]));
        assert!(!hand_limits_valid(&[-5.0, 4.9, -30.0, 30.0]));
        assert!(!hand_limits_valid(&[-45.0, 45.0, 10.0, 15.0]));

        // A stored position: pan -22.70°, tilt -5.40°, zoom 1.00x.
        let stored =
            read_return_record(&hex("0000000022f7e4fd00006400000000006400640064000000")).unwrap();
        assert_eq!(return_position(&stored), (-22.7, -5.4, 1.0));
        let set = set_return_position(&none, (-2270, -540), 1.0).unwrap();
        assert_eq!(set, stored);
        assert!(set_return_position(&none, (0, 0), f32::NAN).is_none());
    }

    #[test]
    fn target_selection_matches_capture() {
        // Gimbal writes from the 2026-11-22 preview captures.
        let f = |text: &str| f32::from_le_bytes(hex(text).try_into().unwrap());
        assert_gimbal_write(
            &select_point(f("9a99473f"), f("02c93a3f")),
            [0x84, 0x06],
            "020065006300feff9a99473f02c93a3f0000000000000000",
        );
        let target = TargetBox {
            x0: f("cdcc043e"),
            y0: f("c5a8e13d"),
            x1: f("0000483e"),
            y1: f("398ea33e"),
        };
        let captured = "030002000000feffcdcc043ec5a8e13d0000483e398ea33e";
        assert_gimbal_write(&select_object(target), [0x84, 0x06], captured);
        // A box drawn from the far corner sends the same corners.
        let reversed = TargetBox {
            x0: target.x1,
            y0: target.y1,
            x1: target.x0,
            y1: target.y0,
        };
        assert_gimbal_write(&select_object(reversed), [0x84, 0x06], captured);
        assert_gimbal_write(
            &cancel_target(),
            [0x84, 0x06],
            "ffffffff0000feff00000000000000000000000000000000",
        );
        assert_eq!(
            select_point(f32::NAN, 7.0).data[8..16],
            hex("0000003f0000803f")
        );
        assert_gimbal_write(
            &object_framing(ObjectFraming::CloseUp),
            [0xc4, 0x06],
            "0300000000000000",
        );
        assert_gimbal_write(
            &object_framing(ObjectFraming::Normal),
            [0xc4, 0x06],
            "0000000000000000",
        );

        let status = |mode: u8, framing: u8| {
            let mut block = hex(
                "2e00000100000001000178000001010001000000017f000000004b030000553c00031000000000000800000003010100000000000000000000000000",
            );
            (block[24], block[28]) = (mode, framing);
            block
        };
        assert_eq!(
            read_object_framing(&status(2, 0x14)),
            Some(ObjectFraming::Normal)
        );
        assert_eq!(
            read_object_framing(&status(2, 0x17)),
            Some(ObjectFraming::CloseUp)
        );
        assert_eq!(read_object_framing(&status(2, 0)), None);
        assert_eq!(read_object_framing(&status(7, 0x14)), None);
        assert_eq!(read_object_framing(&status(2, 0x14)[..20]), None);
    }

    #[test]
    fn composition_matches_capture() {
        let [x, y] = composition(-0.16, 0.16);
        assert_gimbal_write(&x, [0x44, 0x54], "000000000f0000000ad723be");
        assert_gimbal_write(&y, [0x44, 0x54], "00000000110000000ad7233e");
        // Lines dragged past the thirds stopped at the same values.
        let [x, y] = composition(-0.5, f32::INFINITY);
        assert_gimbal_write(&x, [0x44, 0x54], "000000000f0000000ad723be");
        assert_gimbal_write(&y, [0x44, 0x54], "000000001100000000000000");
        assert_gimbal_write(&face_framing(true), [0x44, 0x54], "000000000e00000001");
        assert_gimbal_write(&face_framing(false), [0x44, 0x54], "000000000e00000000");
        assert_eq!(
            read_setting_request(SETTING_FACE_FRAMING),
            hex("000000000e000000")
        );
        assert_eq!(
            read_setting_request(SETTING_COMPOSITION_Y),
            hex("0000000011000000")
        );
        assert_eq!(
            read_composition(&hex("f8ac8bbd")),
            Some(f32::from_le_bytes(hex("f8ac8bbd").try_into().unwrap()))
        );
        assert_eq!(read_composition(&hex("0000c07f")), None);
        assert_eq!(read_composition(&hex("00")), None);
        assert_eq!(read_face_framing(&hex("01")), Some(true));
        assert_eq!(read_face_framing(&hex("02")), None);
    }

    #[test]
    fn target_stream_matches_capture() {
        assert_eq!(target_keepalive().to_vec(), hex("aa0500000c0002b90a0242ae"));

        // Report settings: read, then written back with field 2.4 = 1; the
        // stream started 0.4 s after this write (2026-11-22, Gimbly test).
        let read = read_report_settings();
        assert_eq!(
            (read.flags, read.destination, read.command),
            (0x41, CAMERA, [0x82, 0xad])
        );
        assert_eq!(read.data, hex("0a0101"));
        // The writes from the captures: stream on, then with the box while
        // tracking, and back without it.
        let write = target_report(&hex("0a0408011200"), false).unwrap();
        assert_eq!(
            (write.flags, write.destination, write.command),
            (0x45, CAMERA, [0xc2, 0xad])
        );
        assert_eq!(write.data, hex("080112022001"));
        let on = target_report(&hex("0a06080112022001"), true).unwrap();
        assert_eq!(on.data, hex("0801120408012001"));
        let off = target_report(&hex("0a080801120408012001"), false).unwrap();
        assert_eq!(off.data, hex("080112022001"));
        // Nothing to change, a missing field 2, damage.
        assert!(target_report(&hex("0a080801120408012001"), true).is_none());
        assert!(target_report(&hex("0a06080112022001"), false).is_none());
        assert_eq!(
            target_report(&hex("0a020801"), false).unwrap().data,
            hex("080112022001")
        );
        assert!(target_report(&hex("0a0908"), true).is_none());
        assert!(target_report(&[], true).is_none());

        // CDC serial frames from the 2026-11-22 capture: a person, an object,
        // a cancelled target and nothing tracked.
        let person = "aa44d7dd0c0091c0020a02ae2600cdf40a22100118013525745d3e3dfa3c153e45c4d0293f4d0000803f5543a689425d0000803f2a00";
        let object = "aa446ae50c0034ec020a02ae2500bc6d0a21100118012001280235cdcc043e3dc5a8e13d450000483e4d398ea33e5528b1ab422a00";
        let cancel = "aa4407e90c00f3a8020a02ae0700d6290a0308ff012a00";
        let empty = "aa4432e90c00e347020a02ae0200e1662a00";
        let stream = hex(&format!("0011{person}aa{object}{cancel}{empty}"));
        // Split anywhere, as USB packets do.
        let mut buffer = Vec::new();
        let mut messages = Vec::new();
        for chunk in stream.chunks(7) {
            messages.extend(split_serial(&mut buffer, chunk));
        }
        assert_eq!(messages.len(), 4);
        assert!(buffer.is_empty());
        let targets: Vec<_> = messages
            .iter()
            .map(|m| {
                assert_eq!(
                    (m.source, m.destination, m.command),
                    (0x02, HOST, TARGET_STREAM)
                );
                read_targets(&m.data).unwrap()
            })
            .collect();
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let p = targets[0][0];
        assert!(
            near(p.x0, 0.216) && near(p.y0, 0.146) && near(p.x1, 0.663) && p.y1 == 1.0,
            "{p:?}"
        );
        let o = targets[1][0];
        assert!(
            near(o.x0, 0.13) && near(o.y0, 0.1102) && near(o.x1, 0.1953) && near(o.y1, 0.3194),
            "{o:?}"
        );
        assert!(targets[2].is_empty());
        assert!(targets[3].is_empty());
        // A box without y0 (zero is left out) still counts.
        let no_y0 = hex("0a1d1001180135c01b3b3c4539dd013f4d0000803f55f56795425d0000803f2a00");
        let t = read_targets(&no_y0).unwrap()[0];
        assert_eq!(t.y0, 0.0);
        assert!(t.contains(0.3, 0.5) && !t.contains(0.6, 0.5));

        // Damage: truncated or nonsense data, a bad CRC and a huge length.
        assert_eq!(read_targets(&hex("0a22100118")), None);
        assert_eq!(read_targets(&hex("0f")), None);
        let mut bad = hex(person);
        bad[20] ^= 1;
        assert!(split_serial(&mut Vec::new(), &bad).is_empty());
        let mut buffer = Vec::new();
        let huge = hex("aa44d7dd0c0091c0020a02aeffff");
        assert!(split_serial(&mut buffer, &huge).is_empty());
        assert!(split_serial(&mut buffer, &vec![0x55; 70_000]).is_empty());
        assert!(buffer.len() <= 4 * (HEADER_LEN + 1024));
    }
}
