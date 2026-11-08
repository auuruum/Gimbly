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
//! | 1 | flags ([`FLAGS_NO_REPLY`], ...) |
//! | 2 | sequence number, counted per channel |
//! | 3 | channel |
//! | 4-5 | header length, always 12 |
//! | 6-7 | CRC of bytes 0-11 with this field zeroed |
//! | 8 | source |
//! | 9 | destination |
//! | 10-11 | command |
//! | 12-13 | data length |
//! | 14-15 | CRC of bytes 12-15 (this field zeroed) and the data; 0 without data |
//! | 16.. | data, then zero padding |

/// GUID of the extension unit, as the UVC descriptor stores it.
pub const XU_GUID: u128 = 0x9a1e7291_6843_4683_6d92_39bc7906ee49;
/// Extension unit control that takes messages and returns replies.
pub const SELECTOR_COMMAND: u32 = 2;
/// Every message is padded to this size.
pub const MESSAGE_LEN: usize = 60;
const HEADER_LEN: usize = 16;
/// Largest data part that fits in one message.
pub const DATA_MAX: usize = MESSAGE_LEN - HEADER_LEN;

/// Flags the official app uses for commands it does not wait an answer for.
pub const FLAGS_NO_REPLY: u8 = 0x25;
/// The address the official app sends from.
pub const HOST: u8 = 0x0a;
/// The gimbal's address.
pub const GIMBAL: u8 = 0x04;

/// Begins (data `0`) or ends (data `1`) manual gimbal movement.
pub const CMD_MANUAL_MOVE: [u8; 2] = [0x44, 0x02];
/// Gimbal speed: three `f32`, roll (always 0), pitch and yaw.
pub const CMD_SPEED: [u8; 2] = [0x84, 0x64];

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
            flags: FLAGS_NO_REPLY,
            seq: 0,
            channel: 0,
            source: HOST,
            destination,
            command,
            data,
        }
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
    #[allow(dead_code)] // read once replies are used (presets, zoom ratio)
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let header = bytes.get(..HEADER_LEN)?;
        if header[0] != 0xaa || header[4..6] != [12, 0] {
            return None;
        }
        let mut zeroed = [0u8; 12];
        zeroed.copy_from_slice(&header[..12]);
        zeroed[6..8].fill(0);
        if crc16(&zeroed) != u16::from_le_bytes([header[6], header[7]]) {
            return None;
        }
        let len = u16::from_le_bytes([header[12], header[13]]) as usize;
        let data = bytes.get(HEADER_LEN..HEADER_LEN.checked_add(len)?)?;
        if data_crc(data) != u16::from_le_bytes([header[14], header[15]]) {
            return None;
        }
        Some(Self {
            flags: header[1],
            seq: header[2],
            channel: header[3],
            source: header[8],
            destination: header[9],
            command: [header[10], header[11]],
            data: data.to_vec(),
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

    /// Stamps `message` with the next sequence number and encodes it.
    pub fn number(&mut self, mut message: Message) -> [u8; MESSAGE_LEN] {
        (message.seq, message.channel) = (self.seq, Self::CHANNEL);
        self.seq = self.seq.wrapping_add(1);
        message.encode()
    }
}

/// The message for a gimbal command, or `None` when that command is not
/// supported yet.
pub fn for_gimbal(command: &super::GimbalCommand) -> Option<Message> {
    use super::GimbalCommand::*;
    match *command {
        Drag(active) => Some(manual_move(active)),
        Speed { pitch, yaw } => Some(speed(pitch, yaw)),
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
}
