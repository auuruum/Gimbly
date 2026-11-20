# Human Tracking discovery

## Source and method

2026-11-20. Read the owner's camera descriptors with `lsusb -v -d 3564:ff04`
and listed controls on the physical video capture node with
`v4l2-ctl --list-ctrls-menus`. Ran Gimbly's capture probe with a separate
V4L2 consumer to establish a streaming baseline. No video was saved.

## Setup

OBSBOT Tiny 3 Lite, USB `3564:ff04`, descriptor revision `0x0510`.
Firmware release unknown. CachyOS, Linux `7.3.5-1-cachyos`, x86_64.
OBSBOT Center was outside this test.

## Observed

- The video control interface reported UVC 1.10. The camera terminal
  advertised pan, tilt, zoom and other imaging controls. The V4L2 control
  list contained no region-of-interest or tracking control.
- Extension unit 2 had the same GUID and selector bitmap as the device in
  the [vendor message log](2026-11-08-vendor-messages.md).
- With MJPEG input at 1920×1080 and 30 fps, the capture probe consumed 223
  preview frames in eight seconds. A separate V4L2 process read 45
  1920×1080 YUYV frames from the Gimbly loopback node during that run.

## Inferred

The generic controls exposed by the Linux driver provide no direct
Human Tracking toggle for this device.

## Unknown

The command payloads for starting and stopping Human Tracking and the
camera's state response are unknown. A short USB capture filtered to
this camera while switching tracking off, on, and off again in OBSBOT
Center can identify the traffic to compare. Include a few seconds
between changes so the capture contains subsequent status requests.

## Tests

The Linux baseline used `gimbly --probe --virtual-camera --mode 1920x1080@30`
and a separate `v4l2-ctl --stream-mmap=3 --stream-count=45` client once the
loopback exposed a capture format. Tracking still needs verification on
Windows, macOS, native Wayland, and native X11. The tracking test should
confirm the state after enabling, disabling, and reconnecting the camera,
with preview and virtual output active.
