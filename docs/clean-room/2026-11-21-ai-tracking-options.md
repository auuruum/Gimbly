# AI tracking options: Zone, Sound Source, Object Tracking, AI Mode

## Source and method

2026-11-21.

1. We re-read our own three captures from the
   [Group Mode and Auto Zoom](2026-11-21-group-mode-auto-zoom.md) session
   with our own scripts, this time looking at the `84 54` setting reads the
   official app sends while it runs.
2. Black-box reads with a temporary local build of Gimbly (not committed):
   one `84 54` read for each key from 0 to 20, with Gimbly as the only app
   using the camera.
3. Three new USB captures (USBPcap on `\\.\USBPcap4`, snap length 512,
   descriptors injected, only the camera at address 1), Gimbly closed.
   Claude Code clicked the visible OBSBOT Center UI and checked the switch
   or button highlight after each click, waiting 6–7 s between clicks:
   - **Capture A (Pro Mode):** Resume from sleep, Zone Tracking on, off;
     Sound Source Assisted Tracking off, on; the Zone Tracking gear (opens
     a settings panel), Auto-tracking new target on, off, the Non-tracking
     Area tab, Cancel; Voice Tracking on, off; Hand Tracking on, off.
   - **Capture B:** OBSBOT Center switched to Lite Mode and restarted; a
     box drawn and single and double clicks on an object in the built-in
     preview.
   - **Capture C (Pro Mode again):** the Video Preview window opened, boxes
     drawn on objects in it, the Object Tracking buttons inspected.
     The owner was at the PC during this capture and also used the app
     (joystick, the Zone Tracking panel with Done, and one more switch);
     those clicks are not attributed below.
4. Black-box checks with `gimbly --probe --gimbal DIR` and the new
   switches and buttons in the running app (see the end).

The capture files stay outside the repository.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- Human Tracking was in Single Mode at the start of capture A.

## Observed

### Setting keys

- The app reads setting keys 1, 2, 3, 5, 8 and 11 with `84 54` (request
  `u32 0`, `u32 key`) about every 2–3 s. Key 5 is the
  [tracking speed](2026-11-21-tracking-speed.md).
- Our own reads of keys 0–20: keys 0–12 answered and keys 13–20 got no
  reply. Values with Human Tracking on in Single Mode: 0 `00`, 1 `01`,
  2 `00`, 3 `01 00 00 00`, 4 `00 00 00 00`, 5 `02 00 00 00`, 6 `00`,
  7 `00 00 00 3f`, 8 `00`, 9 `00`, 10 `00 00 00 3f`, 11 `00`,
  12 `00 00 00 00`.
- **Key 2** in the earlier Group Mode capture (owner in view): the app read
  it only while tracking was on. It read `00` in each of 8 reads after
  Single Mode was turned on and `01` in each of 3 reads after Group Mode
  was turned on, within 2.3–4.3 s of the mode writes, over four switches.

### Zone Tracking

- **On:** one `44 54` write to the gimbal, flags `0x25`, data
  `u32 0`, `u32 3`, `u32 1`. **Off:** the same with value `0`.
- Key 3 then read `00 00 00 00` after off.
- After each click the app also exchanged the `82 ad` / `c2 ad` records
  described in the [Human Tracking capture](2026-11-21-human-tracking-capture.md).
- At the start of capture A the switch showed **off** while key 3 read
  `01 00 00 00`. We cannot explain this yet.
- During Hand Tracking the app greys out the Zone Tracking switch.
- **Settings panel (gear):** opening it sent `44 08` with data `00`, read
  `c4 4e` (data `ff`, reply of 28 bytes) and `84 38`, read keys 18–22, and
  then sent a message with destination `0x12`, command `0d 70`, about once a
  second while the panel stayed open. Keys 19–22 read as `f32` −100, 100,
  −30 and 30, which match the pan and tilt limits the panel showed.
  "Auto-tracking new target" on and off wrote key 18 with a one-byte value
  `01` / `00`. Cancel sent `44 08` with `01`, a `0x12` / `4d 70` message, and
  `84 68` with `1e 00 00 00` and `f32` 1.0. The zoom read 1.0× afterwards,
  where it had been 1.3× while the panel was open.

### Sound Source Assisted Tracking

- **Off:** one selector 6 SET_CUR, `25 01 00` and zeros. **On:**
  `25 01 01`.
- Offset 46 of the selector 6 status block followed: `0` about 0.15 s after
  off and `1` about 1.2 s after on.

### AI Mode

- **Voice Tracking on:** one selector 6 write `16 02 07 00`. Offset 24 of
  the status block changed from `2` (Single Mode had been running) to `7`,
  and offset 28 from `0` to `2`. The Single Mode highlight went off.
- **Voice Tracking off:** `16 02 00 00`; offsets 24 and 28 read `0`.
- **Hand Tracking on:** `16 02 03 00`; offset 24 read `3`.
- **Hand Tracking off:** `16 02 00 00`; offset 24 read `0`.
- Single Mode stayed off after either mode was turned off.

### Object Tracking

- In Pro Mode the Normal and Close-up buttons stayed disabled through every
  state we tried: tracking off, Single Mode on, and boxes drawn in the Video
  Preview window.
- In Lite Mode the app shows no Object Tracking control. Drawing a box and
  clicking an object in its preview produced no write to the camera.
- No capture contained a write we could tie to Object Tracking.

### Gimbly black-box checks

- **Group Mode readback (key 2):** with tracking on in Single Mode, a new
  Gimbly process read Single Mode. After the `group` step in another
  process, the next process read Group Mode, and after `track` it read
  Single Mode again. In one of five starts the first read got no reply,
  and the mode stayed unknown for that session.
- **Probe readback:** each of `sound:false`, `sound:true`, `zone:true`,
  `zone:false`, `voice`, `noai`, `hand`, `noai` ran in its own process. A new
  process started 3 s later read the same state from the camera: offset 46
  for Sound Source, key 3 for Zone Tracking, offset 24 for the AI Mode.
- **Running app:** Sound Source off and on, Hand Tracking (the Zone
  Tracking switch greyed out), Voice Tracking (switching from Hand), and
  Voice Tracking off. Each switch or button kept its state after the next
  status read.
- **Video kept flowing:** `--virtual-camera --gimbal` ran 12 steps that
  alternated joystick moves with Hand, Voice, Sound Source and Zone Tracking
  writes. A frame arrived after every step and the virtual camera stayed
  active. The frame rate was not measured in this run.

## Inferred

- Selector 6 command `0x16` sub-command `02` starts Human Tracking, `03`
  Hand Tracking and `07` Voice Tracking, and `00` stops whichever runs.
  Offset 24 of the status block holds the running sub-command. This
  explains why only one AI tracking runs at a time.
- Key 2 is the Human Tracking mode (`0` Single, `1` Group).
- Key 3 is Zone Tracking. Keys 19–22 hold the zone limits and key 18 holds
  "Auto-tracking new target". `44 08 00` / `01` may pause and resume
  tracking while the zone panel is open.
- Selector 6 command `0x25` switches Sound Source Assisted Tracking, and
  offset 46 reports it.
- OBSBOT Center does not offer Object Tracking for the Tiny 3 Lite on this
  firmware.

## Unknown

- Why the Zone Tracking switch showed off while key 3 read `1`.
- The layout of the `c4 4e` reply, the `84 38` reply, and the `0x12`
  `0d 70` / `4d 70` messages, and how the zone limits and Non-tracking
  Areas are written. Gimbly does not implement the zone settings panel yet.
- The meaning of status offset 28 during Voice Tracking, and of keys 0, 1,
  4 and 6–12. In capture C a write of key 1 (`0`, then `1`) followed an app
  click by the owner that we did not see.
- The purpose of the `82 ad` / `c2 ad` record exchange.
- Object Tracking on other models.

## Test

- `camera::vendor::tests::tracking_mode_matches_capture`: key 2 request and
  replies.
- `camera::vendor::tests::ai_options_match_capture`: Voice, Hand and stop
  writes; Sound Source writes; status offsets 24 and 46; Zone Tracking
  write and read; rejects unknown values and short replies.
- `camera::vendor::tests::human_tracking_matches_capture`: offset 24 as
  Human Tracking.

Hardware: Windows 10 with the Tiny 3 Lite, as above. Still to check:
macOS, native Wayland and native X11.
