# Sleep and wake

## Source and method

2026-11-08, USBPcap on `\\.\USBPcap3` while clicking the power control
and Resume in OBSBOT Center. Black-box verification through Gimbly.
OBSBOT Tiny 3 Lite (`3564:ff04`, USB descriptor revision `0x0510`),
Windows 10 Pro 19045. The firmware release and OBSBOT Center version were
not recorded.

## Observed

- Sleep and wake both use address `02`, command `c2 a0`, flags `25`,
  through extension unit 2, selector 2, interface 0. The little-endian
  u32 payload is 1 for sleep and 0 for wake. Resume sent the same wake
  command as the power control.
- On selector 6, GET_CUR returned 60 bytes starting with `2e 00` (46).
  The byte at offset 2 changed from 0 to 1 when the camera slept and
  returned to 0 on wake. The camera was still reachable for reads while
  sleeping, and Gimbly could take a placeholder snapshot and resume
  the live view.
- After a sleep command, closing Gimbly and opening a new capture
  session produced live frames and an awake status without an explicit
  wake command. The new session recovered the two stored presets.

## Inferred

Gimbly reads offset 2 as the sleep flag on the Tiny 3 Lite.

## Unknown

The meaning of the other changes in the selector 6 block.

## Test

`camera::vendor::tests::view_commands_match_capture` checks sleep
encoding and the status decoder against captured bytes. Hardware test
with `gimbly --probe --gimbal DIR sleep wake`.

## Virtual camera during wake

**Source and method (2026-11-08):** User screenshots of Gimbly and the
Discord preview with Gimbly Virtual Camera selected, in the Tiny 3 Lite
sleep flow. The report did not record firmware, capture format, or an
OBSBOT Center version. No OBSBOT software was inspected.

**Observed:** Discord showed the Gimbly icon during sleep. After Resume,
Gimbly's sleep overlay cleared and Discord briefly showed a red OBSBOT
badge with a white ring on a flat, dark background. The badge is centered
horizontally and sits a little above the middle of the picture.

**Inferred:** The camera's standby image can outlast the change in the
control state. Use a few luma and chroma samples on the raw Tiny 3 Lite
frame to identify the standby picture even when the sleep flag is clear.
Keep substituting Gimbly's frame until live video arrives.

**Unknown:** Whether the standby picture differs on other firmware or in
capture formats not shown in the report.

**Superseded 2026-11-28:** the badge detector was replaced by the generic
check in `camera::resume`; see the 2026-11-28 Device Sleep note.

**Test (removed):** `camera::standby::tests` covered post-Resume and late standby
frames, dark live video, and a scene with similar colors. The fixture is
synthetic. To check with the camera, open Discord's preview for Gimbly
Virtual Camera, then use Sleep and Resume in Gimbly. The preview should
switch directly from the Gimbly icon to live video.

**Hardware check (2026-11-08):** Tiny 3 Lite, firmware release unknown,
Windows 10 build 19045, NV12 1920x1080 at 30 fps. A local FFmpeg
DirectShow client read Gimbly Virtual Camera while a probe sent the same
sleep and wake commands as the panel. No video was saved. The build
before the fix (`6657ed8`) sent 60 OBSBOT standby frames after Resume.
With the fix (`5d47a9d`), the client received 59 Gimbly placeholder
frames before live video, with no OBSBOT standby frames detected.
Three additional cycles checked at the shared-memory output also
showed Gimbly frames after the sleep flag cleared and then live video.
Discord itself was not verified with the fix.

## Sleep status flicker

**Source and method (2026-11-08):** Gimbly capture and control trace on a
Tiny 3 Lite (`3564:ff04`, firmware release unknown), Windows 10 build
19045, NV12 1920x1080 at 30 fps. The probe used the same sleep and wake
commands as the panel, with the preview frame consumer and virtual
camera active. FFmpeg read the virtual camera through DirectShow and
logged frame checksums without recording video. OBSBOT Center was not
used for this test.

**Observed:** Before the fix, all three Sleep commands succeeded and the
camera began sending its standby picture. Selector 6 still reported
awake at the first two polls (about one and two seconds after Sleep) and
reported asleep at the third. Gimbly cleared its sleeping state between
the first and third polls without sending a wake command. The raw
standby picture persisted throughout that interval.

**Inferred:** A successful write and the sleep status flag can disagree
during a transition. Gimbly keeps the requested state until the device
confirms it, or until five seconds pass and a valid poll can correct it.
An explicit Resume can replace a pending Sleep immediately.

**Unknown:** Why the camera reports awake during this interval, and
whether other models or firmware releases behave the same way.

**Test:** `camera::sleep::tests` covers the observed poll sequence, a
Resume before Sleep is confirmed, and a command that never confirms.
In two three-cycle runs with the fix, the displayed state stayed asleep
until Resume despite the same delayed readback. In the DirectShow run,
the client received a continuous placeholder through each Sleep and
wake transition, then live frames. The probe also exercised joystick
commands with both frame consumers active. Direct interaction with the
panel and Video Preview was not verified.

## Previous preview frame on Resume

**Source and method (2026-11-08):** User report with Video Preview and
Gimbly Virtual Camera active, inspection of Gimbly's preview path, and
an egui draw test. Follow-up hardware probe on Tiny 3 Lite (`3564:ff04`,
firmware release unknown), Windows 10 build 19045, NV12 1920x1080 at
30 fps. OBSBOT Center was not used.

**Observed:** The user removed their hand during Sleep and briefly saw
the frame with their hand again on Resume. In Gimbly, the preview
texture survived Sleep, and Resume made it eligible to be drawn before
the first new frame arrived. The draw test reproduced that sequence
with a synthetic texture.

**Inferred:** The preview can show a cached image even when the camera
does not resend it. Each sleep or wake transition now invalidates queued
and in-flight preview frames. The window draws the Gimbly placeholder
until a texture from the current video epoch is available.

**Unknown:** Whether the reported flash was also visible in a separate
virtual camera client.

**Test:** `camera::tests::sleep_and_resume_discard_queued_and_in_flight_preview_frames`
and `preview_window::tests::resume_draws_the_placeholder_until_a_new_texture_arrives`.
In three hardware probe cycles the preview consumer received no frame
while the session reported asleep and resumed receiving frames after
wake. The DirectShow client received a continuous Gimbly placeholder
through each transition. None of its first 30 frames after the
placeholder matched the plane checksums of the last 30 frames before
Sleep. The probe also exercised joystick commands with both consumers
active. Only checksums were saved. The native window was not tested by
clicking its controls.
