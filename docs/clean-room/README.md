# Clean-room log

Behavior and evidence notes, one file per finding, named
`YYYY-MM-DD-<topic>.md` (for example `2026-11-08-sleep-wake.md`). Rules are in
[AGENTS.md](../../AGENTS.md#3-clean-room-implementation).

Each entry records:

- **Source and method:** how it was observed (USB capture of the owner's own
  camera while using the official app, watching the app's UI, a black-box
  experiment) and the date.
- **Setup:** camera model, firmware version, official app version, OS.
- **Behavior:** what the user does, what goes over the wire (transfer type,
  request, interface/unit/selector, bytes), what the camera answers, and what
  changes on the camera.
- **Observed / inferred / unknown**, kept apart.
- **Test:** the test that checks it.

Write in your own words. Never paste decompiled code, SDK text, official
documentation, screenshots, or other OBSBOT material, and do not commit the
captures themselves.
