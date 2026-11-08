# Gimbal reset

## Source and method

2026-11-08, USBPcap on `\\.\USBPcap3` while clicking Reset in OBSBOT
Center, followed by a black-box test in Gimbly. OBSBOT Tiny 3 Lite
(`3564:ff04`, USB descriptor revision `0x0510`), Windows 10 Pro 19045.
The firmware release and OBSBOT Center version were not recorded.

## Observed

- Center sent `84 38`, then `04 39` to gimbal address `04`. The
  latter had flags `05` and no payload. It used interface 0, unit 2,
  selector 2.
- Gimbly turned the camera with a positive yaw command, then sent only
  `04 39`. Frames before and after Reset show the camera returning to
  its centre view. Reset also left the previously selected zoom in place
  during the preset test.
- In the running app, Reset bound to Ctrl+Alt+F9 in a temporary settings
  profile changed the UVC pan reading from −5 to +6 and brought the view
  back towards its earlier framing. The default Alt+R did not trigger
  a move in this session. The reason was not established.

## Inferred

The result suggests `84 38` is unnecessary for Reset on this unit.

## Unknown

The meaning of `84 38`.

## Test

`camera::vendor::tests::empty_commands_ignore_the_previous_payload`
compares the reset header and checks the stale payload bytes from the
capture. Hardware test with `gimbly --probe --gimbal DIR 0:15:1000 reset`.
