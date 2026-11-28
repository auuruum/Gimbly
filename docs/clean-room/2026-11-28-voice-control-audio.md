# Voice Control and Audio (More tab)

Status: **UI observed; a capture exists but is not decoded yet.** Nothing is implemented in
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

## Observed: second pass, 18:40-18:49, with a USB capture running

Capture on `\\.\USBPcap3`, snap length 512, started 18:40:22. It could
not be stopped or read yet (it runs elevated; two later UAC prompts were
not answered), so the USB side is still unknown. Actions, about 4 s apart:

| time | action |
| --- | --- |
| 18:40:3x | Sleep, Tiny off, on (notice, Got it) |
| 18:40:4x-18:41:3x | Track Me, Unlock Me, Zoom in Closer, Zoom out Further, Position One/Two/Three, Hi, Tiny: each off, on, Got it |
| 18:41:4x | Voice Locator unticked; Hi, Tiny off, on (notice); Voice Locator ticked |
| 18:42:0x-1x | Language 中文, then English |
| 18:42:3x-18:43:2x | Zoom Factor dragged to 4.00 and 1.00, up arrow, down arrow twice, up; typed `2.5`, `5`, `0.5`, `1.234`, `1` |
| 18:43:3x-18:44:0x | Audio Mode Pure, Smart Omni, Directional, Dual-Directional, Spatial |
| 18:44:1x-2x | Microphone Status During Sleep off, on |
| 18:44:3x-18:46:0x | Disable Microphone on, No; on, OK (camera restarts); off, OK (restarts again) |
| 18:46:1x-5x | Audio Input Vox Wireless Mic, AddTX1, End Pairing, Cam Built-in Mic |
| 18:47:0x-2x | Track Me off, Microphone Status During Sleep off, Zoom Factor `2`, Pure |
| 18:47:32-18:48:0x | OBSBOT Center closed and started again, More tab opened |
| 18:48:3x-5x | Track Me on (notice), Microphone Status During Sleep on, Zoom Factor `1`, Spatial |

What the UI showed:

- Every one of the seven command switches opens the same notice when
  turned on; none when turned off. Voice Locator shows no notice.
- Voice Locator stays unticked across Hi, Tiny off and on.
- With 中文 the row labels become the Chinese command phrases (嗨, Tiny;
  Tiny, 休眠; 开始跟踪; 取消跟踪; 拉近; 拉远; 预置位一/二/三). The switches
  keep their state. With English they are the English labels above.
- Zoom Factor: 1.00 to 4.00, two decimals; arrows step and stop at 1.00;
  `2.5` taken, `5` and `0.5` refused (old value stays), `1.234` gave 1.23.
  Same behaviour as the Gesture Control factor.
- Disable Microphone on asks "the device will restart after turning off
  the microphone"; No keeps it off. OK: the app shows "Device not found",
  the camera comes back after about 20-30 s with the switch on, and
  Windows lists only the camera, no OBSBOT audio device. Off asks the same
  for turning on and restarts again; the audio device came back.
- Audio Input Vox Wireless Mic replaces the rows below with AddTX1 and
  AddTX2 buttons, a greyed Microphone Status During Sleep (off) and a
  greyed Noise Reduction drop-down (Off); Audio Mode and Disable
  Microphone are hidden. AddTX1 opens a "Pairing Prompt" (hold the
  transmitter's button 6 s until it blinks green) with End Pairing.
- After the restart every value was shown as set: Track Me off,
  Microphone Status During Sleep off, Pure, and the voice Zoom Factor as
  **1.99** (typed 2), which points to a value read back from the camera
  as a float.
- The Gesture Control section showed Dynamic Zoom on, which it was not
  on 13:11 earlier today; not touched in this session (unknown cause).

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
