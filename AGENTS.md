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
- Keep the MIT license. Do not change it without the owner's approval.
- Icons: SVG or egui-phosphor. Never use Unicode characters or emoji as UI icons.

### Platform support

- Design and implement each feature for Windows, Linux, and macOS in the same change. Include the tests for that feature in the same change.
- Account for native Wayland and native X11 from the start of any Linux work. Gimbly runs as a native Wayland client in a Wayland session and as an X11 client in an X11 session. XWayland is a compatibility path.
- Keep shared behavior in platform-neutral code. Put native camera, windowing, shortcut, and desktop APIs behind shared interfaces.
- On Wayland, use the compositor or desktop portal for operations that require its permission. Register global hotkeys through the XDG Desktop Portal `org.freedesktop.portal.GlobalShortcuts` and let the portal handle consent and compositor bindings. On X11, use native key grabs for global hotkeys. Use the FileChooser portal for native Linux folder selection when the desktop provides it.
- Report features that the OS, compositor, or desktop portal cannot provide with a clear in-app message. Identify any setup or permission the feature needs.

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

## 5. Implementation workflow

### Branches and human approval

- `main` is the only long-lived branch. Start each new task on a separate branch based on the latest `origin/main`. Continue related work on that task branch.
- After fetching, run `git switch --no-track -c <kind>/<short-task-name> origin/main`. Use `feature/`, `fix/`, `docs/`, `refactor/`, or `chore/`. Name the branch after the work, without agent, tool, or model names.
- Within the task, agents may edit, test, commit, and push the task branch. Push explicitly to the task branch.
- Report the changes, validation results, and any checks still needed. Ask the owner whether to merge the task branch into `main`.
- After the owner approves that merge, update local `main` from `origin/main` with a fast-forward, merge the task branch with `git merge --no-ff`, and push `main`. Rerun the checks before pushing if `main` advanced since the task branch was cut.
- Any change to local or remote `main` requires explicit approval for that operation. Fetching `origin/main`, branching from it, or merging it into a task branch is allowed.
- Open a pull request only if the owner asks, and target `main`. Keep auto-merge disabled.

### Implementation and validation

- Read the relevant code and clean-room notes before changing files. Keep changes small; no speculative architecture.
- Add tests for logic that works without hardware (message encoding and decoding, checksums, settings round-trips). Run shared tests on all three OSes and platform-specific tests on the applicable OS.
- Before finishing, run the local checks in [CONTRIBUTING.md](CONTRIBUTING.md#make-a-change). GitHub Actions checks formatting and, on each of Windows, Linux, and macOS, runs Clippy, builds the full workspace including Gimbly and its platform binaries, and runs the workspace tests.
- Check visual UI changes in the running app. For Linux windowing and desktop integration changes, verify the affected behavior in separate native Wayland and native X11 sessions. Report any XWayland run as X11 under XWayland.
- Report compile checks separately from runtime and hardware tests for Windows, macOS, native Wayland, and native X11. Record any target or session you could not verify and the check it still needs. Never claim a camera model, firmware, or OS works until it was tested and recorded in `docs/devices.md`.
