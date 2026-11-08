# Gimbal joystick

## Source and method

- 2026-11-08: the USB capture described in
  [2026-11-08-vendor-messages.md](2026-11-08-vendor-messages.md); the owner
  dragged the panel joystick of OBSBOT Center six times (full and partial
  pulls, different directions, different Gimbal Speed settings). The exact
  order and settings the owner used were not written down.
- 2026-11-08: standard controls read through `IAMCameraControl`
  (`gimbly --probe --controls`).
- 2026-11-08: black-box test through Gimbly's own command path
  (`gimbly --probe --gimbal DIR 0:10:1000 10:0:1000 0:-10:1000 -10:0:1000`),
  saving a preview frame after each turn and comparing the frames.

## Setup

Tiny 3 Lite (`3564:ff04`), firmware **unknown**, OBSBOT Center version
**unknown**, Windows 10 Pro 19045.

## Observed

- Standard UVC controls exist: pan absolute -130..130, tilt absolute
  -90..90, roll -180..180 in 90° steps, zoom 0..100, pan relative speed
  1..80, tilt relative speed 1..120; there is no combined pan/tilt relative
  control. The official app did not use any of them for the joystick.
- A joystick drag in the official app sends, to destination `04`, with flag
  `25` and no reply:
  1. command `44 02`, data `00` when the drag starts;
  2. command `84 64`, 12 bytes: three `f32` values, the first always 0, then
     pitch and yaw, about every 100 ms while the knob is held, even when it
     does not move;
  3. `84 64` with all zeros, then `44 02` with data `01` about 20 ms later
     when the knob is released.
- The length of (pitch, yaw) at full pull was 30, 20 or 10 in different
  drags; partial pulls gave smaller values (for example 4.2).
- Black-box test: yaw +10 for one second turned the view to the right
  (the picture moved left) and the UVC pan reading went from 0 to -6; yaw
  -10 turned it back. Pitch +10 turned the view down; pitch -10 back up.

## Inferred

- The first float is roll and stays 0 on a two-axis gimbal.
- Full-pull speeds 30 / 20 / 10 fit "Gimbal Speed 5 / 3 / 1" with the top
  speed `30 * (level + 1) / 6`, and Fine as a third of Coarse; this is what
  `crate::gimbal` uses.
- The UVC pan reading counts the other way round from the vendor yaw.

## Unknown

- How far the knob must travel for a given speed (the curve between centre
  and full pull); Gimbly uses a linear curve.
- Which drag used which Gimbal Speed level and Coarse/Fine mode, so levels
  2 and 4 and the Fine factor are not confirmed.
- What the Video Preview drag-to-turn sends (not captured; Gimbly sends the
  same commands as the panel joystick).
- The unit of the speed values (10 for one second gave about 6-7° of pan,
  including the start-up ramp).

## Test

`camera::vendor::tests::encodes_captured_messages` (start and speed
messages byte for byte), `gimbal::tests` (speed curve).
