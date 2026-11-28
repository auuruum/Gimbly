# Device Sleep and Gesture Control (More tab)

## Source and method

2026-11-28, 13:00-13:11. Six USB captures with USBPcap (snap length 512,
device address 1 on the camera's root hub `\\.\USBPcap2`), Gimbly closed,
OBSBOT Center the only app using the camera. Claude Code went through every
control of the **Device Sleep** and **Gesture Control** sections of the
**More** tab in the visible OBSBOT Center UI, one action at a time with a few
seconds between them, and checked the UI after each one. The captures stay
outside the repository and were decoded with our own script.

| time | action |
| --- | --- |
| 13:00:51-13:01:04 | Sleep Time 30s, 10min, 2min |
| 13:02:12-17 | Auto Sleep off, on |
| 13:02:22-38 | Sleep Background tiles 1, 2, 3, 4 |
| 13:02:43-49 | Sleep Background Mirror on, off |
| 13:02:5x | `?` after Sleep Background; right-click and wheel on the tiles |
| 13:03:4x | click on the Gesture Control header (folds), unfold, `?` after the title |
| 13:04:14 | Locked Target on; a notice opened and the camera fell asleep (Auto Sleep, 2 min) |
| 13:05:5x | notice closed, Resume, Sleep Time 10min |
| 13:06:08-17 | Locked Target off, Zoom on (notice), Got it |
| 13:06:29-13:07:24 | Zoom Factor dragged to both ends, arrows, typed `2.5`, `5`, `0.5`, `1.234`, `2` |
| 13:07:24-13:08:04 | Zoom off; Dynamic Zoom on (notice, "No more prompts" ticked), off; Direction Flip on, off; Locked Target on (no notice), off |
| 13:08:1x | right-click on the header, a switch, the slider; hover on a gesture icon |
| 13:09:18 | Zoom, Dynamic Zoom, Direction Flip on, factor 3, black tile, mirror on; OBSBOT Center closed and started again |
| 13:10:24-13:11:05 | factor typed `2` with Zoom off, everything back to the defaults; Device Sleep folded and unfolded; Auto Sleep off and on at 10min |

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (not
  reread).
- Official app: OBSBOT Center `v2.0.17.532` (not reread).
- OS: Windows 10 Pro 19045. Preview closed, no AI mode running.

## Observed: the UI

**Device Sleep**, a foldable section (click anywhere on the header):

- **Auto Sleep** switch, on by default. Off hides Sleep Time and its hint.
- **Sleep Time** drop-down: 30s, 2min, 10min (2min was set). A dim hint
  below says the camera sleeps after this time while no video stream runs.
- **Sleep Background** with a `?`. The `?` opens a Tips box: black frames by
  default, and the user can add pictures or videos (formats and a 10 s limit
  listed). Four tiles below: black, two pictures and the camera's standby
  badge; the fourth was selected. The selected tile has a red frame. No
  hover text, no right-click menu, no add button at this window size.
- **Sleep Background Mirror** switch, off.

**Gesture Control**, a foldable section with a `?` after the title. The
`?` opens a Tips box with three animated examples: Locked Target (a raised
open palm), Zoom (one raised hand), Dynamic Zoom (both hands).

- **Locked Target** (palm icon, "Turn on/off Tracking"), **Zoom** (pointing
  hand, "Execute zoom in/out"), **Dynamic Zoom** (two hands, "Zoom in/out
  with hands"): each a switch, all off.
- **Zoom Factor**: slider and field with arrows, `2.00 x` by default,
  1.00 to 4.00, arrows step 0.01 and stop at the ends. Typed `2.5` gave
  2.50, `1.234` gave 1.23, `5` and `0.5` were refused (the old value
  stayed). The slider stays enabled with Zoom off.
- **Direction Flip** switch, off, no icon.
- Turning Locked Target, Zoom or Dynamic Zoom on opens an "Activation
  Success Notice": it advises turning the gesture off when hand gestures
  are frequent in the shot, with a "No more prompts" box and "Got it".
  After the box was ticked, no notice appeared again.
- No right-click menus or tooltips anywhere in either section.

After the restart at 13:09 every value was shown as it was set.

## Observed: USB

Device Sleep, selector 6 blocks (byte 1 is the number of bytes after it):

- Sleep Time: `0b 02` and an i16 in seconds: `1e 00` 30, `78 00` 120,
  `58 02` 600.
- Auto Sleep off sends the negative of the current time: `88 ff` (-120),
  `a8 fd` (-600). On sends the positive time again.
- Sleep Background: `0e 02 00 n`, n 0 to 3 in tile order.
- Mirror: `0e 02 02 v`.
- The status block shows the time at offsets 10-11 (same i16, negative
  while off), the background at 27 and the mirror at 29.
- After wake the app read four files on selector 4 whose names point to
  three pictures and one video for the tiles; the third tile is the video.

Gesture Control, all `44 34` to the gimbal, flags `25`, data a u32 key and
then the value:

| key | control | value |
| --- | --- | --- |
| 1 | Locked Target | u8 0 or 1 |
| 2 | Zoom | u8 |
| 3 | Dynamic Zoom | u8 |
| 7 | Direction Flip | u8 |
| 8 | Zoom Factor | f32 ratio (`00 00 00 40` is 2.0) |

- Each Zoom Factor change also zoomed the camera to that ratio: a UVC zoom
  write, the usual `42 19` ratio message and the key 8 message again. This
  happened with the Zoom gesture off too (13:10:28).
- Status offset 1 was 1 while Locked Target was on and 0 otherwise. No
  status byte changed for the other gestures.
- On start the app read no gesture state. It sent `44 34` key 6 with 0.

## Inferred

- The official app remembers Zoom, Dynamic Zoom, Direction Flip and the
  factor itself, since the camera never reports them.
- Status offset 1 is not part of the block marker. Gimbly required
  `2e 00`, so with Locked Target on it would have ignored the whole status
  block (sleep, AI mode, HDR and the rest).

## Unknown

- What key 6 means.
- Whether Direction Flip shows the notice (it was tried only after "No
  more prompts").
- How pictures or videos are added to the camera (the selector 4 file
  transfer was not studied). Gimbly offers only the four built-in choices
  and draws its own stand-ins, not the camera's pictures.
- Whether the camera keeps the gesture settings across power loss.
- The exact hand gestures; Gimbly's Tips text describes the official
  app's icons and examples in our own words.

## Gimbly

- More tab: both sections with the same controls, order, defaults and
  folding. Typed factors outside 1-4 are refused. Changing the factor
  sends key 8 and zooms the camera, like the official app.
- Locked Target and the Device Sleep values come from the status block.
  Zoom, Dynamic Zoom, Direction Flip, the factor and "Don't show again"
  are kept in Gimbly's settings (`[Gestures]`).
- The notice shows for the three hand gestures, not Direction Flip.

## Tests

- `camera::vendor::tests::device_sleep_matches_capture`,
  `gestures_match_capture` and
  `locked_target_does_not_hide_the_status_block` (regression for the
  status block) use captured bytes.
- `app::view_more::tests` check the time labels and typed factors.
- `settings::tests` round-trip the gesture settings.

## Checked in Gimbly

2026-11-28, same camera and OS, OBSBOT Center closed, with a USB capture:
`gimbly --probe --virtual-camera --gimbal DIR wake sleeptime:30
sleeptime:-30 sleeptime:600 sleepbg:0 sleepmirror:on sleepbg:3
sleepmirror:off gesture:locked:on … gesture:flip:off gesture:factor:2.5
gesture:factor:2` ran every step without an error. The status read after
each step showed the new time, picture, mirror and Locked Target. The
captured blocks and messages matched the official app's byte for byte.
The camera was left at 2min, picture 4, mirror off, zoom 1.0, all gestures
off.

Running app, 13:34-13:39, same camera, OBSBOT Center closed:

- The More tab read the camera (2min, picture 4, mirror off).
- Sleep Time 30s and 2min, Auto Sleep off (time hidden) and on, all four
  tiles, mirror on and off: each value held after the next status poll.
- Both `?` buttons opened their Tips; a click on the `?` did not fold the
  section. Both sections fold and unfold; right-click does nothing.
- Locked Target on showed the notice and stayed on after the poll (status
  offset 1). Zoom on showed it again; with "Don't show again" ticked,
  Dynamic Zoom and Direction Flip showed nothing.
- Zoom Factor dragged to 1.00 and 4.00, up arrow at 4.00 stayed, down
  arrow gave 3.99, typed `2.5` and `1.234` (1.23) taken, `5` refused. The
  Console zoom showed 1.23x, so the camera zoomed with the factor.
- After closing and starting Gimbly again, Zoom, Dynamic Zoom, Direction
  Flip and 1.23 were restored from `[Gestures]`.
- The first build showed a help icon after Direction Flip, which the
  official app does not have; it was removed and the switch checked again.

The camera and Gimbly's settings were left as they were before the test.
