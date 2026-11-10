# Linux virtual camera

## Source and method

2026-11-10. Used the public [v4l2loopback setup documentation](https://github.com/v4l2loopback/v4l2loopback#options),
[device management manual](https://manpages.debian.org/unstable/v4l2loopback-utils/v4l2loopback-ctl.1.en.html),
[V4L2 streaming parameter specification](https://docs.kernel.org/userspace-api/media/v4l/vidioc-g-parm.html)
and [polkit manual](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html).
Observed Gimbly and Linux devices on the host and in a QEMU/KVM guest.
OBSBOT Center was outside the test.

## Setup

CachyOS x86_64, Linux 7.3.3-1-cachyos with v4l2loopback 0.15.4. The host
had an OBSBOT Tiny 3 Lite, USB `3564:ff04`, descriptor revision `0x0510`,
firmware release unknown. The guest used the kernel's vivid test camera
and v4l2loopback. Native Wayland was Hyprland 0.57.1. Native X11 was Xvfb.

## Observed

- The host had the module on disk and no loopback video node. Gimbly's
  setup window detected that combination and offered to create a dedicated
  device. The window was opened and visually checked on native Wayland
  and native X11.
- In the guest, the documented `modprobe` arguments created a separate
  output named `Gimbly Virtual Camera` with an automatically allocated
  video device number. With Gimbly publishing, the V4L2 client negotiated
  YUYV at 30 fps and received 20 complete frames in 640×480, 1920×1080
  and 720×1280 portrait modes. The first and last frame checksums
  differed in both landscape modes. Two independent 640×480 runs
  succeeded in the same guest. Another client received 350 frames
  across a producer restart without reopening its capture stream.
- On the host, the release probe opened the Tiny 3 Lite in MJPEG
  1920×1080 at 30 fps and received live preview frames.

## Inferred

An exclusive-capabilities loopback is initially an output node. Video
clients can enumerate it as a camera after the producer sends frames.
Gimbly keeps setup off the capture thread and highlights the camera
button only after the first complete frame reaches its output.

## Unknown

The system authentication approval and a receiving video app with the
physical Tiny 3 Lite still need a run on the host. No new Windows or
macOS runtime test was performed.

## Tests

The Linux tests check node discovery, exact device label and setup
arguments. The shared frame test checks the active indication after
publication and its reset on failure. The guest used
`gimbly --probe --virtual-camera --mode 1920x1080@30` and a separate
`v4l2-ctl --stream-mmap` capture. To repeat with the physical camera,
enable Virtual Camera in Gimbly and select its output in another app.
