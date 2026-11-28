# White Balance

## Source and method

2026-11-28. One USB capture with USBPcap (snap length 512, device address 1
on the camera's root hub `\.\USBPcap2`, descriptors injected), Gimbly
closed, OBSBOT Center the only app using the camera. Claude Code went
through every control of the **White Balance** section on the **Image** tab
of the visible OBSBOT Center UI, one action at a time with a few seconds
between them, and checked the UI after each one. The capture stays outside
the repository and was decoded with our own script.

| time | action |
| --- | --- |
| 12:13:45-57 | the camera was asleep (auto sleep), Resume |
| 12:14:04 | Auto WB on: a click on the color square, up and right of the middle |
| 12:14:15-31 | clicks on the four corners of the square, then the middle |
| 12:14:38-43 | B-A up arrow twice, G-M down arrow |
| 12:14:45-58 | `3.3` and `9` typed into the B-A and G-M fields, double-click and `Ctrl+A` in B-A |
| 12:15:04-19 | marker dragged down-left, then up-right out of the square; mouse wheel over the square |
| 12:15:20-41 | right-clicks on the square, the header and the B-A field; hover on the truncated label and the reset icon; reset icon |
| 12:15:46 | Auto WB off |
| 12:15:54-57 | Temperature dragged to the left and the right end; the camera fell asleep, Resume |
| 12:16:17-41 | Temperature arrows down, down, up; `4321`, `4389`, `1500`, `12000` typed; wheel over the slider |
| 12:16:42-53 | hover on the method label and both options, then R/B |
| 12:17:02-13 | R Gain dragged to both ends, down arrow, `100` typed; `300` typed into B Gain |
| 12:17:20-26 | Temperature, R/B, reset icon (in R/B mode) |
| 12:17:33-56 | Auto WB off, Temperature, Auto WB on, B-A and G-M up arrows, Auto WB off and on |
| 12:18:00-15 | section folded and unfolded; the camera fell asleep, Resume |
| 12:18:22-28 | Auto WB off, R/B |
| 12:18:35-19:01 | OBSBOT Center closed and opened again, Image tab, Temperature |
| 12:19:12 | reset icon |

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (not
  reread).
- Official app: OBSBOT Center `v2.0.17.532` (not reread).
- OS: Windows 10 Pro 19045.
- No AI mode running, preview closed, Auto Exposure on.

## Observed: the UI

- The header shows **White Balance**, a round-arrow reset icon after the
  title and a caret on the right that folds the section. Folded, only the
  header stays.
- **Auto WB** is a switch. While it is on, the section shows a color square
  and, on its right, **White Balance...** (the tooltip gives the full name,
  **White Balance Shift**) with **B-A:** and **G-M:** fields with arrows.
- The square is a color gradient with a small ring marker: violet-blue at
  the top left, red-pink at the top right, cyan-green at the bottom left,
  yellow at the bottom right. Its left edge is B-A -7.00, its right edge
  7.00; its top edge is G-M 7.00, its bottom edge -7.00. A click moves the
  marker there; dragging moves it, and outside the square it stays at the
  edge. Values are shown with two decimals in steps of 0.25.
- The B-A and G-M arrows step by 0.25. Typed text was not taken: the fields
  kept their value after Enter. Their right-click menu is the plain field
  menu (Copy, Select All, Step up, Step down).
- While Auto WB is off, in **Temperature** mode the section shows
  **Temperature** (slider and field `5700 k`) and then **Manual
  Adjustment...** (tooltip **Manual Adjustment Method**) with two options,
  **Tem...** (tooltip **Temperature**) and **R/B ...** (tooltip **R/B
  Gain**). In R/B mode the method row comes first, followed by **R Gain** and
  **B Gain**, each a slider and a field.
- Temperature: 2000 K at the left end, 10000 K at the right; arrows step
  100. Typed `4321` became 4300, `4389` became 4400; `1500` and `12000` were
  refused.
- R Gain and B Gain: 0 to 255, arrows step 1; the field takes no digit that
  would pass 255 (`300` became `30`).
- The mouse wheel over the square or the sliders scrolled the panel, not
  the value. No right-click menus elsewhere.
- Reset icon: Auto WB on and the shift back to 0.00. The temperature, the
  gains and the method stayed.
- Turning Auto WB off and on kept the shift. The shift and the manual
  values survived camera sleep and an OBSBOT Center restart.
- The camera fell asleep twice during the session (auto sleep), not because
  of a White Balance control.

## Observed: USB

All writes are one vendor message to the camera (`02`). Framing as in
[vendor messages](2026-11-08-vendor-messages.md).

- **Write `c2 2a`:** 28 bytes, seven u32:
  1. `0` Auto WB on, `ff` off;
  2. temperature in kelvin (`1644` 5700, `07d0` 2000, `2710` 10000, `10cc`
     4300);
  3. method: 0 Temperature, 1 R/B;
  4. B gain (140 → 3 → 30 while B Gain was edited);
  5. R gain (145 → 0, 255, 254, 1, 100 while R Gain was edited);
  6. B-A shift, 0 to 56 (`23` for 1.75, `00` for -7.00, `38` for 7.00);
  7. G-M shift, the same way (`2f` for 4.75).

  Every write carries all seven values. Many were sent twice in a row about
  20 ms apart, and again about 60 ms apart while dragging.
- **Read `82 2a`:** the reply has the same 28 bytes. OBSBOT Center read it
  on start and wake, and before and after each change of the method.
- Reset: one `c2 2a` with the first value 0 and both shift values 28.
- The app also read the UVC processing unit white balance temperature
  (`GET_CUR` selector 10, `44 16` for 5700) and its auto control (selector
  11) now and then, but never wrote them.

## Inferred

- The shift unit is a quarter: 28 is 0.00 and each step is 0.25, so the
  range is -7 to 7.
- The vendor message is the only control the official app uses, so it is
  the same on every OS.

## Unknown

- Whether other models take the same message or ranges.
- Why most writes are sent twice; Gimbly sends each once.
- Whether the camera accepts values outside the ranges the UI offers;
  Gimbly clamps them.

## Tests

- `camera::vendor::tests::white_balance_matches_capture` checks a captured
  `c2 2a` write byte for byte with its CRC, the data of an Auto WB write,
  clamping, the captured `82 2a` reply (with the bytes the camera leaves
  after the data) and bad replies.
- `app::view_white_balance::tests` check the shift text and typed shift,
  typed temperatures (`4321` to 4300, `1500` refused) and the pad's corner
  colors.

## Checked in Gimbly

2026-11-28, same camera and OS, OBSBOT Center closed:

- `gimbly --probe --gimbal DIR wake wb:3000 wb:rb:200:40 wb:auto:40:10
  wb:auto:28:28` sent every step without an error. The first status read
  back what OBSBOT Center had left (4400 K, R/B 100 and 30). The snapshots
  turned blue at 3000 K, orange with R 200 and B 40, and warmer with the
  shift; `wb:rb:60:220` read back in the next process and turned the
  picture blue.
- `gimbly --probe --virtual-camera --gimbal DIR wake wb:2000 … wb:10000
  wb:auto:28:28` saved a frame after every step with the virtual camera
  running.
- In the running app (Image tab): read the camera's state; R Gain typed
  `150`, B Gain arrow; Temp; Temperature typed `4321` (4300) and `1500`
  (refused), slider dragged; Auto WB on; a click on the square, drag out
  past the bottom-left corner (stayed at -7.00), B-A arrow, G-M typed `2.6`
  (2.50); restart read the shift back; reset (Auto WB on, 0.00); fold. A
  quick click on the square first did nothing because press and release
  came in one frame; fixed before the commit.
- The camera was left in Auto WB, 5700 K, R/B 145 and 140, no shift.
