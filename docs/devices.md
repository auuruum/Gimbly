# Tested cameras and systems

Only what was tried on real hardware is listed. "Compile only" means the
code builds for that system but was not run there.

| Camera | USB ID | bcdDevice | Firmware | OS | Feature | Result | Date |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Gimbal speed commands (pitch and yaw, both directions) through `gimbly --probe --gimbal` | works | 2026-11-08 |

Not yet tested on hardware: the joystick and Video Preview drag in the
running app, gimbal reset, sleep/wake, field of view, vendor zoom, presets,
Linux, macOS.
