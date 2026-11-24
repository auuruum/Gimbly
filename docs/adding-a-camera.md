# Adding support for another camera

This guide is for a contributor who owns an OBSBOT camera that Gimbly does
not support yet, working alone or with an AI coding agent. Every step works
on your own camera. The [clean-room rules](../AGENTS.md#3-clean-room-implementation)
apply to everything here.

Agents: follow the steps in order, stop at each **Ask the owner** point,
and never skip the clean-room limits below.

## What "supported" means

Any camera gets live video, preview, snapshots and the virtual camera.
Gimbly sends vendor commands (gimbal, zoom, field of view, presets,
sleep/wake, tracking) only to models listed in `SUPPORTED_MODELS` in
[crates/gimbly/src/camera/mod.rs](../crates/gimbly/src/camera/mod.rs).
Each listed model was checked on real hardware. Adding a model means
proving that it understands the same messages, then adding one line.

## Hard limits

- Only black-box work: watch the official app's UI, capture USB traffic of
  your own camera, send test commands to your own camera.
- Never decompile, disassemble, debug or read the files or memory of
  OBSBOT Center, the OBSBOT SDK, firmware or any OBSBOT binary. Never use
  the SDK or its headers and docs. Never read other OBSBOT control
  projects' source code.
- Never commit USB captures, screenshots or official assets. Describe the
  capture in a note instead.

## Steps

### 1. Identify the camera

Build Gimbly (see [CONTRIBUTING.md](../CONTRIBUTING.md#build-and-run)),
close every app that uses the camera, and run:

```sh
cargo run --release -p gimbly -- --probe --formats
```

Each `device:` line shows `vid`, `pid` and `supported_model`. OBSBOT uses
`vid=3564`. Write down the `pid` and the camera's firmware version from
the official app.

### 2. Check whether the existing protocol fits

Gimbly talks to a UVC extension unit with GUID
`{9a1e7291-6843-4683-6d92-39bc7906ee49}` using 60-byte messages, described
in [the vendor message notes](clean-room/2026-11-08-vendor-messages.md).

On a local branch, add your model to `SUPPORTED_MODELS`:

```rust
pub const SUPPORTED_MODELS: &[(u16, &str)] = &[(0xff04, "Tiny 3 Lite"), (0x1234, "Your Model")];
```

Then run the control probe. `status` only reads from the camera:

```sh
cargo run --release -p gimbly -- --probe --controls status
```

- Lines with real values for `vendor zoom`, the status block (`selector 6`
  on Windows, `sleeping and tracking` on Linux and macOS) and `presets`
  mean the camera answers the same channel. Windows also prints
  `vendor extension unit: true`. Go on to step 3.
- `vendor extension unit: false`, `UVC vendor extension unit not found`
  or control errors mean the camera uses a different channel. Gimbly needs
  new research for it: go to step 4 and keep what the probe printed.

**Ask the owner** before sending movement commands. Then try one at a time
and watch the camera:

```sh
cargo run --release -p gimbly -- --probe --controls move turn:0:10:1000 stop
```

### 3. Check each feature

Run the app with the preview and virtual camera open, and try every
feature listed for the Tiny 3 Lite in the [README](../README.md#supported-webcams).
For each one note: works, works differently, or does nothing. Things that
differ often between models:

- zoom range and field of view steps,
- which AI modes exist,
- the standby picture shown while asleep. Gimbly recognises the Tiny 3
  Lite's picture in [standby.rs](../crates/gimbly/src/camera/standby.rs);
  another picture needs its own check.

If a feature behaves differently, capture what the official app sends for
it (step 4) before changing code.

### 4. Capture the official app (only where needed)

Use the research loop in
[the clean-room log](clean-room/README.md): close Gimbly, start a USB
capture (USBPcap or Wireshark on Windows, `usbmon` on Linux), change
exactly one setting in the official app at a time and note the time and
value of each click. Stop the capture and match each click to the UVC
`SET_CUR`/`GET_CUR` transfers it produced. Compare them with the existing
notes in `docs/clean-room/`; for each difference write down the bytes
that changed.

### 5. Record the evidence

- Add a dated note `docs/clean-room/YYYY-MM-DD-<model>-<topic>.md` in the
  format from [the clean-room README](clean-room/README.md): source and
  method, camera model and firmware, official app version, OS, observed /
  inferred / unknown, and the test that checks it.
- Add one row per verified feature to [devices.md](devices.md). List only
  what you ran on real hardware.
- Add the model to the README camera table.

### 6. Implement and test

- Keep the line in `SUPPORTED_MODELS` only if step 2 succeeded and step 3
  found no harmful behavior.
- For each difference, branch on the model in the shared code in
  `crates/gimbly/src/camera/`, not in a platform file. Add a unit test that
  encodes or decodes the model's bytes from your note.
- Run the checks from [CONTRIBUTING.md](../CONTRIBUTING.md#make-a-change).

### 7. Send it

Push a branch and open a pull request with the probe output, the list of
tested features, the OS you tested on, and links to your notes. Say which
operating systems you could not test.
