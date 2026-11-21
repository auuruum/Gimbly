# Group Mode and Auto Zoom captures

## Source and method

2026-11-21. Two USB captures with USBPcap on `\\.\USBPcap4`, snap length
512, descriptors injected; only the camera (address 1) appeared. Gimbly was
closed. Claude Code clicked the visible OBSBOT Center UI on the owner's PC,
waiting 6–8 s between clicks and checking the button highlight after each.
We analysed both captures with our own scripts. The files stay outside the
repository.

1. **Dark room, nobody in view:** Group Mode twice, Single Mode twice (the
   camera slept through the first attempt; it was woken with Resume), then
   Auto Zoom Full Body, Upper Body, Close-up, Headless, Lower Body, Off.
2. **Owner sitting in front of the camera with the light on:** Single Mode,
   Group Mode (switch), Group Mode (off), Group Mode (on from off), Single
   Mode (switch), Single Mode (off).

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1`.
- Official app: OBSBOT Center `v2.0.15.186`.
- OS: Windows 10 Pro 19045.
- Zone Tracking and Sound Source Assisted Tracking were on, Tracking Speed
  Standard, throughout.

## Observed

- Every click produced only SET_CUR writes of 60 bytes to extension unit 2,
  **selector 6**, starting with `16 02`, followed by two bytes; the rest was
  zero.
- **Human Tracking buttons:** clicking a mode that was not highlighted sent
  two writes about 1 ms apart: `16 02 02 00`, then `16 02 0e 00` for
  Single Mode or `16 02 0e 01` for Group Mode. This happened both when
  tracking was off and when switching from the other mode. Clicking the
  highlighted mode sent `16 02 00 00`.
- **Without a person in view** (capture 1) neither button stayed
  highlighted, and status offset 24 stayed `0`, although the app sent the
  same on writes. The app sent the on writes again on the next click.
- **With the owner in view** (capture 2) offset 24 of the selector 6
  status block read `2` within about 2.5 s of the on writes, in **both**
  Single and Group Mode, and `0` after the off write (one read showed `6`
  for about 0.5 s while turning off). Switching between modes did not
  change any byte of the status block.
- **Auto Zoom:** each button sent one write `16 02 0f NN`:

  | Button | `NN` |
  | --- | --- |
  | Off | `00` |
  | Full Body | `01` |
  | Upper Body | `02` |
  | Close-up | `03` |
  | Headless | `05` |
  | Lower Body | `06` |

  Offset 42 of the status block read the same value 0.7–2 s after each
  write. Auto Zoom changed with tracking off.
- In Group Mode the app greys out Close-up, Headless and Lower Body.
- After each tracking change the app exchanged the `82 ad` / `c2 ad`
  records already described in the
  [Human Tracking capture](2026-11-21-human-tracking-capture.md); the
  records were the same for both modes.
- During sleep transitions offset 43 read `1` for one read and offset 9
  changed `1` → `3`; these did not follow any click.

## Inferred

- Selector 6 command `0x16` carries an AI tracking sub-command in byte 2
  and its value in byte 3: `02` starts tracking, `00` stops it, `0e` picks
  the Human Tracking mode (`0` Single, `1` Group) and `0f` picks the Auto
  Zoom framing. This replaces the earlier guess that `0x0e` was a set of
  flags.
- Tracking engages only while the camera detects a person. That explains
  the unreliable "on" in the earlier black-box tests in a dark room.
- The status block reports whether tracking runs but not the mode, so a
  host can only know the mode it set itself.
- Value `4` is skipped by the app; it may be a framing this model does not
  offer.

## Black-box check with Gimbly (same day, owner in view)

Gimbly with the new buttons: Group Mode, Auto Zoom Upper Body, Single
Mode, Close-up, Group Mode, Auto Zoom Off, Group Mode (off). After every
click the button stayed highlighted past the 2.5 s status read, so the
camera's status agreed with each write.

- **Observed:** switching to Group Mode while Close-up was selected left
  offset 42 at `3`; the camera did not reset the framing.

## Unknown

- Where, if anywhere, the camera reports Single vs Group Mode.
- What value `4` and the transient tracking status `6` mean.
- Whether Close-up, Headless and Lower Body have any effect in Group Mode.
- Behavior on other models and firmware versions.

## Test

`camera::vendor::tests::human_tracking_matches_capture` compares the Single,
Group and off writes with the captured blocks.
`camera::vendor::tests::auto_zoom_matches_capture` compares all six Auto
Zoom writes with the captured blocks, decodes offset 42 for each value, and
rejects value `4`, short blocks and foreign blocks.

Hardware: Windows 10 with the Tiny 3 Lite, in the running app as above.
Still to check: a joystick drag with preview and virtual camera running
while the buttons are used; macOS, native Wayland and native X11.
