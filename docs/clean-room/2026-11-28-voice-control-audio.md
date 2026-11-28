# Voice Control and Audio (More tab)

Status: **UI observed, USB not captured yet.** Nothing is implemented in
Gimbly from this note yet; the messages are unknown.

## Source and method

2026-11-28, 18:16-18:19. Claude Code looked at the **Voice Control** and
**Audio** sections of the More tab in the visible OBSBOT Center UI: help
icons, drop-downs, hover and right-click. The USB capture could not be
started: the UAC prompt for USBPcap was not answered three times (18:12,
18:14, 18:18), so no click below has its USB traffic recorded.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (not
  reread). The camera is on root hub `\\.\USBPcap3` this session.
- Official app: OBSBOT Center `v2.0.17.532` (not reread).
- OS: Windows 10 Pro 19045. Preview closed, Gimbly closed.

## Observed: Voice Control

A foldable section with a `?` after the title. The `?` opens a Tips box
with an animation of the camera waking to the spoken "Hi, Tiny" and a note
that commands for presets do nothing until presets are stored.

Rows, top to bottom, with the values found:

| control | kind | value found |
| --- | --- | --- |
| Language | drop-down: Chinese (中文), English | English |
| Hi, Tiny | switch | on |
| Voice Locator | check box under Hi, Tiny, with `?` | ticked |
| Sleep, Tiny | switch | on |
| Track Me | switch | on |
| Unlock Me | switch | on |
| Zoom in Closer | switch | on |
| Zoom out Further | switch | on |
| Zoom Factor | slider and field with arrows | `1.00 x` |
| Position One/Two/Three | switch | on |

- The Voice Locator `?` opens Tips: with Voice Locator on, "Hi, Tiny" also
  wakes the camera from sleep and turns it toward the voice; only in
  landscape, not at 1080P 120.
- Hi, Tiny off hides the Voice Locator row; on shows it again.
- Turning Hi, Tiny on opened an "Activation Success Notice" in the middle
  of the screen: it advises turning the command off where background
  noise is loud or everyday speech sounds like the command phrases, with a
  "No more prompts" box (left unticked) and "Got it". While it was open,
  clicks on the panel did nothing; so three clicks on Voice Locator and an
  off/on of Hi, Tiny right after it were not applied.
- No hover text, no right-click menus.

## Observed: Audio

A foldable section, no `?` on the header.

| control | kind | value found |
| --- | --- | --- |
| Audio Input | drop-down: Cam Built-in Mic, Vox Wireless Mic | Cam Built-in Mic |
| Audio Mode | drop-down with `?` | Spatial Audio Mode |
| Microphone Status During Sleep | switch | on |
| Disable Microphone | switch | off |

Audio Mode choices, in order: Pure Audio Mode, Spatial Audio Mode, Smart
Omni Mode, Directional Mode, Dual-Directional Mode. Its `?` opens a box
that describes each in a sentence (in our words):

1. Pure: left and right channels unprocessed.
2. Spatial: left and right with stronger stereo placement, AI noise
   reduction and automatic gain.
3. Smart Omni: picks up all directions and balances voices and
   surroundings, for meetings with several people.
4. Directional: the front, suppressing voices and noise around it.
5. Dual-Directional: front and back, filtering the sides, for interviews.

The box ends with: at 1080P 120 only Smart Omni is offered.

No hover text, no right-click menus.

## Inferred

- Voice commands are recognised by the camera, so each switch is most
  likely a camera setting sent over USB; the Language drop-down too.
- "Vox Wireless Mic" refers to a separate OBSBOT wireless microphone; the
  owner has none, so that choice cannot be checked here.

## Unknown

- Every USB message: needs the capture (see the research loop).
- Whether the camera reports any of these values in the status block.
- Whether Sleep, Track Me and the others also show the notice.
- Zoom Factor range and steps.
- What Disable Microphone and Microphone Status During Sleep do to the
  USB audio stream.

## Next step

Rerun with the USB capture: every switch off and on, both languages,
all five audio modes, Zoom Factor at both ends, typed and stepped, both
Audio Input choices, Voice Locator with Hi, Tiny on and off, and an app
restart. Then implement in Gimbly from that capture.

## Tests

None yet.
