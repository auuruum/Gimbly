# AI tracking options: settings the camera reports

Goal: Zone Tracking, Sound Source Assisted Tracking, Object Tracking (Normal,
Close-up) and AI Mode (Voice Tracking, Hand Tracking). This note records
what we could learn **without** clicking those controls. The click-by-click
capture is still to do (see the end of this note).

## Source and method

2026-11-21.

1. We re-read our own three captures from the
   [Group Mode and Auto Zoom](2026-11-21-group-mode-auto-zoom.md) session
   with our own scripts, this time looking at the `84 54` setting reads the
   official app sends while it runs. None of these options was clicked in
   those captures.
2. Black-box reads with a temporary local build of Gimbly (not committed):
   one `84 54` read for each key from 0 to 20, with Gimbly as the only app
   using the camera.
3. Black-box check of the new Group Mode readback with
   `gimbly --probe --gimbal DIR` in separate processes (see below).

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app (captures only): OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- In the captures, Zone Tracking and Sound Source Assisted Tracking were
  on, no AI Mode was active, and Tracking Speed was Standard.

## Observed

- The app reads setting keys 1, 2, 3, 5, 8 and 11 with `84 54` (request
  `u32 0`, `u32 key`). Key 5 is the
  [tracking speed](2026-11-21-tracking-speed.md). The other values stayed
  the same through all three captures, except key 2:

  | key | reply | note |
  | --- | --- | --- |
  | 1 | `01` (once `01 00 00 00`) | constant |
  | 2 | `00` or `01` | follows Human Tracking mode, see below |
  | 3 | `01 00 00 00` | constant |
  | 8 | `00` | constant |
  | 11 | `00` | constant |

- **Key 2** in the second capture (owner in view): the app read it only
  while tracking was on. It read `00` in each of 8 reads after Single Mode
  was turned on, and `01` in each of 3 reads after Group Mode was turned on.
  The value changed within 2.3–4.3 s of the mode writes over four
  switches. In the dark-room capture, where tracking never engaged, the app
  did not read key 2 at all.
- Our own reads of keys 0–20: keys 0–12 answered and keys 13–20 got no
  reply. Values with Human Tracking on in Single Mode: 0 `00`, 1 `01`,
  2 `00`, 3 `01 00 00 00`, 4 `00 00 00 00`, 5 `02 00 00 00`, 6 `00`,
  7 `00 00 00 3f`, 8 `00`, 9 `00`, 10 `00 00 00 3f`, 11 `00`,
  12 `00 00 00 00`.
- At connect the app also reads command `02 ad`. The camera answered
  `0a 08 08 01 10 01 18 01 20 01`, in the protobuf-like form described in the
  [Human Tracking capture](2026-11-21-human-tracking-capture.md): field 1
  holding fields 1–4, each `1`.
- **Gimbly black-box check:** with tracking on in Single Mode, a new Gimbly
  process read key 2 = `0` and showed Single Mode. After the `group` step
  in another process, the next process read Group Mode. After `track`,
  the next process read Single Mode again. In one of five starts the first
  read got no reply, and the mode stayed unknown for that session.

## Inferred

- Key 2 is the Human Tracking mode (`0` Single, `1` Group). It answers the
  open question in the Group Mode note about where the camera reports the
  mode.
- Keys 1 and 3 read `1` while Zone Tracking and Sound Source Assisted
  Tracking were both on, and keys 8 and 11 read `0` while no AI Mode was
  active. These are **candidates** only. Nothing yet ties a key to a
  control, and some of them may be unrelated.
- The four `1` fields in the `02 ad` reply might be feature flags. This is
  unconfirmed.

## Unknown

- The writes behind Zone Tracking (and its settings dialog), Sound Source
  Assisted Tracking, Object Tracking Normal and Close-up (including how a
  selection in the preview is sent), Voice Tracking and Hand Tracking.
- What keys 0, 1, 3, 4, 6–12 mean.

## Still to capture

A USB capture with OBSBOT Center, one click at a time, Gimbly closed and the
owner in view: Zone Tracking off/on, the Zone Tracking settings, Sound
Source off/on, Object Tracking Normal and Close-up with a box drawn in the
preview, Voice Tracking on/off and Hand Tracking on/off. In the 2026-11-21
session, Computer Use was not granted for OBSBOT Center, so these clicks
were not made.

## Test

`camera::vendor::tests::tracking_mode_matches_capture` checks the key 2
request bytes, decodes the captured `00` and `01` replies, and rejects
other values and lengths.

Hardware: the Group Mode readback was checked on Windows 10 with the Tiny 3
Lite as above. Still to check: macOS, native Wayland and native X11.
