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
| 📷 **OBSBOT Tiny 3 Lite** | ✅ | 🚧 | 🚧 |
| 📷 Other OBSBOT models | ❔ | ❔ | ❔ |

### 📷 OBSBOT Tiny 3 Lite

| Feature | 🪟 Windows | 🐧 Linux | 🍎 macOS |
| :--- | :---: | :---: | :---: |
| 🕹️ Gimbal joystick and drag-to-turn | ✅ | 🚧 | 🚧 |
| 🎯 Gimbal reset | ✅ | 🚧 | 🚧 |
| 🔍 Zoom 1×–4× | ✅ | 🚧 | 🚧 |
| 🖼️ Field of view (Wide / Medium / Narrow) | ✅ | 🚧 | 🚧 |
| 📌 Presets | ✅ | 🚧 | 🚧 |
| 💤 Sleep / wake | ✅ | 🚧 | 🚧 |
| 🎥 Virtual camera | ✅ | 🚧 | 🚧 |
| 🤖 AI tracking | 🗓️ | 🗓️ | 🗓️ |
| 🎨 Image settings | 🗓️ | 🗓️ | 🗓️ |
| ⏺️ Recording | 🗓️ | 🗓️ | 🗓️ |

✅ tested on real hardware · 🚧 in progress · 🗓️ planned · ❔ not tested yet

Full test results are in [docs/devices.md](docs/devices.md). Tried another
camera? See [CONTRIBUTING.md](CONTRIBUTING.md#tested-a-camera).

## Status

Early development. Windows has a camera backend; Linux and macOS builds
currently lack camera capture.

The Windows app has the control panel, live preview with rotate and mirror,
snapshots, UVC zoom, a virtual camera (DirectShow, 64-bit apps), and
Software Settings. AI tracking, image settings, and recording are planned.

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
