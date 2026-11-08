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

Early development. Nothing works yet.

## How it is built

Gimbly is a clean-room implementation: we observe how the official app and the
camera behave, write that down in our own words in
[docs/clean-room](docs/clean-room/README.md), and implement it from scratch.
No OBSBOT code, SDK, assets, or documentation are used, and no OBSBOT software
is decompiled. This lowers the risk of copying; it is not a legal guarantee.
See [AGENTS.md](AGENTS.md) for the full rules.

## License

MIT — see [LICENSE](LICENSE).
