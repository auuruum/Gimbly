# Tested cameras and systems

Only what was tried on real hardware is listed. "Compile only" means the
code builds for that system but was not run there.

| Camera | USB ID | bcdDevice | Firmware | OS | Feature | Result | Date |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Gimbal speed commands (pitch and yaw, both directions) through `gimbly --probe --gimbal` | works | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Gimbal reset, Wide/Medium/Narrow FOV and 1×–4× vendor zoom | verified through the capture session and saved frames | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Reset hotkey in the running app | verified with a temporary Ctrl+Alt+F9 binding, camera readback and saved frames | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Sleep and wake | verified through the capture session, selector 6 status and saved frames | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | unknown | Windows 10 build 19045 | Virtual camera output on Resume, 1080p30 NV12 | verified with a local DirectShow client: Gimbly placeholder then live video | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | unknown | Windows 10 build 19045 | Sleep state and virtual camera across repeated Sleep/Resume, 1080p30 NV12 | verified with the hardware probe and a local DirectShow client: stable sleep state and continuous placeholder until live video | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Preset list, add, rename, update, recall and delete | verified through the capture session, camera readback and saved frames using a temporary slot | 2026-11-08 |

The hardware probe exercises the same capture commands as the app.
The joystick, Video Preview drag and panel buttons still need direct UI
verification. Linux and macOS camera control remain untested.
