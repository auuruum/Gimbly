<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/gimbly-logo-dark.svg">
    <img src="assets/logo/gimbly-logo-light.svg" alt="Gimbly" width="420">
  </picture>
</p>

# Gimbly

A free, open-source desktop app for controlling OBSBOT webcams on Linux,
Windows, and macOS: gimbal, zoom, presets, sleep/wake, and more. Written in
Rust with egui.

**Unofficial.** Gimbly is not made, approved, or supported by OBSBOT or Remo
Tech.

## Supported webcams

| Camera | 🪟 Windows | 🐧 Linux | 🍎 macOS |
| :--- | :---: | :---: | :---: |
| 📷 **OBSBOT Tiny 3 Lite** | ✅ | ✅ | 🚧 |
| 📷 Other OBSBOT models | ❔ | ❔ | ❔ |

### 📷 OBSBOT Tiny 3 Lite

| Feature | 🪟 Windows | 🐧 Linux | 🍎 macOS |
| :--- | :---: | :---: | :---: |
| 🕹️ Gimbal joystick and drag-to-turn | ✅ | ✅ | 🚧 |
| 🎯 Gimbal reset | ✅ | ✅ | 🚧 |
| 🔍 Zoom 1×–4× | ✅ | ✅ | 🚧 |
| 🖼️ Field of view (Wide / Medium / Narrow) | ✅ | ✅ | 🚧 |
| 📌 Presets | ✅ | ✅ | 🚧 |
| 💤 Sleep / wake | ✅ | ✅ | 🚧 |
| 🎥 Virtual camera | ✅ | ✅ | 🚧 |
| 🤖 AI tracking | 🗓️ | 🗓️ | 🗓️ |
| 🎨 Image settings | 🗓️ | 🗓️ | 🗓️ |
| ⏺️ Recording | 🗓️ | 🗓️ | 🗓️ |

✅ tested on real hardware · 🚧 in progress · 🗓️ planned · ❔ not tested yet

Full test results are in [docs/devices.md](docs/devices.md). Tried another
camera? See [CONTRIBUTING.md](CONTRIBUTING.md#tested-a-camera). To add
support for your model, follow [docs/adding-a-camera.md](docs/adding-a-camera.md).

## Status

Early development. Capture backends use Media Foundation on Windows,
V4L2 on Linux and AVFoundation on macOS. Linux supports native Wayland
and X11. Virtual camera setup uses DirectShow on Windows, v4l2loopback on
Linux and a signed Core Media I/O system extension on macOS.

On Linux, click **Virtual Camera** and approve the first-time driver setup
when prompted. Once the icon lights up, choose **Gimbly Virtual Camera** in
your video app. See [Linux setup](CONTRIBUTING.md#linux-wayland-and-x11) for the driver and
terminal instructions. To see the tracking box in the preview, install the
udev rule described there so Gimbly can read the camera's serial port.

Live preview, rotate and mirror, snapshots, camera controls and Software
Settings share the same app interface. AI tracking, image settings and
recording are planned. See [CONTRIBUTING.md](CONTRIBUTING.md#build-and-run)
for setup and [docs/devices.md](docs/devices.md) for hardware verification.

## How it is built

Gimbly is a clean-room implementation: we observe how the official app and the
camera behave, write that down in our own words in
[docs/clean-room](docs/clean-room/README.md), and implement it from scratch.
No OBSBOT code, SDK, assets, or documentation are used, and no OBSBOT software
is decompiled. This lowers the risk of copying; it is not a legal guarantee.

## Contributing

Build instructions and the contributor guide are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
