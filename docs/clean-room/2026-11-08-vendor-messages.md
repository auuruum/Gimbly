# Vendor message channel (UVC extension unit)

## Source and method

- 2026-11-08: USB capture with USBPcap (`\\.\USBPcap3`, the root hub the
  camera is on, descriptors injected) while the owner used OBSBOT Center:
  woke the camera, moved the panel joystick in several directions and at
  several Gimbal Speed settings, opened the preset list. About 310 s.
- Captures were analysed with our own scripts (outside the repository);
  the capture files are not committed.
- Black-box check: Gimbly built messages with the layout below and sent them
  (`gimbly --probe --controls move turn:0:10:1000 stop ...`); the camera
  answered and moved.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`. Firmware version: not read
  yet (**unknown**).
- Official app: OBSBOT Center, version not recorded (**unknown**).
- OS: Windows 10 Pro 19045.

## Observed

- The camera is a composite device: interface 0 video control (camera),
  interface 2 audio, interface 4 a CDC serial port (COM3). The official app
  sent nothing over the serial port during the capture; all control traffic
  went to the video control interface.
- The video control interface has one extension unit: unit ID 2, GUID
  `{9a1e7291-6843-4683-6d92-39bc7906ee49}`, 19 controls, bmControls
  `ff ff 3f 00`.
- The app sends commands as UVC class `SET_CUR` requests to that unit,
  selector 2, always 60 bytes. For commands that expect an answer it reads
  selector 2 back with `GET_CUR` (60 bytes). It asks `GET_LEN` first each
  time.
- Selector 6 is read (`GET_CUR`, 60 bytes) roughly once a second; one byte
  of it changes between two values over the capture (meaning **unknown**,
  possibly the sleep state).
- Selector 4 carries file-like transfers (names of image and video files
  inside the camera). Not needed; not used.
- Message layout (little-endian):

  | bytes | content |
  | --- | --- |
  | 0 | `aa` |
  | 1 | flags: `01`, `21`, `25` from the app; `29` in replies |
  | 2 | sequence number, counting up per channel |
  | 3 | channel (the app used 1, 2, 12, 13 at the same time) |
  | 4-5 | `0c 00` |
  | 6-7 | checksum of bytes 0-11 |
  | 8, 9 | source, destination (`0a` the app; `04` and `02` in the camera); replies swap them |
  | 10-11 | command |
  | 12-13 | data length |
  | 14-15 | checksum of the data, `00 00` when there is no data |
  | 16- | data; the rest of the 60 bytes is left over from earlier messages |

- Both checksums are CRC-16 with the reflected polynomial 0x8005, initial
  value 0xffff and final XOR 0xffff. The header checksum covers bytes 0-11
  with bytes 6-7 taken as zero; the data checksum covers bytes 12-15 (bytes
  14-15 as zero) plus the data. Found by fitting the CRC parameters to the
  captured messages; all 1 156 intact messages of the capture match.
- The camera did not care about the sequence number in the black-box test:
  messages with sequence 0 every time were carried out.

## Inferred

- Flag `0x25` marks commands sent without waiting for a reply (no `GET_CUR`
  followed them), `0x08` marks replies.
- Byte 4-5 is the length of the part covered by the header checksum.

## Unknown

- The meaning of most command IDs seen while idle (`04 01`, `84 54`,
  `04 3d`, `02 1d`, ...), probably status polling.
- Firmware differences between camera models.

## Test

`camera::vendor::tests` encodes two captured messages byte for byte, decodes
a captured reply, and checks that damaged messages are rejected.
