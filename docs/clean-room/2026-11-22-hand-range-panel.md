# Hand Tracking Set panel: moves, resets and the range question

Follows [AI Mode](2026-11-22-ai-mode.md), which covers opening, Done and
Cancel.

## Source and method

2026-11-22, 14:17–14:28. Gimbly closed, OBSBOT Center the only app using
the camera. Claude Code clicked the visible OBSBOT Center UI with one
control per step (left and right clicks on every element of the panel)
and logged the time of each click.

A first USB capture (14:17–14:24) was started on `\\.\USBPcap4` and held no
packets: the camera had moved to `\\.\USBPcap2`. Its UI observations are
used below. A second capture on `\\.\USBPcap2` (USBPcap, snap length 512,
camera at address 1) covered a shorter repeat from 14:25 to 14:27. The
captures stay outside the repository and were decoded with our own
script.

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.

## Observed

### UI

- Each limit (30.0°, −30.0°, 45.0°, −45.0°) and the P/T/Z line is a
  button. A left click turns the gimbal; a right click shows one entry,
  **Update**.
- Clicking −45° and then 30° gave P −45.00°, T 30.00° on the next Update:
  a limit click moves only its own axis and keeps the other.
- Two circular icons sit to the right of the two explanation texts, one
  for the range and one for the return position. Right-clicking them, the
  joystick or the zoom spin box showed no menu.
  - Range icon: the limits went back to 45.0°, −45.0°, 30.0°, −30.0°
    (after the right limit had been updated to −55.4°).
  - Return icon: the line went back to P 0.00°, T 0.00°, Z 1.00. The
    gimbal and zoom did not move.
- **Reset** at the bottom of the panel turned the gimbal to the centre
  and set the zoom to 1.00x with Wide highlighted (Narrow, 1.42x, before).
- Clicking the P/T/Z line (P 44.90°, T −30.00°, Z 1.42) after Reset turned
  the gimbal back and set the zoom to 1.42x with Narrow highlighted.
- Update on a limit that would leave opposite limits less than 10° apart
  showed "Update failed, the current angle is …" with OK, and changed
  nothing.
- Update on the return position while the gimbal was outside the range
  (P −55.60°, T 30.20°, Z 1.42 with the right limit −45° and top 30°)
  asked "The set position is beyond the Tracking Range. Would you like to
  update the Tracking Range?" with No and OK.
  - OK: the right limit became −55.6° and the top 30.2°; the other two
    stayed; the line showed the new position.
  - No: the line kept its old value.
- The camera slept twice (Auto Sleep) during the first run. After Resume
  the app closed the panel by itself.

### USB (second capture)

All gimbal messages go to `0x04` on selector 2, flags `25`.

| click | write |
| --- | --- |
| limit −45° (right) | `44 64`, `f32` −1000, −1000, −45 |
| limit 30° (top) | `44 64`, `f32` −1000, −30, −1000 |
| limit 45° (left) | `44 64`, `f32` −1000, −1000, 45 |
| limit −30° (bottom) | `44 64`, `f32` −1000, 30, −1000 |
| Narrow | selector 6 `04 01 02` (known) |
| Update (return) | reads only (`04 66`, `04 68`); no write |
| Reset (panel) | `04 39` with data `01` |
| P/T/Z line | selector 11 `0e 00` on interface 0, then `44 64`, `f32` −1000, 30, 44.9 |
| both reset icons | nothing |
| Cancel | `18 01 00`, `c4 26 01`, `84 68` (known) |

## Inferred

- `44 64` turns the gimbal to absolute angles. Its three `f32` are roll,
  pitch, yaw in degrees in the camera's sign (the panel shows tilt
  negated). −1000 means "keep this axis".
- `04 39` with data `01` is the gimbal reset that also zooms to 1.0x; the
  earlier note saw `04 39` with no data and the zoom unchanged.
- Selector 11 `0e 00` is the standard UVC zoom absolute value 14 for
  1.42x; Gimbly sets the same ratio through its vendor zoom message.
- The reset icons and the range question only change the values the panel
  holds; Done writes them as before.
- OK widens only the limits the return position crosses, to its exact
  value.

## Unknown

- Whether the console's own Reset now also sends `01`. It was not
  captured this time.
- Why the first run once showed the left limit as "-0.0°" after the range
  icon while the camera slept.
- What "Instructions for Use" opens; it was not clicked.

## Test

- `camera::vendor::tests::hand_range_panel_moves_match_capture`: the four
  limit moves, the return move, the panel Reset and the default return
  record.
- `camera::tests::return_outside_the_range_widens_only_the_crossed_limits`:
  the OK case above.

Hardware: Windows 10 with the Tiny 3 Lite as above. Gimbly with the preview
and the virtual camera on: limit clicks, Update with the range question
(OK), panel Reset, P/T/Z click and both icons behaved as observed, and
frames kept flowing. Still to check: macOS, native Wayland and native X11.
