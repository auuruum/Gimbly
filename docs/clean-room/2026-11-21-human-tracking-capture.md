# Human Tracking on/off capture

## Source and method

2026-11-21. USB capture with USBPcap (`\\.\USBPcap4`, device address 1 only,
descriptors injected) while Claude Code clicked the visible OBSBOT Center UI
on the owner's PC: tracking off for about 5 s, **Human Tracking → Single
Mode** on for about 5 s, then the same button again to turn it off, followed
by about 5 s more. The button highlight confirmed each state. The camera was
woken from sleep before the capture started, and no other setting was
changed during it. We analysed the capture with our own scripts. The capture
file is kept outside the repository.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, model `OAB-2305-CW`,
  firmware `6.5.11.1` (from the app's device list).
- Official app: OBSBOT Center `v2.0.15.186`, Pro Mode.
- OS: Windows 10 Pro 19045.
- Other AI Tracking options shown in the app during the capture: Auto Zoom
  Off, Zone Tracking on, Sound Source Assisted Tracking on, Tracking Speed
  Standard.

## Observed

- All traffic went to extension unit 2 on interface 0. As in the
  [2026-11-08 vendor message log](2026-11-08-vendor-messages.md), the CDC
  serial interface received only a 12-byte bulk OUT write every 500 ms
  during the whole capture, with no visible change at either click.
- **Turning on (t = 14.753 s):** two SET_CUR requests to **selector 6**, each
  60 bytes, about 1 ms apart. The first started with `16 02 02 00`, the
  second with `16 02 0e 00`; the remaining bytes were zero.
- **Turning off (t = 20.576 s):** one SET_CUR to selector 6 starting with
  `16 02 00 00`, with zeros after it.
- **Status:** in the 60-byte selector 6 GET_CUR block (the same block that
  holds the sleep flag at offset 2, see
  [sleep/wake](2026-11-08-sleep-wake.md)), **offset 24** was `00` in every
  read before the on click, `02` in each read from 16.775 s to 19.984 s,
  and `00` again from 20.897 s. The rest of the block did not change.
- After each click the app also exchanged messages on selector 2 that use
  flag bit `0x40`, which earlier captures did not contain. Both checksums
  follow the existing rules, and all six messages pass them:

  | t (s) | flags | src→dst | command | data |
  | --- | --- | --- | --- | --- |
  | 16.862 | `41` | `0a→02` | `82 ad` | `0a 01 01` |
  | 16.955 | `49` (reply) | `02→0a` | `82 ad` | `0a 06 08 01 12 02 20 01` |
  | 16.980 | `45` | `0a→02` | `c2 ad` | `08 01 12 04 08 01 20 01` |
  | 20.987 | `41` | `0a→02` | `82 ad` | `0a 01 01` |
  | 21.072 | `49` (reply) | `02→0a` | `82 ad` | `0a 08 08 01 12 04 08 01 20 01` |
  | 21.094 | `45` | `0a→02` | `c2 ad` | `08 01 12 02 20 01` |

  Messages with flag `0x40` carry a data length and data checksum even
  though bit `0x20` is clear. No reply to `c2 ad` was read.
- The `82 ad` reply at 16.955 s still contained the "off" form. The status
  byte at offset 24 had already read `02` at 16.775 s.

## Inferred

- Selector 6 command `0x16` turns Human Tracking on and off. It is
  laid out like the field-of-view command on the same selector (`04 01 xx`):
  command byte, data length (2), then a little-endian value. `0x0000` turns
  tracking off.
- Offset 24 of the selector 6 status block shows the tracking state:
  `0` = off, `2` = Human Tracking in Single Mode. The `02` matches the value
  in the first on write.
- The second on value, `0x0e`, has bits 1, 2 and 3 set. It may add flags for
  the other options that were on (Zone Tracking and Sound Source Assisted
  Tracking). This guess is unconfirmed.
- Flag `0x40` marks data in a second encoding. The `82 ad` / `c2 ad` data is
  consistent with protobuf wire format (`08 01` = field 1 varint 1,
  `12 nn` = field 2 with length `nn`, `20 01` = field 4 varint 1). `82 ad`
  reads a record and `c2 ad` writes it: the commands differ only in bit
  `0x40`, as in `04 39`/`44 39`/`c4 39`. In the record, field 2 → field 1
  was 1 after tracking was on and missing (0) after it was off. The
  app appears to read the record and write it back with that field updated.
  The camera's state changed before that exchange, so this record looks like
  a stored setting and not the toggle itself.

## Unknown

- Whether the selector 6 command alone starts tracking. A black-box write of
  `16 02 02 00` from Gimbly, checked with status offset 24 and by watching
  the camera follow a person, would settle it.
- The meaning of the second on value (`0x0e`) and whether it changes with
  Zone Tracking, Sound Source Assisted Tracking, or Group Mode.
- The status value for Group Mode and other AI modes.
- The purpose of the `82 ad`/`c2 ad` record and of its fields 1 and 4.
- Behavior on other models and firmware versions.

## Test

There is no Gimbly code for this yet. The implementation should add unit
tests that compare the encoded selector 6 on/off blocks with the bytes above.
It should also decode status offset 24 from the captured off/on blocks and
decode the `0x40`-flagged messages, including their data. The hardware test
should confirm the state after enabling, disabling, and reconnecting while
preview and the virtual camera are running.
