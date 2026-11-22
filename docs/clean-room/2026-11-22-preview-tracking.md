# Preview tracking: Object Tracking, double-click target, AI Composition lines

## Source and method

2026-11-22. Two USB captures with USBPcap (snap length 512, all devices on
the camera's root hub; the camera was on `\\.\USBPcap2` this time, address
1). Gimbly was closed; OBSBOT Center was the only app using the camera.
Claude Code clicked and dragged in the visible OBSBOT Center UI, one action
at a time with 3–7 s between actions, and took a screenshot after each. The
owner sat in front of the camera with the light on. The captures stay
outside the repository and were decoded with our own scripts.

Actions in capture 1 (15:04–15:06): AI Composition lines open, vertical line
to the left third, horizontal line to the upper third, Face Framing on and
off, Reset, Done; open again, vertical line to the right third, Done; open,
Reset, Done; double-click on an empty area, double-click on the person;
Object Tracking button twice, banner close (×); Object Tracking button, box
around a teddy bear, ✕; box again, ✓; Close-up, Normal, Normal.

Actions in capture 2 (15:08–15:11): AI Composition lines with tracking off;
double-clicks on the face, an empty area, the body, an empty area and the
head; AI Composition lines: lines dragged past the frame edge in all four
directions, then to positions between the centre and the limits, Reset,
Done; Object Tracking: box around a sign, ✓; Close-up, Normal, Close-up,
Close-up again.

A third, shorter test opened the camera's CDC serial port (COM3) with the
standard Windows serial API while OBSBOT Center was closed, read for 2–3 s
without writing, then wrote the 12-byte message described below every
500 ms.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- Auto Zoom Off, Zone Tracking off, Sound Source Assisted Tracking on,
  Tracking Speed Standard.

## Observed: UI

- The preview bar has two buttons between Mirror and ME. Their tooltips are
  **Object Tracking** (an "AI" label in corner brackets) and **AI
  Composition lines** (a square split in four).
- **Object Tracking** turns red and dims the picture. A banner at the top
  reads "Select object for tracking" with a close (×). Dragging draws a
  green box; when the button is released a ✕ and a ✓ appear under the
  box. ✕ removes the box and the dimmed state stays. ✓ starts tracking
  and leaves the select state. A single click selects nothing. The banner
  × leaves the select state. Clicking the button a second time did not
  leave the select state in one try.
- While a target is tracked the preview draws a thin white box with green
  corner marks around it; the box follows the target.
- In the console, the Object Tracking buttons **Normal** and **Close-up**
  are grey unless object tracking runs. Selecting an object highlights
  Normal and greys Zone Tracking. Close-up zooms in on the object, Normal
  zooms back out, and clicking the highlighted button stops tracking.
- **Double-click** on a person starts tracking that person and highlights
  Single Mode. A double-click on the empty area while tracking stops it.
  A double-click on a clock with tracking off did nothing visible.
- **AI Composition lines** with tracking off shows a toast "Enable Human
  Tracking first" and nothing else. With tracking on it hides the bar
  buttons and shows, along the bottom, **Face Framing** on the left and
  **Reset** and **Done** on the right. The picture gets a thin white
  rule-of-thirds grid and two yellow lines through the centre, each with a
  short thick handle at its ends. A banner reads "Move lines to adjust
  composition position". Dragging a line moves it; the camera reframes
  the person live. Lines stop at the thirds; they cannot be moved past
  them.
- **Face Framing** turns red, hides the vertical line and shows "Auto
  horizontal framing based on face orientation." Clicking it again brings
  the vertical line back.
- No tutorial picture appeared when the preview opened or from the "?"
  next to the title (that shows a text about using the camera in other
  apps). The owner's screenshot of the two-mouse hint ("Drag the mouse to
  control the direction of the gimbal", "Double-click to select the
  target") was taken earlier; we infer it is a first-run hint.

## Observed: USB

All writes are vendor messages on extension unit selector 2 (see
[vendor messages](2026-11-08-vendor-messages.md)) from the host (`0a`) to
the gimbal (`04`) with flags `25` (data, no reply), unless noted.

### Target selection: `84 06`, 24 bytes

| Action | Data |
| --- | --- |
| Double-click on the person | `i16` 2, 101, 99, −2; `f32` x, y; 8 zero bytes |
| Box ✓ | `i16` 3, 2, 0, −2; `f32` x0, y0, x1, y1 |
| Double-click on an empty area while tracking | `ff ff ff ff 00 00 fe ff`, then the `f32` x, y of the previous double-click, 8 zero bytes |
| Highlighted Normal or Close-up clicked | `ff ff ff ff 00 00 00 00`, then 16 bytes that changed between captures |

x and y are fractions of the picture width and height from its top left
corner. The five double-clicks matched the click positions within 0.005
(for example a click at 78.1 % / 73.1 % sent 0.780 / 0.730). The box sent
0.1295, 0.1103, 0.1953, 0.3194 for a box drawn from 12.9 % / 11.0 % to
19.5 % / 32.0 %, so the last two values are the far corner, not a size.

Offset 24 of the selector 6 status block went `00` → `02` within 1 s of a
person double-click, and `02` → `00` after a cancel. After a box ✓ offset
28 went `00` → `14`.

### Object framing: `c4 06`, 8 bytes

- Close-up: `03 00 00 00 00 00 00 00`; offset 28 went `14` → `17`.
- Normal: eight zero bytes; offset 28 went `17` → `14`.
- While Close-up ran, offsets 4 and 17 changed (`06`/`2a`/`2c` and
  `01`/`03`), and the app read setting `0x17` every 2 s (`84 54` with
  `u32 0, u32 0x17`, reply `u32 10`).
- After Normal the app also wrote 2 bytes `00 00` to selector 11 of the
  same unit.

### AI Composition lines: settings `0x0e`, `0x0f`, `0x11`

These use the setting messages already known from
[Tracking Speed](2026-11-21-tracking-speed.md): read `84 54` with
`u32 0, u32 key` (reply requested), write `44 54` with `u32 0, u32 key,
value`.

- Opening the mode reads keys `0e`, `0f`, `11`. Replies: `0e` one byte,
  `0f` and `11` an `f32` each.
- Releasing either line writes **both** `0f` (vertical line) and `11`
  (horizontal line) as `f32`. Nothing is written while dragging.
- Values: centre `0`; left third `−0.16`; right third `0.16`; upper third
  `−0.16`; lower third `0.16`; between, for example, `−0.0682` and
  `0.0648`. The lines stopped at `±0.16` when dragged past the frame
  edge.
- Face Framing on writes key `0e` with one byte `01`; off writes `00`.
  Before each the app read `0f` and `11` again.
- Reset writes `0f = 0` and `11 = 0`. Done writes nothing.

### Target box stream on the CDC serial port

- The camera's CDC serial function (interface 4) sends vendor messages on
  its bulk IN endpoint all the time, about 30 per second, split over USB
  packets: flags `44`, source `02`, destination `0a`, command `02 ae`.
- OBSBOT Center writes `aa 05 00 00 0c 00 02 b9 0a 02 42 ae` (command
  `42 ae` to `02`, no data, header only) every 500 ms on the bulk OUT
  endpoint. The camera never answered it with a separate message.
- Opening COM3 with OBSBOT Center closed, the same stream arrived **without
  any write** (about 800 bytes/s), also after the port had been unused for
  15 s.
- The data is in protobuf wire format. With nobody tracked it is `2a 00`.
  While tracking it is field 1 (a target) followed by `2a 00`:

  | Field | Wire type | Person | Object |
  | --- | --- | --- | --- |
  | 2 | varint | 1 | 1 |
  | 3 | varint | 1 | 1 |
  | 4 | varint | – | 1 |
  | 5 | varint | – | 2 |
  | 6 | `f32` | x0 | x0 |
  | 7 | `f32` | y0 (left out when 0) | y0 |
  | 8 | `f32` | x1 | x1 |
  | 9 | `f32` | y1 | y1 |
  | 10 | `f32` | 60–300 | 85.8 |
  | 11 | `f32` | 1.0 | – |

  Right after ✓ the object target carried exactly the box the app sent
  (0.13, 0.11, 0.1953, 0.3194). The person box matched the white box drawn
  in the preview (for example 0.256–0.68 of the width).
- After a cancel the data was `0a 03 08 ff 01 2a 00` (a target whose field
  1 is 255) and then `2a 00`.
- The selector 2 extension unit did not answer `42 ae`, `82 ae` or
  `02 ae` sent to `02` (Gimbly black-box test, tracking on): every read
  returned zeros.

### Starting the stream (black-box tests with Gimbly, same day)

The stream above was running because OBSBOT Center had started it. Later
tests with only Gimbly found:

- **Observed:** with the camera asleep the port sent nothing.
- **Observed:** awake, after OBSBOT Center had been closed for a few
  minutes, the port sent nothing, with or without `42 ae` every 500 ms and
  with or without `SetCommState`. Reads returned 0 bytes.
- **Observed:** a capture of OBSBOT Center connecting showed, on selector 2
  to `02`: `02 c8`, then `42 c4` with no data (reply `u32 0`), `02 ad`,
  `82 ad` with `0a 01 01` (reply `0a 04 08 01 12 00`), and `c2 ad` with
  `08 01 12 02 20 01`. The first serial message arrived 2 ms after that
  write. OBSBOT Center opened the port with 9600 baud, 8 bits, DTR off.
- **Observed:** Gimbly sending `42 c4`, then reading `82 ad` and writing
  the reply's record back unchanged (`08 01 12 00`), did not start it.
- **Observed:** writing `08 01 12 02 20 01` (field 2.4 = 1) started the
  stream 0.4 s later. It carried `2a 00` only, also while an object was
  tracked.
- **Observed:** writing `08 01 12 04 08 01 20 01` (also field 2.1 = 1, the
  record the official app writes after tracking turns on) made the stream
  carry the target box; the preview box followed the object, Close-up and
  the person.
- **Observed:** the stream stopped within about 10 s once no process sent
  `42 ae` (between two Gimbly runs); a process that waited for data before
  sending it never got any.
- **Observed:** Windows did not send SET_LINE_CODING or
  SET_CONTROL_LINE_STATE when Gimbly opened the port without
  `SetCommState`, and the camera stayed silent; with `SetCommState`
  (9600 baud) it sent both.

## Inferred

- `84 06` selects the tracking target. The first `i16` is the kind: 2 a
  point (the camera picks the person there and starts Human Tracking in
  Single Mode), 3 a box (object tracking), −1 (`ff ff`) no target (stop).
- The app decides a double-click is a cancel when it falls outside the
  box of the current target; otherwise it sends the point.
- Offset 28 of the status block shows the object framing while object
  tracking runs: `0x14` Normal, `0x17` Close-up. Offset 24 stays `2`, the
  Human Tracking value.
- `c4 06` sets the object framing: `3` Close-up, `0` Normal.
- Settings `0x0f` and `0x11` are the horizontal and vertical offset of the
  subject from the picture centre, in fractions of the picture, limited to
  ±0.16. `0x0e` is Face Framing.
- Record `82 ad`/`c2 ad` field 2.4 turns the serial report on and field
  2.1 adds the tracked target to it. `42 ae` keeps the report going.

## Unknown

- The `i16` values 101, 99 and −2 in the point selection, and the 16
  trailing bytes of the button stop.
- What setting `0x17` (`10`) and selector 11 are for.
- Whether `42 c4` and the line setup are needed; Gimbly sends both as
  the official app does.
- Whether field 2.1 = 1 without tracking has any effect.
- Fields 2–5, 10 and 11 of a target.
- Whether Face Framing is kept by Reset.
- What the first-run hint looks like on another machine; it was not shown
  again in these sessions.

## Test

- `camera::vendor::tests::target_selection_matches_capture`: point, box and
  cancel messages; object framing; status offset 28.
- `camera::vendor::tests::composition_matches_capture`: setting writes and
  reads for keys `0e`, `0f`, `11` and the ±0.16 limit.
- `camera::vendor::tests::target_stream_matches_capture`: the keep-alive
  bytes, the report record read and writes, splitting the serial byte
  stream into messages, and decoding the person, object, cancel and empty
  targets; damaged and oversized input is rejected.
- `preview_tracking::tests` check the double-click decision and the
  composition lines under rotation.
- `preview_view::tests` map preview points to camera coordinates and back
  for every rotation, mirror and the portrait crop.

## Hardware results (Gimbly, 2026-11-22)

Windows 10 Pro 19045, Tiny 3 Lite firmware 6.5.11.1, Gimbly debug build
with the preview open. Each step was checked in the preview, in the panel
and in a USB capture.

- Object Tracking button: dimmed picture, banner, box with ✕/✓; ✓ sent
  the box, the camera turned to the object and the preview drew its box.
- Close-up zoomed in on the object; Normal zoomed out; clicking the
  highlighted Normal stopped tracking. The buttons were grey otherwise.
- Double-click outside the person's box stopped tracking; double-click on
  the person started Single Mode with the box.
- AI Composition lines: lines stopped at the thirds, the camera reframed
  after each release, Face Framing hid the vertical line, Reset centred
  both, Done closed the mode. With tracking off the toast "Enable Human
  Tracking first" appeared.
- A gimbal drag on the preview kept the video moving.
- First-run hint showed once and went away on click.

Not checked: macOS and Linux (native Wayland and X11) builds were only
compile-checked with Clippy; the Linux and macOS serial port code and the
macOS port naming are untested. The virtual camera was not running during
these steps.
