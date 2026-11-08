# Contributing to Gimbly

Thanks for helping. This page is the short version; [AGENTS.md](AGENTS.md)
has the full rules and applies to everyone, human or AI.

## Build and run

You need stable Rust 1.95 or newer.

```bash
cargo build --workspace
cargo run -p gimbly
```

Build the whole workspace, not only `gimbly`: the virtual camera DLL is a
separate crate and must sit next to `gimbly.exe`. Close any other app that
uses the camera first.

## Make a change

1. Branch from the latest `dev`:

   ```bash
   git fetch
   git switch --no-track -c feature/short-name origin/dev
   ```

   Prefixes: `feature/`, `fix/`, `docs/`, `refactor/`, `chore/`.

2. Keep the change small. Add tests for logic that works without a camera.
3. Before you open a pull request, run:

   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

4. Open the pull request against `dev`, never `main`.

## Rules that matter most

- **Clean room.** Never decompile, disassemble, or debug OBSBOT software or
  firmware, and never use the OBSBOT SDK. Learn from the official app's
  visible behavior and from your own camera, then write what you found in
  [docs/clean-room](docs/clean-room/README.md).
- **Never crash.** No `unwrap`, `expect`, or `panic!` on data from the
  camera, a file, or the user.
- **Never block the video.** The UI and frame loop must not wait on camera
  writes.
- **All three platforms.** Windows, Linux, and macOS. If a feature is missing
  on one, return a clear "unsupported" error.
- **Icons** are SVG or egui-phosphor, never emoji.

## Tested a camera?

Add a row to [docs/devices.md](docs/devices.md) with the camera, firmware,
OS, what you tried, and the result. Only list what you ran on real hardware.

## License

By contributing you agree that your work is released under the
[MIT license](LICENSE).
