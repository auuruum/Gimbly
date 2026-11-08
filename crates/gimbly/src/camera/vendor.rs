//! OBSBOT vendor messages, carried in the camera's UVC extension unit.
//!
//! Learned from USB captures of the official app talking to a Tiny 3 Lite;
//! see `docs/clean-room/2026-11-08-vendor-messages.md`. Every message is
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
//! | 12-13 | data length (only with flag `0x20`) |
//! | 14-15 | CRC of bytes 12-15 (this field zeroed) and the data |
//! | 16.. | data, then padding |

use super::{Fov, GimbalCommand, PRESET_NAME_MAX, PRESET_SLOTS, PresetPosition, ZOOM_RATIO_MAX};

/// GUID of the extension unit, as the UVC descriptor stores it.
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
        let data = if header[1] & FLAG_DATA != 0 {
            let &[lo, hi, crc_lo, crc_hi] = bytes.get(12..HEADER_LEN)? else {
                return None;
            };
            let len = u16::from_le_bytes([lo, hi]) as usize;
            if len > DATA_MAX {
                return None;
            }
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
        TriggerPreset(id) => {
            let mut data = preset_id(id);
            data.extend([1.0f32; 4].into_iter().flat_map(f32::to_le_bytes));
            Some(Message::new(GIMBAL, [0xc4, 0x39], data))
        }
        DeletePreset(id) => Some(Message::new(GIMBAL, [0x84, 0x39], preset_id(id))),
        _ => None,
    }
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

pub fn sleeping(data: &[u8]) -> Option<bool> {
    match data {
        [46, 0, state @ 0..=1, ..] if data.len() >= 46 => Some(*state == 1),
        _ => None,
    }
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

pub fn store_preset(id: u8, current: &[u8], ratio: f32) -> Option<Message> {
    let &[_, _, _, _, _, _, _, _, pitch0, pitch1, yaw0, yaw1, ..] = current else {
        return None;
    };
    if current.len() != 24 {
        return None;
    }
    let yaw = i16::from_le_bytes([yaw0, yaw1]).checked_mul(10)?;
    let pitch = i16::from_le_bytes([pitch0, pitch1]).checked_mul(10)?;
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
}
