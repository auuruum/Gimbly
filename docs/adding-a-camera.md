# Adding support for another camera

This guide is for a contributor who owns an OBSBOT camera that Gimbly does
not support yet, working alone or with an AI coding agent. Every step works
on your own camera. The [clean-room rules](../AGENTS.md#3-clean-room-implementation)
apply to everything here.

Agents: follow the steps in order, stop at each **Ask the owner** point,
and never skip the clean-room limits below.

## How support works

- Any camera gets live video, preview, snapshots and the virtual camera.
- Vendor features (gimbal, zoom, presets, sleep, tracking and the rest)
  are switched on per model. Each model is one entry in `MODELS` in
  [models.rs](../crates/gimbly/src/camera/models.rs) with the list of
  features checked on it:

  ```rust
  Model {
      product_id: 0xff04,
      name: "Tiny 3 Lite",
      features: &[Feature::Gimbal, Feature::Zoom, /* ... */],
      standby_picture: true,
  }
  ```

- The app shows only listed features. Unlisted ones stay disabled with a
  "not verified on this model yet" tip. A model can start with two or
  three features and grow later.
- Every message is described once in [protocol.md](protocol.md). Models
  share it; a difference is written there under the feature and handled
  in the shared code.

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

Each `device:` line shows `vid`, `pid` and `model`. OBSBOT uses
`vid=3564`. `model=None` means Gimbly does not know the camera yet. Write
down the `pid` and the firmware version the official app shows.

### 2. Read what the camera answers

```sh
cargo run --release -p gimbly -- --probe --controls report
```

`report` only reads. It prints one line per feature with what the camera
answered, for example:

```text
status block (selector 6): recognized
Gimbal control: Ok(Some((-30, 8500)))
Zoom: Ok(1.0)
Tracking Speed: Ok(Standard)
Hand Tracking: Err("No matching camera reply for 84 20")
```

- `vendor extension unit: false` (Windows), `UVC vendor extension unit
  not found` (Linux, macOS) or `status block: not recognized`: the camera
  uses a different channel. Stop here and open an issue with the output;
  this needs new research.
- `Ok(...)` or `Some(...)` with sensible values: the camera reads that
  feature like the Tiny 3 Lite. Wake the camera first; a sleeping camera
  leaves some reads unanswered.
- `Err(...)`: that read differs or the feature is missing. Check it in
  step 4 before listing it.

### 3. Add the model

Add an entry to `MODELS` with the `pid`, the name, and only
`Feature::Gimbal` at first. Keep `standby_picture: false` unless the
camera shows the same standby picture as the Tiny 3 Lite (see
[standby.rs](../crates/gimbly/src/camera/standby.rs)).

**Ask the owner** before the first command that moves the camera. Then run
the app with the preview and virtual camera open and try the joystick.

### 4. Check one feature at a time

For each feature in [protocol.md](protocol.md):

1. Add it to the model's `features`.
2. Use it in the app and watch the camera. Compare with the official app.
3. Keep it if it behaves the same. Remove it if it does nothing or
   something else, and go to step 5 for it.

Frames must keep flowing in the preview and the virtual camera while you
use the gimbal and the zoom.

### 5. Capture where the camera differs

Only for features that differ. Close Gimbly, start a USB capture
(USBPcap or Wireshark on Windows, `usbmon` on Linux), change exactly one
setting in the official app at a time, and note the time and value of
each click. Stop the capture and match each click to the `SET_CUR` and
`GET_CUR` transfers it produced. Compare them with the feature's table in
[protocol.md](protocol.md) and write down the bytes that differ.

To support the difference, add what the model sends to `protocol.md`
under that feature, and pick the bytes by model in
[vendor.rs](../crates/gimbly/src/camera/vendor.rs) or
[control.rs](../crates/gimbly/src/camera/control.rs). Add a unit test
with the bytes from your capture.

### 6. Record the evidence

- Add a dated note `docs/clean-room/YYYY-MM-DD-<model>-<topic>.md` in the
  format from [the clean-room README](clean-room/README.md): source and
  method, camera model and firmware, official app version, OS, observed /
  inferred / unknown, and the test that checks it. Paste the `report`
  output there.
- Add one row per checked feature to [devices.md](devices.md). A test
  fails if a model in `MODELS` has no row there.
- Add the model to the README camera table.

### 7. Test and send it

Run the checks from [CONTRIBUTING.md](../CONTRIBUTING.md#make-a-change),
push a branch and open a pull request. List the checked features, the OS
you tested on, and the OS you could not test.
