# Zone Tracking

## Source and method

2026-11-24, 22:58–23:11. USB capture with USBPcap (snap length 512, all
devices on the camera's root hub `\\.\USBPcap3`, the camera at address 1),
Gimbly closed, OBSBOT Center the only app using the camera. Claude Code
clicked the visible OBSBOT Center UI with one control per step, left and
right clicks on every element of the Zone Tracking switch and its settings
panel, and logged the time of each click. The capture stays outside the
repository and was decoded with our own script.

Auto Sleep was turned off in the More tab from 23:07:33 to 23:11:23
because the camera slept three times during the first clicks; it was set
back to on, 2 min. A short UI-only check without a capture followed at
23:14–23:15 for the two questions below.

| time | click |
| --- | --- |
| 23:02:54 | Zone Tracking switch on |
| 23:03:16 / 23:03:24 | switch off / on |
| 23:03:33 | gear: panel opens |
| 23:03:45 | Cancel |
| 23:03:53 / 23:04:01 | gear / Done, nothing changed |
| 23:04:18 | gear |
| 23:04:26 / 23:04:32 | top limit / left limit (left click) |
| 23:04:4x | left limit right-click, Update, question, No (camera then slept) |
| 23:05:28 | gear (after Resume) |
| 23:05:40 | right limit; right-click, Update, question, OK |
| 23:06:11 / 23:06:18 | Auto-tracking new target on / off |
| 23:06:24 | return line (left click) |
| 23:06:30 / 23:06:34 | range reset icon / return reset icon |
| 23:06:46 | return line right-click, Update |
| 23:06:57 | Done |
| 23:07:53 | gear |
| 23:08:03 | Non-tracking Area tab |
| 23:08:10 / 23:08:19 | Add (Area1) / Area1 menu, Update PTZ |
| 23:08:29 | second Add (Area2) |
| 23:08:39 | joystick drag |
| 23:08:44–23:09:28 | Area2 Update PTZ, recall Area1, recall Area2, right-click Area1, Rename Area2 to Desk, Delete Desk |
| 23:09:34 | Reset (Non-tracking Area tab) |
| 23:09:4x | Instructions for Use (opened a web page) |
| 23:10:00 | Reset (Tracking Range tab) |
| 23:10:14 / 23:10:20 | Auto-tracking on / Done |
| 23:10:26 | gear: values read back |
| 23:10:37–23:10:48 | Non-tracking tab, Tracking Range tab, Auto-tracking off, Done |
| 23:10:59 / 23:11:05 / 23:11:10 | switch off / on / off |

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- Human Tracking Single Mode on at the start, Auto Zoom Off, Zone Tracking
  off, no Non-tracking Areas stored.

## Observed

Gimbal messages go to `0x04` on selector 2 (see
[vendor messages](2026-11-08-vendor-messages.md)). `84 54` / `44 54` are
the numbered settings read and write from
[tracking speed](2026-11-21-tracking-speed.md): `u32 0`, `u32 key`, then
the value.

### UI

- The AI Tracking section has a Zone Tracking row with a help icon, a gear
  and a switch. The gear opened the panel also with the switch off.
- The panel replaces the console: Cancel and Done, "Instructions for Use"
  (opens a web page), and two tabs, Tracking Range and Non-tracking Area.
- Tracking Range is laid out like the Hand Tracking Set panel
  ([range panel](2026-11-22-hand-range-panel.md)): four limits around a
  dashed box (shown 30.0°, −30.0°, 100.0°, −100.0°), a range reset icon, a
  return position line "P | T | Z" with its reset icon, then an
  "Auto-tracking new target" switch with help, the joystick, the zoom
  slider and Reset. A left click on a value turns there; a right click
  shows one entry, Update.
- Non-tracking Area: a note, three slots, joystick, zoom slider and Reset.
  An empty slot shows Add. Add stores the current view as "AreaN" (N is the
  slot number) and shows a caret menu with Update PTZ, Delete and Rename
  (the Rename dialog matches the preset one). Delete asked nothing. After
  deleting Area2 the row showed Area1, Add, Add. A left click on an area
  turns and zooms there; a right click did nothing.
- Updating a limit so the return position falls outside the range asked
  "The set position is beyond the Tracking Range. Would you like to update
  the position?" with No and OK.
  - OK (23:05:59): the limit took the gimbal's pan (0.8°), and the return
    pan became 0.80° with tilt and zoom unchanged.
  - 23:14 check: with the right limit updated to 13.4°, OK gave return P
    13.40°, T 0.00°, Z 1.00 from P 0.58°. No left both the limit (−100°) and
    the return position unchanged.
- Updating the return position outside the range asked whether to update
  the Tracking Range; OK widened only the crossed limit to the new position
  (6.4°), as in the Hand Tracking panel.
- Every Done rewrote the return pan one hundredth lower than shown before
  (0.60° → 0.59° → 0.58°).
- The Tracking Range Reset stopped Human Tracking: status offset 24 went
  `02` → `00`, and Single Mode was no longer highlighted.

### USB

| action | messages |
| --- | --- |
| switch on / off | `44 54` key 3, `u32` 1 / 0 |
| switch read | `84 54` key 3 every ~3 s; reply `u32` |
| open | `44 08` data `00`; `c4 4e` data `ff`, then `c4 4e` without data; `44 04` data `01 01`; `84 54` keys `13`–`16` (4 × `f32`) and `12` (`u8`); `44 13` (no data); `84 38` (no data); per stored area `04 15` `u32 id` and `44 14` `u32 id` |
| Done | `44 54` keys `13`–`16`, `f32` each; for each of the three slots either `84 13` (24-byte record) and `44 15` (`u32 id`, ASCII name) or `c4 13` `u32 id`; `84 4e` (24-byte return record); `44 08` data `01`; `4d 70` to `0x12` data `3d`; `84 68` `u32 1e`, `f32`; `44 04` data `01 00` |
| Cancel | `44 08` data `01`; `4d 70` to `0x12`; `84 68` `u32 1e`, `f32` (no `44 04`) |
| limit click | `44 64` as in the Hand Tracking panel |
| limit or return Update | `04 66` (and `04 68` for the return); no write |
| return click | standard UVC zoom (selector 11 on interface 0), then `44 64` |
| Auto-tracking switch | `44 54` key `12`, `u8` 1 / 0, at once |
| range reset icon | `84 54` keys `13`–`16` with one more byte `01`; replies −100, 100, −30, 30 |
| return reset icon, Delete, Rename | nothing |
| Add, Update PTZ | `04 66` only |
| area click | standard UVC zoom, then `44 64` (roll −1000) |
| Reset, Tracking Range tab | `04 39` data `01` |
| Reset, Non-tracking Area tab | UVC zoom value 10 (1.30x), `44 64` roll 0, pitch 3.30, yaw 6.39 |

Replies:

- `c4 4e` with data `ff`: id 0, P 0, T 0, Z 1.00 (`…64 00…`), also after a
  position was stored. Without data: the stored record; after the first
  Done its pan was `3c 00` (0.60°).
- Limits: `f32` −100, 100, −30, 30 for keys `13`–`16`.
- `44 13`: before any area was stored the reply had flag `09` (no data
  flag). With Area1 stored: flag `29` and four zero bytes.
- `44 14` `u32 0`: id 0, yaw 340, pitch 360 (hundredths), roll 0, zoom 193,
  eight zero bytes, `64 00`, `00 00`. `04 15` `u32 0`: `Area1`.
- `84 38`: 38 bytes, `f32` 6.39 at byte 5 and 3.30 at byte 9, then `f32`
  0, 1.0, 0.5, 0.5, 1.0, 1.0 and one zero byte.
- The area record Done wrote for Area1 was the one read back, with bytes
  20–21 as `00 00` (read `64 00`), as for the Hand Tracking return record.
- The `f32` in `84 68` was 2.1 in the first Dones and 4.41 later, while the
  zoom read 1.0–1.3; it did not change the zoom.

## Inferred

- Key 3 is the Zone Tracking switch; keys `13`–`16` the range in degrees
  in the camera's sign (yaw min, yaw max, pitch min, pitch max), shown like
  the Hand Tracking range; key `12` Auto-tracking new target.
- The extra byte `01` on a settings read asks for the default value.
- `c4 4e` / `84 4e` read and write the zone return position in the preset
  record layout; data `ff` reads the default.
- `44 08` (`00` enter, `01` leave) and `44 04` (`01 01` / `01 00`) put the
  camera in and out of a zone-edit state.
- `44 13` lists the stored area ids as one `u32` each, confirmed with two
  areas in the Gimbly check below. `44 14` / `04 15` read an area's record
  and name, `84 13` / `44 15` write them and `c4 13` deletes a slot. Areas
  are only written by Done.
- A limit Update that leaves the return position outside moves only the
  crossed axis of the return position to the limit; No discards the Update.
- `84 38` holds the camera's initial position (yaw, pitch); the Non-tracking
  Area Reset turns there.

## Unknown

- What `4d 70` to `0x12` and the `f32` in `84 68` key `1e` mean; Gimbly
  does not send them.
- Why Cancel sends no `44 04 01 00`. Gimbly does the same.
- Where the 1.30x zoom of the Non-tracking Area Reset comes from; Gimbly
  only turns to the initial position.
- The rest of the `84 38` reply.
- What the switch, the range and the areas change in tracking: no tracking
  behavior with a person was tested.
- Whether the gear is disabled during Hand Tracking (the switch is, see
  [AI mode](2026-11-22-ai-mode.md)).

## Tests

- `camera::vendor::tests::zone_tracking_matches_capture`: switch,
  Auto-tracking and limit reads, edit-state messages, the area list replies,
  an area record from a position, the Done sequence with one area and with
  none, and the initial position.
- `camera::tests::zone_limit_moves_the_return_position_inside`: the 13.4°
  OK case above.

## Checked in Gimbly

2026-11-24, same camera and OS, OBSBOT Center closed:

- Probe: the panel read the values OBSBOT Center left (P 0.58°, Area1,
  initial position 6.39°, 3.30°). Gimbly added Area2 and saved; reopening
  listed ids 0 and 1, and OBSBOT Center then showed Area1 and Area2.
- Just after Gimbly connected, the first `c4 4e` reads went unanswered
  twice in five runs; with one retry the panel opened in three of three
  runs straight after start.
- Running app with the preview and virtual camera on: switch on, limit
  click, the 10° error, joystick drag (preview frames kept changing),
  limit Update with the move question and OK, Auto-tracking on and off,
  area recall, Rename, Delete, Add, the Non-tracking Area Reset, Cancel
  (reopening showed the old values), both reset icons and Done.
- The camera was left with no areas, the default range and return
  position, Auto-tracking off and Zone Tracking off; a probe read that
  back. Human Tracking, on before the capture, stayed off after the
  Tracking Range Reset stopped it.

Still to check: macOS, native Wayland and native X11, and tracking with a
person in the range.
