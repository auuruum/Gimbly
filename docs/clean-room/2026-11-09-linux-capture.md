# Linux capture and controls

## Source and method

2026-11-09. Read the owner's camera descriptors from Linux sysfs and
queried the formats and terminal controls with `v4l2-ctl`. Ran Gimbly's
release build through the same capture session used by the GUI. Tested the
main panel and preview on Wayland and X11, and compared device readback
and locally saved frames. OBSBOT Center was not used for this test.

The native transport uses Linux's
[UVC extension unit query](https://docs.kernel.org/userspace-api/media/drivers/uvcvideo.html).
The vendor messages come from the existing Gimbly
[evidence log](2026-11-08-vendor-messages.md).

## Setup

OBSBOT Tiny 3 Lite, USB `3564:ff04`, descriptor revision `0x0510`.
Firmware release unknown. CachyOS, Linux `7.3.3-1-cachyos`, x86_64.
Native Wayland under Hyprland and native X11 under Xvfb. The panel
joystick and preview drag test also used a separate XWayland server.

## Observed

- The physical capture node reported MJPEG at 720p, 1080p and 4K, and YUYV
  at 640×360 and 640×480. The second video node exposed metadata.
- Gimbly found the captured extension GUID on video-control interface 0,
  unit 2. Both selectors used the recorded 60-byte size. V4L2 reads and
  writes worked while MJPEG frames were streaming. Some reads made before
  the capture had fully started returned a previous or missing vendor reply.
- The probe received preview frames before and after gimbal movement,
  Reset, each FOV setting, zoom and wake. The sleep snapshot used Gimbly's
  placeholder. The temporary preset was saved, renamed, recalled, updated
  with its name intact and deleted. The two original slots were still present.
- The short release probes reported 29.9 fps at 1080p30, 60.4 fps at
  1080p60 and 30.7 fps at 4K30 in their final measurement windows. A
  720p30 portrait probe produced 404×720 frames at both zoom limits
  (1× and 4×), then restored 1×.
- Dragging the panel joystick and dragging in the preview changed vendor
  position readback in the X11 app under XWayland. The Reset shortcut
  changed the position again. Native X11 on Xvfb opened a live preview,
  changed rotation and mirror, and saved 1920×1080 PNGs through the
  preview and a global shortcut. Native Wayland on Hyprland opened a
  live preview, rotated it and saved a PNG. With the desktop entry
  installed, Hyprland registered Gimbly's portal actions and invoking
  Snapshot through the compositor saved another PNG.

## Inferred

The matching extension descriptor and working requests let Linux reuse the
same protocol, reply checks and write pacing as Windows. AVFoundation
capture and macOS endpoint-zero transport use those same protocol checks.

## Unknown

The macOS USB ID mapping and concurrent AVFoundation and IOKit access
still need verification on a Mac with the camera. Linux virtual output and
the joystick with both preview and virtual output still need a run with a
configured v4l2loopback node. The module was installed on the test system,
but loading it required administrator access.

## Test

`camera::control::tests` checks the extension GUID and rejects a truncated
descriptor and a streaming interface. `camera::linux::tests` checks padded
YUYV and rejects truncated frames. The existing `camera::vendor::tests`
cover message encoding and reply checks. To repeat the streaming test,
run `gimbly --probe --mode 1920x1080@60`. After configuring virtual output,
enable it in the app and drag the joystick with both the preview and a
second video client open. The probe also accepts `--virtual-camera` before
`--gimbal` for checking the same command path with virtual output enabled.
