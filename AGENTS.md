# Gimbly — agent instructions

Read this file first. It applies to every human and AI contributor.

## 1. Project goal

A free, open-source desktop app for controlling OBSBOT webcams on Linux, Windows, and macOS: gimbal, zoom, field of view, presets, sleep/wake, AI tracking, image settings, virtual camera.

The layout should feel familiar to people who know OBSBOT Center, so they do not have to relearn it. Everything is drawn in Gimbly's own code with Gimbly's own assets.

Gimbly is unofficial. Never imply that it is made, approved, or supported by OBSBOT or Remo Tech. Never put "OBSBOT" in the product name, binary names, crate names, or icons. Nominative use in text ("works with the OBSBOT Tiny 3 Lite") is fine.

The product name is written `Gimbly` in user-facing text; crates, binaries, and identifiers stay lowercase (`gimbly-*`).

## 2. Core requirements

- All first-party code is Rust. UI: egui on eframe with the wgpu renderer. No webview, Electron, or other languages in the app.
- Prefer pure-Rust dependencies and keep the tree small. Use OS libraries or FFI only for camera access, the virtual camera, and desktop integration; isolate that code in small platform modules and explain why it is needed.
- Windows, macOS, Linux Wayland, and Linux X11 are first-class targets. Gimbly runs as a native Wayland client in a Wayland session and as an X11 client in an X11 session. XWayland is a compatibility path and does not substitute for native Wayland support.
- Implement each feature for all four targets in the same task. Keep shared behavior in platform-neutral code and put native camera, windowing, shortcut, and desktop APIs behind shared interfaces. Use the Wayland compositor or desktop portal for operations that require its permission. Register global hotkeys through the XDG Desktop Portal `org.freedesktop.portal.GlobalShortcuts` on Wayland and native key grabs on X11. Let the portal handle consent and compositor bindings. Use the FileChooser portal for native Linux folder selection when the desktop provides it.
- Report features that the OS, compositor, or desktop portal cannot provide with a clear in-app message. Identify any setup or permission the feature needs. If a target could not be built or tested, record the gap and the verification that still needs to run.
- Keep the MIT license. Do not change it without the owner's approval.
- Icons: SVG or egui-phosphor. Never use Unicode characters or emoji as UI icons.

## 3. Clean-room implementation

Gimbly is a clean-room reimplementation: study features and behavior, document findings in our own words, implement from scratch in Rust.

- Allowed sources: public documentation (the USB Video Class spec, OS camera API docs, crate docs), observation of the official app's UI and behavior, USB captures of the owner's own camera, and black-box experiments on that camera.
- No reverse engineering of OBSBOT software: never decompile, disassemble, debug, or inspect the files or process memory of OBSBOT Center, libdev, the OBSBOT SDK, firmware, or any other OBSBOT binary. OBSBOT has forbidden this in writing, and the owner has told them it will not happen.
- Never use the OBSBOT SDK: do not link it or read its headers, samples, or docs.
- Do not copy proprietary code, firmware, UI assets, icons, artwork, sounds, screenshots, or substantial documentation text into the repository. Do not commit USB captures; describe them in the log.
- Do not read or copy source code of other OBSBOT control projects or of the old `Gimbly-OLD` / `OBSBOTter` repository. The starting code (UI, video, virtual camera, settings) was carried over from the owner's own old repository once, on 2026-11-08, without its vendor protocol, its decompile-based joystick curve, and its notes. Implement everything new from our own notes and hardware observations.
- A familiar layout and visually similar interface are explicit goals; draw everything in Gimbly's own code.
- Keep the behavior and evidence log in `docs/clean-room/`. For each entry record the source or method, date, camera model and firmware, official app version where used, what was observed, and the test that checks it. Keep **observed**, **inferred**, and **unknown** apart.
- A clean-room process lowers copying risk; it is not legal clearance. Never describe it as such.

## 4. Golden rules

- **Never crash.** No `unwrap`, `expect`, unchecked indexing, or `panic!` on data from the camera, a file, or the user. A camera that disconnects, sleeps, or answers garbage is a normal event. Treat every camera response as hostile: bound lengths and sizes.
- No `unsafe` outside small, documented platform modules; each `unsafe` block states its safety requirements.
- **Never block the video.** The UI thread and the frame loop must never wait on the camera. Camera control writes are slow (tens of milliseconds each), so:
  - Do not send one camera write per UI frame or per input event. Coalesce high-rate commands (joystick speed, sliders) so only the newest value is sent; keep order relative to other commands.
  - Never let a command queue grow faster than the camera drains it, and never drain it in a loop that can starve frame reads.
  - Every sleep or retry in the capture loop delays frames; keep them short and bounded, or move the work off the frame path.
  - Check any change to camera control by dragging the joystick with the preview and virtual camera running: frames must keep flowing.
- **Stay light.** Low startup time, idle CPU, and memory. No busy polling. Send a command to the camera only when the value changed. Measure before optimizing and never claim numbers you did not measure.
- Settings and camera state stay in plain serializable data; the UI is a view over it.
- Every crash fix comes with a regression test.

## 5. Workflow

### Branches and human approval

- `main` is the stable branch; `dev` is the integration branch.
- Start each task on its own branch from the latest `origin/dev` after fetching: `git switch --no-track -c <kind>/<short-task-name> origin/dev`. Kinds: `feature/`, `fix/`, `docs/`, `refactor/`, `chore/`. Never put the name of an agent, tool, or model in a branch name.
- Within the task, agents may edit, test, commit, and push the task branch without asking at each step. Push explicitly to the task branch, never `dev` or `main`.
- When the task is done, report the changes and validation results, then ask whether to merge the task branch into `dev`. Merge and push `dev` only after explicit human approval.
- Merge `dev` into `main` only when the owner asks for it. Approval to merge into `dev` is not approval to update `main`.
- Without explicit approval for the specific operation, never change local or remote `dev` or `main`: no commits, merges, cherry-picks, rebases, resets, pushes, force-pushes, or deletions. Fetching and reading them, branching from `origin/dev`, and merging `origin/dev` into a task branch are allowed.
- Pull requests may be prepared for review; never enable auto-merge.

### Implementation and validation

- Read the relevant code and clean-room notes before changing files. Keep changes small; no speculative architecture.
- Add tests for logic that works without hardware (message encoding and decoding, checksums, settings round-trips).
- Check visual UI changes in the running app. Validate Linux windowing and desktop integration separately with native Wayland and X11 sessions. An X11 test under XWayland exercises the X11 code path and must be reported as such.
- Before finishing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Report compile checks separately from runtime and hardware tests for Windows, macOS, Linux Wayland, and Linux X11. Never claim a camera model, firmware, or OS works until it was tested and recorded in `docs/devices.md`.
