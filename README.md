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

## Status

Early development. Only Windows has a camera backend so far; Linux and macOS
build but cannot capture yet.

Works on Windows: the control panel, live video preview with rotate and
mirror, snapshots, zoom, the virtual camera (DirectShow, 64-bit apps), and
Software Settings. On the OBSBOT Tiny 3 Lite the gimbal joystick and
drag-to-turn in the Video Preview work too.

Not implemented yet: gimbal reset, sleep/wake, field of view, presets, AI
tracking, image settings, recording. Their controls are visible but either
disabled or report that they are not implemented.

## Build and run

You need a stable Rust toolchain (1.95 or newer).

```bash
cargo build --workspace
cargo run -p gimbly
```

Build the whole workspace: the virtual camera DLL is a separate crate and
must sit next to `gimbly.exe`. Close any other app that streams the camera
first.

## How it is built

Gimbly is a clean-room implementation: we observe how the official app and the
camera behave, write that down in our own words in
[docs/clean-room](docs/clean-room/README.md), and implement it from scratch.
No OBSBOT code, SDK, assets, or documentation are used, and no OBSBOT software
is decompiled. This lowers the risk of copying; it is not a legal guarantee.
See [AGENTS.md](AGENTS.md) for the full rules.

## License

MIT — see [LICENSE](LICENSE).
