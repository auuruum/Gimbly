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

**Test:** `camera::standby::tests` covers post-Resume and late standby
frames, dark live video, and a scene with similar colors. The fixture is
synthetic. To check with the camera, open Discord's preview for Gimbly
Virtual Camera, then use Sleep and Resume in Gimbly. The preview should
switch directly from the Gimbly icon to live video.
