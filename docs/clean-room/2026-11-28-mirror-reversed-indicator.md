# Mirror Image, View and Gimbal Reversed, Indicator Status (More tab)

## Source and method

2026-11-28, 19:40-19:45. Claude Code went through the three rows and the
Indicator Status section of the More tab in the visible OBSBOT Center UI,
one action at a time, with a USB capture running (USBPcap, snap length
512, camera at address 1 on `\\.\USBPcap3`, Gimbly closed). Every left
click, right click, `?` icon, the section header and the number field were
tried; OBSBOT Center was closed and started again to see what it reads
back. The capture stays outside the repository and was decoded with our
own script.

19:51-19:56 Gimbly was checked with `gimbly --probe` and in the running
app (see below), OBSBOT Center closed.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (shown
  on the More tab).
- Official app: OBSBOT Center `v2.0.17.532` (not reread).
- OS: Windows 10 Pro 19045.

## Observed: UI

- **Mirror Image**: a switch with `?`, below Disable Microphone, no section
  header. The `?` opens a Tips box: the picture is mirrored left to right
  on the camera side. OK closes it.
- **View and Gimbal Reversed**: a switch with `?`. Tips: it controls the
  lens's left-right rotation; on, the lens turns left-right the other way.
  No dialog when switched, nothing else on the More or Console tab changed.
- **Indicator Status**: a foldable section, open by default, no `?`.
  Clicking anywhere on the header folds or unfolds it.
  - Status Light switch, on. Turning it off opens a Tips box: with the
    light off it may be hard to tell the camera's status, turn it off
    anyway? Cancel and Done. The switch is drawn off behind the box;
    Cancel puts it back on and sends nothing.
  - Brightness: slider and number field with arrows, 1 to 3, value 3.
    While the light is off the row is greyed and a drag does nothing.
    Arrows step by 1 and stop at 1 and 3. Typed 1 and 3 taken, 5 and 0
    refused (the old value stays).
- No hover text and no right-click menu on any of these controls.
- After OBSBOT Center was closed and started again with Mirror Image on,
  View and Gimbal Reversed on and Brightness 2, all three showed so.
- The camera fell asleep twice during the session (19:42:43 and 19:44:54),
  each time right after a click; status offset 9 went to 3 before the app
  sent anything. The camera's Auto Sleep time was 2 minutes (status
  offsets 10-11) and nothing streamed video, so this is most likely Auto
  Sleep, not these controls (inferred).

## Observed: USB

| time | action | message |
| --- | --- | --- |
| 19:41:28 | Mirror Image on | `sel 6: 14 01 01` |
| 19:41:31 | Mirror Image off | `sel 6: 14 01 00` |
| 19:41:37 | View and Gimbal Reversed on | `44 3f → 04`, `04 00 00 00 01` |
| 19:41:57 | View and Gimbal Reversed off | `44 3f → 04`, `04 00 00 00 00` |
| 19:42:09 | Status Light off, Cancel | nothing |
| 19:42:14 | Status Light off, Done | `sel 6: 1a 01 00` |
| 19:42:21 | drag on the greyed slider | nothing |
| 19:42:24 | Status Light on | `sel 6: 1a 01 03` (the brightness) |
| 19:42:32 | Brightness dragged to 1 | `1a 01 03`, `1a 01 01`, `1a 01 01` while dragging |
| 19:42:36-39 | up arrow twice | `1a 01 02`, `1a 01 03` |
| 19:43:01-17 | down arrow; typed 1, 5, 0, 3 | `1a 01 02`, `01`, `01`, `01`, `03` (a refused value resends the old one) |
| 19:43:41-47 | Mirror on, Reversed on, Brightness 2 | as above |
| 19:44:28 | OBSBOT Center started | no read of key 4 (see below) |
| 19:44:49-52 | Mirror off, Reversed off | as above |
| 19:45:12 | Brightness 3 | `1a 01 03` |

There is no separate brightness message: `1a 01 v` with v 1 to 3 turns
the light on at that brightness, 0 turns it off.

Status block (selector 6):

- 19: Mirror Image, 0 off, 1 on.
- 45: Status Light, 1 on, 0 off.
- 33: Brightness 1 to 3. It stayed 3 while the light was off.

View and Gimbal Reversed is not in the status block. At start-up OBSBOT
Center read `84 3f` keys 5 and 6 (replies f32 1.0 and 0) but not key 4.

## Observed: black-box experiments with Gimbly's probe

19:47, OBSBOT Center closed, a temporary probe step that was not kept:

- `84 3f → 04` with u32 4 replied one byte: `00`, after writing on `01`,
  after writing off `00`. So the setting can be read back.
- Joystick test (same yaw speed for 1 s, reversed off then on): with it
  off the camera's pan went from 6 to -33, with it on from -34 to 0. The
  camera turns the other way.
- Snapshots from the reset position with reversed off, on, off were the
  same picture: it does not mirror or turn the video.
- Mirror on gave a left-right mirrored snapshot.

## Inferred

- "View" in the name refers to the turning direction as seen in the
  picture, not to the picture itself (the picture did not change).
- OBSBOT Center keeps View and Gimbal Reversed in its own settings or
  reads it another way we did not see; Gimbly reads it from the camera.

## Unknown

- Whether the status light colours or blink patterns change with
  brightness beyond what the eye sees (not measured).
- What `84 3f` keys 5 and 6 are.
- Whether View and Gimbal Reversed also reverses tracking and gestures
  (the help text in Gimbly says so in general terms; only the joystick was
  tried).

## Gimbly

- More tab, after Audio: Mirror Image and View and Gimbal Reversed rows
  with `?` and Tips in our own words, then the Indicator Status section
  with Status Light and Brightness (slider, field, arrows, 1 to 3, typed
  values outside refused, greyed while the light is off). Turning the
  light off asks first (No / OK); No sends nothing. Turning it on sends
  the brightness the camera kept.
- Mirror Image and the status light are read from the status block every
  second; View and Gimbal Reversed is read once at start (and retried with
  the hand setting) from `84 3f` key 4.
- `gimbly --probe --gimbal DIR mirror:on reversed:on light:0 light:2`.

## Tests

- `camera::vendor::tests::mirror_reversed_and_status_light_match_capture`
  uses the captured writes and the first status block of the capture.
- `app::view_more::tests::brightness_steps_like_the_official_app`.

## Checked in Gimbly

2026-11-28 19:49-19:56, same camera and OS, OBSBOT Center closed:

- `gimbly --probe --virtual-camera --gimbal DIR wake mirror:on reversed:on
  light:0 light:1 light:3 0:40:800 mirror:off reversed:off 0:40:800`: no
  errors, the status after each step showed the new value; the mirrored
  snapshot was mirrored.
- Running app with the preview and the virtual camera on: Mirror Image on
  mirrored the preview, off restored it; both Tips boxes; View and Gimbal
  Reversed on, joystick dragged right turned the camera left with frames
  flowing, off turned it right; Status Light off asked, No kept it on, OK
  turned it off and greyed Brightness (drag ignored), on again at 3;
  Brightness dragged to 1, arrows up to 3 and down to 2, typed 5 and 0
  refused, typed 3 taken; the section folds and unfolds.
- Linux and macOS: compiled by CI only, not run.
- 20:04, after the owner saw "Camera control queue is full" while
  dragging Brightness: the slider sent a write every frame and the
  status light write was not coalesced. It now is, like the other
  sliders (`camera::queue::tests::brightness_slider_never_fills_the_queue`);
  a long back-and-forth drag with the preview and virtual camera on gave
  no error.

The camera was left with Mirror Image off, View and Gimbal Reversed off,
Status Light on at 3.
