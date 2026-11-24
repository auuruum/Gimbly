# Sound Source Assisted Tracking

## Source and method

2026-11-24. USB capture with USBPcap (snap length 512, all devices on the
camera's root hub `\\.\USBPcap3`, the camera at address 1), Gimbly closed,
OBSBOT Center the only app using the camera. Claude Code clicked the
"Sound Source Assisted Tracking" switch in the AI Tracking section of the
visible OBSBOT Center UI, 8 s apart, and checked the switch after each
click. The capture stays outside the repository and was decoded with our own
script.

| time | click |
| --- | --- |
| 46:39 | switch off (it was on) |
| 46:47 | switch on |

An earlier pair of clicks at 43:23 / 43:31 was captured on the wrong root
hub and is not used.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- Human Tracking off, Auto Zoom Off, Zone Tracking off.

## Observed

- **Off:** one SET_CUR on extension unit selector 6: `25 01 00` (rest of
  the 60 bytes zero). Status offset 46 changed `01` → `00` within 0.6 s.
- **On:** selector 6 `25 01 01`. Offset 46 changed `00` → `01`.
- No other write and no vendor message on selector 2 came with either
  click; the periodic reads around them were unchanged.
- The command has the same shape as Voice Tracking Range (`24 01 xx`, see
  [AI mode](2026-11-22-ai-mode.md)), and offset 46 bit 1 is that range.

## Inferred

- Offset 46 bit 0 is the switch state, also when Voice Tracking is not
  running. This matches the earlier sessions where it stayed `1` while the
  switch was on.

## Unknown

- What the switch changes in tracking behavior. Not tested with sound.
- Whether the camera keeps the setting after a power cycle.
- Offset 46 values above `03`; Gimbly treats them as unknown.

## Tests

- `camera::vendor::tests::ai_mode_matches_capture` checks the two writes
  and reading bit 0 of offset 46.

## Checked in Gimbly

2026-11-24, same camera and OS: the switch in the running Gimbly turned
off and on and kept its state after the 1 s status poll. After Gimbly
turned it off and closed, OBSBOT Center opened with the switch off.
