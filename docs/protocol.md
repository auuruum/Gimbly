# Vendor protocol reference

One table of every vendor message Gimbly sends or reads, grouped by
feature. The features match `Feature` in
[models.rs](../crates/gimbly/src/camera/models.rs). Evidence and the
observed / inferred / unknown split are in the linked
[clean-room notes](clean-room/README.md). The code that builds and parses
these messages is [vendor.rs](../crates/gimbly/src/camera/vendor.rs).

Everything here was observed on the **Tiny 3 Lite** (`3564:ff04`). When a
model differs, add a line under the feature that names the model and
what it sends instead.

## Transport

The camera's UVC extension unit, GUID
`{9a1e7291-6843-4683-6d92-39bc7906ee49}` (unit 2 on the Tiny 3 Lite).
Every block is 60 bytes.

| Selector | Use |
| --- | --- |
| 2 | Framed messages (below): `SET_CUR` sends one, `GET_CUR` reads the reply |
| 6 | Plain blocks: `SET_CUR` for view and AI settings, `GET_CUR` for the status block |
| 16 | `GET_CUR`: the exposure curve (see [Exposure](#exposure)) |

Framed message on selector 2 (little-endian):

| Bytes | Field |
| --- | --- |
| 0 | `aa` |
| 1 | flags: `01` always, `04` no reply wanted, `08` reply, `20` data, `40` protobuf data |
| 2 | sequence number |
| 3 | channel (Gimbly uses 1) |
| 4-5 | header length, `0c 00` |
| 6-7 | CRC-16 of bytes 0-11 with this field zero |
| 8 | source: `0a` host |
| 9 | destination: `04` gimbal, `02` camera |
| 10-11 | command |
| 12-13 | data length |
| 14-15 | CRC-16 of the length, two zero bytes and the data |
| 16.. | data, then zero padding |

CRC-16: reflected polynomial `0x8005` (`0xa001`), initial value and final
XOR `0xffff`. A read request is a message without flag `04`; the camera
answers with the same sequence, channel and command and flag `08`.
Source: [vendor messages](clean-room/2026-11-08-vendor-messages.md).

In the tables, `44 02 → 04` means command `44 02` to destination `04` on
selector 2. `sel 6: 04 01 v` means a selector 6 block starting with those
bytes. `u32`, `i16` and `f32` are little-endian.

## Status block

`GET_CUR` on selector 6, read about once a second.

| Offset | Meaning | Values |
| --- | --- | --- |
| 0 | block marker | `2e` |
| 1 | Locked Target gesture | 0 off, 1 on |
| 2 | asleep | 0 awake, 1 asleep |
| 3 | Anti-Flicker | 0 off, 1 50 Hz, 2 60 Hz |
| 6 | HDR | 0 off, 1 on |
| 7 | Auto Exposure Mode | 0 Global, 1 Face |
| 10-11 | Sleep Time | i16 seconds, negative while Auto Sleep is off |
| 13 | Auto Focus Mode | 0 Global, 1 Face |
| 14 | Auto Focus | 0 off, 1 on |
| 15 | Manual Focus position | 0 to 100 |
| 24 | AI mode | 0 off, 2 human, 3 hand, 7 voice, 6 switching (under 2 s) |
| 27 | Sleep Background | 0 to 3 |
| 28 | object framing, while AI mode is 2 | `14` Normal, `17` Close-up |
| 29 | Sleep Background Mirror | 0 off, 1 on |
| 31 | frame rate | fps the camera streams at: 30 (`1e`), 60 (`3c`) |
| 42 | Auto Zoom | as in [Auto Zoom](#auto-zoom) |
| 46 | voice bits | bit 1 Forward range, bit 0 Sound Source |

## Gimbal

| Action | Message | Data |
| --- | --- | --- |
| Start / end manual movement | `44 02 → 04` | u8: 0 start, 1 end |
| Speed | `84 64 → 04` | 3 × f32: roll 0, pitch, yaw |
| Turn to angles | `44 64 → 04` | 3 × f32: roll, pitch, yaw in degrees; -1000 keeps that axis |
| Reset | `04 39 → 04` | none |
| Read position | `04 66 → 04` | reply 24 bytes: pitch i16 at 8, yaw i16 at 10, tenths of a degree |

Notes: [joystick](clean-room/2026-11-08-gimbal-joystick.md),
[reset](clean-room/2026-11-08-gimbal-reset.md).

## Zoom

| Action | Message | Data |
| --- | --- | --- |
| Set ratio | `42 19 → 02` | u32 8, u32 ratio × 100 (100-400) |
| Read ratio | `04 68 → 04` | reply f32 ratio |

Note: [view and zoom](clean-room/2026-11-08-view-zoom.md).

## Field of view

| Action | Message | Data |
| --- | --- | --- |
| Wide / Medium / Narrow | `sel 6: 04 01 v` | v: 0 Wide, 1 Medium, 2 Narrow |

No readback; the camera zooms to the view. Note:
[view and zoom](clean-room/2026-11-08-view-zoom.md).

## Presets

| Action | Message | Data |
| --- | --- | --- |
| List | `04 3d → 04` | reply: one byte per stored slot id |
| Read name | `c4 3c → 04` | u32 id; reply ASCII name |
| Read position | `44 3a → 04` | u32 id, `00`; reply 24 bytes: id, yaw i16 at 4, pitch at 6, roll at 8, zoom at 10, hundredths |
| Store position | `44 39 → 04` | u32 id, i16 yaw, pitch, 0, zoom × 100, -200, -200, 300, 300, 0, 0 |
| Name | `84 3a → 04` | u32 id, ASCII name |
| Recall | `c4 39 → 04` | u32 id, 4 × f32 1.0 |
| Delete | `84 39 → 04` | u32 id |

Note: [presets](clean-room/2026-11-08-presets.md).

## Sleep

| Action | Message | Data |
| --- | --- | --- |
| Sleep / wake | `c2 a0 → 02` | u32: 1 sleep, 0 wake |
| Read | status block offset 2 | |

Note: [sleep and wake](clean-room/2026-11-08-sleep-wake.md).

## Device Sleep

Selector 6 blocks; byte 1 is the length of what follows.

| Action | Message | Data |
| --- | --- | --- |
| Auto Sleep and Sleep Time | `sel 6: 0b 02 t` | t: i16 seconds (30, 120, 600); Auto Sleep off sends the negative of the current time |
| Sleep Background | `sel 6: 0e 02 00 n` | n: 0 black, 1 to 3 the pictures stored in the camera |
| Sleep Background Mirror | `sel 6: 0e 02 02 v` | v: 0 off, 1 on |
| Read | status block offsets 10-11, 27, 29 | |

Note: [device sleep and gestures](clean-room/2026-11-28-device-sleep-gestures.md).

## Gesture Control

| Action | Message | Data |
| --- | --- | --- |
| Switch a gesture | `44 34 → 04` | u32 key, u8 0 off / 1 on; key 1 Locked Target, 2 Zoom, 3 Dynamic Zoom, 7 Direction Flip |
| Zoom Factor | `44 34 → 04` | u32 8, f32 ratio 1.0 to 4.0 |
| Read Locked Target | status block offset 1 | |

The camera reports no other gesture; the official app keeps them itself.
Note: [device sleep and gestures](clean-room/2026-11-28-device-sleep-gestures.md).

## Human Tracking

AI blocks on selector 6 are `16 02 sub value`.

| Action | Message | Data |
| --- | --- | --- |
| Start | `sel 6: 16 02 02 z`, then `sel 6: 16 02 0e m` | z: Auto Zoom value; m: 0 Single, 1 Group |
| Stop | `sel 6: 16 02 00 00` | |
| Read | status block offset 24 | 2 while it runs |

The camera reports the same state for Single and Group. Notes:
[human tracking](clean-room/2026-11-20-human-tracking.md),
[group mode](clean-room/2026-11-21-group-mode-auto-zoom.md).

## Auto Zoom

| Action | Message | Data |
| --- | --- | --- |
| Select | `sel 6: 16 02 0f z` | z: 0 Off, 1 Full Body, 2 Upper Body, 3 Close-up, 5 Headless, 6 Lower Body |
| While tracking | also the Start blocks of Human Tracking with the new z | |
| Read | status block offset 42 | |

Note: [group mode and auto zoom](clean-room/2026-11-21-group-mode-auto-zoom.md).

## Tracking Speed

Gimbal settings are read with `84 54 → 04` (u32 0, u32 key) and written
with `44 54 → 04` (u32 0, u32 key, value).

| Action | Message | Data |
| --- | --- | --- |
| Set | write key 4 = u32 0, then key 5 = u32 speed | speed: 0 Slow, 2 Standard, 3 Fast |
| Read | read key 5 | reply u32 speed |

The meaning of key 4 is **unknown**; the official app writes it first.
Note: [tracking speed](clean-room/2026-11-21-tracking-speed.md).

## Sound Source Assisted Tracking

| Action | Message | Data |
| --- | --- | --- |
| On / off | `sel 6: 25 01 v` | v: 1 on, 0 off |
| Read | status block offset 46, bit 0 | |

Note: [sound source tracking](clean-room/2026-11-24-sound-source-tracking.md).

## HDR

| Action | Message | Data |
| --- | --- | --- |
| On / off | `sel 6: 01 01 v` | v: 1 on, 0 off |
| Read | status block offset 6 | |

The official app enables the switch only while offset 31 is 30 or less.

Note: [HDR](clean-room/2026-11-25-hdr.md).

## Focus

Auto Focus and the manual position are the standard UVC camera terminal
controls, not vendor messages. Gimbly writes them through the system's
camera control API (`IAMCameraControl` on Windows, V4L2 on Linux) or as
plain UVC requests on macOS.

| Action | Message | Data |
| --- | --- | --- |
| Auto Focus on / off | UVC `CT_FOCUS_AUTO_CONTROL` | u8: 1 on, 0 off |
| Manual position | UVC `CT_FOCUS_ABSOLUTE_CONTROL` | u16, 0 to 100; also sent right after Auto Focus off |
| Auto Focus Mode | `02 36 → 02` | u32: 0 Global, 1 Face |
| Read | status block offsets 13, 14 and 15 | |

Note: [focus](clean-room/2026-11-26-focus.md).

## Exposure

Shutter, the ISO limits and Auto Exposure are vendor messages to the camera
(`02`). Manual ISO, compensation and Anti-Flicker are the standard UVC
processing unit controls, written through `IAMVideoProcAmp` on Windows, V4L2
on Linux and plain UVC requests on macOS.

| Action | Message | Data |
| --- | --- | --- |
| Auto Exposure on | `82 29 → 02` | u8 `02`, u32 `1` |
| Auto Exposure off | `42 24 → 02` (meaning unknown), read `42 29`, then `82 29` | `42 24`: u32 `1`; `82 29`: u8 `01`, u32 the shutter read back |
| Read exposure | `42 29 → 02`, reply | u8 1 manual or 2 auto, u32 shutter in 100 µs (kept while auto) |
| Shutter | `82 29 → 02` | u8 `01`, u32 shutter in 100 µs |
| ISO limits | `c2 3d → 02` | u32 lower, u32 upper (100, 200 … 12800), 8 more bytes (Gimbly sends u32 100, u32 6400) |
| Read ISO limits | `82 3d → 02`, reply | u32 lower, u32 upper, u32 100, u32 6400 |
| Auto Exposure Mode | selector 6 block `03 01 vv` | 0 Global, 1 Face; status offset 7 |
| Exposure curve | selector 6 block `26 LL 01 NN` + NN points | `LL` = 2 + 2·NN; each point u8 x, u8 y (0–255) between the fixed ends |
| Read curve | selector 16 `GET_CUR` | u8 version (0 or 1), u8 NN, the points |
| Manual ISO | UVC `PU_GAIN_CONTROL` | u16 ISO / 100, 1 to 64 |
| Compensation | UVC `PU_BACKLIGHT_COMPENSATION_CONTROL` | u16 0 to 18, 9 is 0 EV, steps of 1/3 EV; applied only with Auto Exposure on |
| Anti-Flicker | UVC `PU_POWER_LINE_FREQUENCY_CONTROL` | 0 off, 1 50 Hz, 2 60 Hz; applied only with Auto Exposure on; status offset 3 |

Shutter speeds the official app offers at 60 fps and their values: 1/60
167, 1/80 125, 1/100 100, 1/120 83, 1/160 63, 1/200 50, 1/240 42, 1/320
31, 1/400 25, 1/500 20, 1/640 16, 1/800 13, 1/1000 10, 1/1250 8, 1/1600 6,
1/2000 5, 1/2500 4, 1/3200 3, 1/5000 2, 1/6400 1.

Note: [exposure](clean-room/2026-11-27-exposure.md).

## White Balance

One vendor message to the camera (`02`) carries the whole section; the
official app always sends all seven values.

| Action | Message | Data |
| --- | --- | --- |
| Write | `c2 2a → 02` | seven u32: Auto WB (0 on, `ff` off), temperature in kelvin (2000–10000), method (0 temperature, 1 R/B gains), B gain (0–255), R gain (0–255), B-A shift, G-M shift |
| Read | `82 2a → 02`, reply | the same 28 bytes |

The shift runs from -7 to 7 in quarters, sent as 0 to 56 (28 is 0); B-A
goes from blue to amber, G-M from green to magenta. The camera keeps the
manual values while Auto WB is on and the shift while it is off.

Note: [white balance](clean-room/2026-11-28-white-balance.md).

## Image

Contrast, saturation, sharpness and hue are standard UVC processing unit
controls, written through `IAMVideoProcAmp` on Windows, V4L2 on Linux and
plain UVC requests on macOS. Each is a u16 from 0 to 100; 50 is the default
the official app's reset writes.

| Control | UVC selector |
| --- | --- |
| Contrast | `PU_CONTRAST_CONTROL` (3) |
| Hue | `PU_HUE_CONTROL` (6) |
| Saturation | `PU_SATURATION_CONTROL` (7) |
| Sharpness | `PU_SHARPNESS_CONTROL` (8) |

A sleeping camera answers reads with `c0 63`, which is out of range.

Note: [image](clean-room/2026-11-28-image.md).

## Zone Tracking

Settings as in [Tracking Speed](#tracking-speed).

| Action | Message | Data |
| --- | --- | --- |
| Switch | key 3 | u32: 1 on, 0 off |
| Auto-tracking new target | key `12` | u8: 1 on, 0 off; written at once |
| Range limits | keys `13`-`16` | f32 degrees: yaw min, yaw max, pitch min, pitch max |
| Default limit | read with one more byte `01` | |
| Enter / leave editing | `44 08 → 04`, then `44 04 → 04` | `44 08`: 0 enter, 1 leave; `44 04`: `01 01` enter, `01 00` after Done |
| Read return position | `c4 4e → 04` | data `ff` the default, none the stored; reply 24-byte record |
| Write return position | `84 4e → 04` | 24-byte record |
| List Non-tracking Areas | `44 13 → 04` | reply u32 per stored id (0-2) |
| Read area | `44 14 → 04` / `04 15 → 04` | u32 id; reply 24-byte record / ASCII name |
| Write area | `84 13 → 04`, then `44 15 → 04` | 24-byte record; u32 id and ASCII name |
| Delete area | `c4 13 → 04` | u32 id |
| Read initial position | `84 38 → 04` | reply 38 bytes, f32 yaw at 5, pitch at 9 |

Done writes the limits, every area slot (stored or deleted) and the return
position, then leaves editing. Records use the preset layout; bytes 20-21
are written as 0. Note: [zone tracking](clean-room/2026-11-24-zone-tracking.md).

## Object Tracking

| Action | Message | Data |
| --- | --- | --- |
| Track the person at a point | `84 06 → 04` | i16 2, 101, 99, -2; f32 x, y, 0, 0 |
| Track an object box | `84 06 → 04` | i16 3, 2, 0, -2; f32 x0, y0, x1, y1 |
| Cancel | `84 06 → 04` | i16 -1, -1, 0, -2; f32 0 × 4 |
| Framing | `c4 06 → 04` | 8 bytes, byte 0: 0 Normal, 3 Close-up |
| Read framing | status block offset 28 | |

Coordinates are fractions of the picture. The meaning of the i16 values is
**unknown**; they are copied from the capture.

The tracked target's box arrives on the camera's USB serial port (CDC),
in the same framed format: keepalive `42 ae → 02` every 500 ms, start
`42 c4 → 02` once, report settings read `82 ad → 02` and write `c2 ad → 02`
(protobuf), and the stream itself as `02 ae` (protobuf, box corners in
fields 6-9). Note: [preview tracking](clean-room/2026-11-22-preview-tracking.md).

## AI Composition

Gimbal settings as in [Tracking Speed](#tracking-speed).

| Action | Message | Data |
| --- | --- | --- |
| Face Framing | key `0e` | u8: 1 on, 0 off |
| Horizontal offset | key `0f` | f32, -0.16 to 0.16 |
| Vertical offset | key `11` | f32, -0.16 to 0.16 |

Note: [preview tracking](clean-room/2026-11-22-preview-tracking.md).

## Voice Tracking

| Action | Message | Data |
| --- | --- | --- |
| Start | `sel 6: 16 02 07 00` | |
| Stop any AI mode | `sel 6: 16 02 00 00` | |
| Range | `sel 6: 24 01 v` | v: 0 Omnidirectional, 1 Forward |
| Read | status block offsets 24 and 46 (bit 1) | |

Note: [AI mode](clean-room/2026-11-22-ai-mode.md).

## Hand Tracking

Hand settings are read with `84 20 → 04` (u32 key) and written with
`44 20 → 04` (u32 key, value).

| Action | Message | Data |
| --- | --- | --- |
| Start | `sel 6: 16 02 03 00` | |
| Hand | key 4 | u8: 1 left, 0 right |
| Range limits | keys 0-3 | f32 degrees |
| Read return position | `04 25 → 04` | reply 24-byte record |
| Write return position | `c4 24 → 04` | 24-byte record |
| Enter / leave range editing | `sel 6: 18 01 v`, then `c4 26 → 04` | v: 1 enter, 0 leave; `c4 26` data 0 enter, 1 leave |
| Reset with zoom (range panel) | `04 39 → 04` | u8 1 |
| Zoom back to 1.0x on close | `84 68 → 04` | u32 `1e`, f32 1.0 |

Notes: [AI mode](clean-room/2026-11-22-ai-mode.md),
[range panel](clean-room/2026-11-22-hand-range-panel.md).
