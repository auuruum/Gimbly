# Tracking Speed capture

## Source and method

2026-11-21. Captured USB traffic with USBPcap on `\\.\USBPcap4`, all
devices, snap length 512, descriptors injected. Only the camera (address 1)
appeared in the capture. Gimbly was closed. Claude Code clicked the
**AI Tracking → Tracking Speed** buttons in OBSBOT Center on the owner's PC,
waiting about 8 s between clicks, in this order:

1. Slow, Fast, Standard.
2. Fast, Slow. The camera went to sleep, so the Standard click hit the
   sleep overlay. After Resume, Fast and then Standard.

Human Tracking was off throughout, and no other setting changed. We
analysed the capture with our own script. The capture file stays outside
the repository.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (as in
  the [Human Tracking capture](2026-11-21-human-tracking-capture.md)).
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.

## Observed

- Every click produced exactly two SET_CUR writes on extension unit 2,
  selector 2, about 22 ms apart. Both were messages from the host (`0a`) to
  the gimbal (`04`) with command `44 54`, flags `0x25` and 12 data bytes
  forming three little-endian `u32` values:
  - first write: `0`, `4`, `0`. This was identical for every click;
  - second write: `0`, `5`, speed value.
- Speed values in the second write, seven clicks in all:

  | Button | value |
  | --- | --- |
  | Slow | `0` |
  | Standard | `2` |
  | Fast | `3` |

- The app polls command `84 54` (flags `0x21`) about every 2 s with 8 data
  bytes, `u32 0` and `u32 key`. The camera answers with the same sequence
  number, flags `0x29`, and a 4-byte `u32`. For key `5`, the answers were
  `2` before the first click, then `0` after Slow, `3` after Fast and `2`
  after Standard. Each answer matched the button selected at that moment.
- The click produced no selector 6 write. The [Human Tracking](2026-11-21-human-tracking-capture.md)
  toggle uses selector 6 instead.
- Black-box check with Gimbly (`gimbly --probe --gimbal DIR fast`, then
  `slow` in a new process) on the same camera:
  - Gimbly read `Standard` at start and wrote Fast.
  - The next process read `Fast` and wrote Slow.
  - OBSBOT Center, opened afterwards, showed Slow selected.
  - The setting was set back to Standard in OBSBOT Center.

## Inferred

- `44 54` writes and `84 54` reads a numbered gimbal setting. Key `5` holds
  the tracking speed, and the setting survives the host app closing.
- The three buttons map to `0`, `2` and `3`. No button uses value `1`. It
  might be a level this model's app does not offer.

## Unknown

- What key `4` controls, and why the app writes `0` to it before every speed
  change. Gimbly sends the same write to match the app.
- Whether value `1` is valid, and how other models or firmware versions use
  key `5`.
- Whether the camera's tracking movement changes visibly with each value.
  Tracking was off during the capture.

## Test

`camera::vendor::tests::tracking_speed_matches_capture` encodes all three
speeds and the read request, and compares them with the captured blocks. It
also decodes the captured reply and rejects value `1` and short replies.

Hardware: Windows 10 with the Tiny 3 Lite, through the probe readback above
and the Tracking Speed buttons in the running app. Still to check:

- visible tracking movement at each speed;
- a joystick drag with preview and virtual camera running;
- macOS, native Wayland and native X11.
