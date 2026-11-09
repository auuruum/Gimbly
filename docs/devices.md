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
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | unknown | Windows 10 build 19045 | Preview frames and virtual camera output on Resume, 1080p30 NV12 | three probe cycles: preview frames stopped during Sleep and resumed after wake. DirectShow received the placeholder followed by frames with new checksums | 2026-11-08 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | Windows 10 Pro 19045 | Preset list, add, rename, update, recall and delete | verified through the capture session, camera readback and saved frames using a temporary slot | 2026-11-08 |

| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3 x86_64 | MJPEG preview at 1080p30, 1080p60 and 4K30 | release capture probe delivered live frames in all three modes. A 720p30 portrait probe saved 404×720 frames at 1× and 4× | 2026-11-09 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3 x86_64 | Gimbal movement, Reset, FOV, zoom, sleep/wake and presets | capture probe, camera readback and snapshots, including sleep placeholder | 2026-11-09 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3, X11 client under XWayland | Panel joystick, preview drag, Reset shortcut | position changed in camera readback with preview open | 2026-11-09 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3, X11 on Xvfb | Live preview, rotation, mirror, snapshots and X11 global Snapshot | verified in running app, PNGs saved at 1920×1080 | 2026-11-09 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3, native Wayland on Hyprland | Live preview, rotation, snapshots and portal Snapshot | verified in running app with `DISPLAY` unset, compositor activation saved a PNG | 2026-11-09 |

The 2026-11-08 Windows rows describe the earlier Windows build. The
cross-platform change was checked by the compiler and Clippy for
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, including the camera
extension for macOS. Native runtime checks of those builds are pending.
Linux v4l2loopback output needs a run with a configured node.
See the [Linux evidence log](clean-room/2026-11-09-linux-capture.md).
