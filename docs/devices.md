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
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.3 x86_64 | MJPEG 1080p30 preview on the virtual camera branch | release capture probe received live frames at 30 fps | 2026-11-10 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | unknown | CachyOS Linux 7.3.5 x86_64 | 1080p30 preview and virtual camera | capture probe received preview frames and a separate V4L2 client read 45 YUYV frames from the loopback | 2026-11-20 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Tracking Speed (Slow, Standard, Fast) | probe readback across processes, OBSBOT Center showed the value Gimbly wrote, buttons in the running app | 2026-11-21 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Human Tracking Single/Group Mode and Auto Zoom | buttons in the running app with a person in view; camera status agreed with each write | 2026-11-21 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Preview Object Tracking (box, Normal, Close-up, stop), double-click target and cancel, AI Composition lines with Face Framing, tracked-target box from the CDC port | running app with a person in view; camera status, USB capture and the preview agreed with each action | 2026-11-22 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | not reread | CachyOS Linux 7.3.5, native Wayland | Tracked-target box from `/dev/ttyACM0` in the preview (Human Tracking Single Mode, Object Tracking) | owner saw both boxes in the running app with `DISPLAY` unset. Needed `packaging/linux/70-gimbly.rules` (uaccess); without it the `root:uucp 0660` port gave permission denied and no box | 2026-11-22 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | 0x0510 | not reread | CachyOS Linux 7.3.5, native Wayland on Hyprland | Serial port access dialog: Allow access through pkexec | with the rule removed and the camera replugged, the running app installed `/etc/udev/rules.d/70-gimbly.rules` (root, 0644, same bytes as the packaged rule) and the port got the user ACL; the copy-command path was not run | 2026-11-22 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Sound Source Assisted Tracking switch | switch in the running app held its state after the camera status poll; OBSBOT Center opened afterwards showed the value Gimbly wrote | 2026-11-24 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Zone Tracking switch, Tracking Range panel, Auto-tracking new target and Non-tracking Areas | running app with preview and virtual camera; probe readback; areas Gimbly saved showed in OBSBOT Center. Tracking with a person in the range not tested | 2026-11-24 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | High-fps Recording: 1080P 120 and 720P 120, Normal and Slow Motion, mkv/mp4/mov, pause, camera microphone WAV | running app with preview and virtual camera, joystick dragged while recording; files checked with ffprobe and decoded with ffmpeg; a test tone appeared in the WAV | 2026-11-25 |
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | HDR switch, enabled only at 30 fps | running app at 4K 30: switch held its state after the status poll and OBSBOT Center showed the value Gimbly wrote; at 1080P 60 the switch was greyed. Picture effect not judged | 2026-11-25 |

The 2026-11-08 Windows rows describe the earlier Windows build. The
cross-platform change was checked by the compiler and Clippy for
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, including the camera
extension for macOS. Native runtime checks of those builds are pending.
The Linux virtual camera delivered frames to a second process both with
synthetic input in a guest and with a physical Tiny 3 Lite on the host.
See the [virtual camera evidence log](clean-room/2026-11-10-linux-virtual-camera.md)
and the [physical-camera baseline](clean-room/2026-11-20-human-tracking.md).
| OBSBOT Tiny 3 Lite | `3564:ff04` | not reread | 6.5.11.1 | Windows 10 Pro 19045 | Focus: Auto Focus, Global/Face, Manual Focus | probe steps without errors; the camera status read in the next process matched each write; snapshots at positions 10 and 90 differed in sharpness. Panel in the running app not checked | 2026-11-26 |
