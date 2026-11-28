# Voice Control and Audio (More tab)

## Source and method

2026-11-28. Claude Code went through every control of the **Voice
Control** and **Audio** sections of the More tab in the visible OBSBOT
Center UI, one action at a time, and checked the UI after each one:

- 18:16-18:19 a first look without a capture (the UAC prompt for USBPcap
  was not answered); a few clicks then were swallowed by a notice and not
  applied.
- 18:40-18:49 every control again with a USB capture (USBPcap, snap length
  512, camera at address 1 on `\\.\USBPcap3`, Gimbly closed).
- 18:59-19:00 a second capture: the sleep screen's microphone switch and
  AddTX2.

The captures stay outside the repository and were decoded with our own
script.

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

Capture started 18:40:22. Actions, about 4 s apart:

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

Second capture, 18:59-19:00: the camera was asleep; the sleep screen's
"MIC Status During Sleep" switch off and on, Resume, Audio Input Vox,
AddTX2, the pairing box closed with its X, Cam Built-in Mic.

## Observed: USB

Voice Control, selector 6 blocks `15 02 key value`:

| key | control | value |
| --- | --- | --- |
| 0 | Hi, Tiny | 0 off, 1 on |
| 1 | Sleep, Tiny | |
| 2 | Track Me | |
| 3 | Unlock Me | |
| 4 | Zoom in Closer | |
| 5 | Zoom out Further | |
| 6 | Position One/Two/Three | |
| `65` | Language | 0 Chinese, 1 English |
| `64` | Zoom Factor | 0 to 100 (`64`) |

- Voice Locator: `23 02 00 v`.
- Zoom Factor: 4.00 sent 100, 1.00 sent 0, one arrow step 1, typed 2.5
  sent 50, 1.234 sent 7, 2 sent 33 (shown as 1.99 after the restart).
  While typing the app also sent the partial value (`2` before `2.5`).
  A refused value resent the old one. The camera zoomed to the factor
  (status offset 4 followed it) without any zoom message from the app.
- Audio Mode: `22 02 00 m`, m 5 Pure, 1 Spatial, 0 Smart Omni,
  2 Directional, 4 Dual-Directional.
- Microphone Status During Sleep: `13 01 v`, also from the sleep screen.
- Disable Microphone: `1c 01 00` turns the microphone off, `1c 01 01` on;
  the camera restarts and the app reopens it (the start-up reads follow).
  "No" sent nothing.
- Audio Input: `42 89` to the camera with u8 3 (Vox) or 0 (built-in).
- AddTX1 / AddTX2: `0e 0c` to destination `13` with 1 or 2, reply
  wanted; the reply echoed the number. End Pairing and the box's X both
  sent `ce 0d` to `13` (reply `01`). Nothing was paired.

Status block (the 60-byte block on selector 6):

- 20: language (0 Chinese, 1 English).
- 21: one bit per command key (`7f` with all on; Track Me off gave `7b`).
- 22: Zoom Factor steps.
- 44: bit 0 Voice Locator (`03` on, `02` off; bit 1 unknown).
- 40: bits 0-2 the input (0 built-in, 3 Vox), bits 3-5 the mode (same
  numbers as written): `08` Spatial, `28` Pure, `0b` Spatial with Vox.
- 16: Microphone Status During Sleep.
- 34: `10` with the microphone on, `00` after it was turned off and the
  camera came back.

## Inferred

- The official app reads these settings from the status block, as
  Gimbly does: after its restart it showed the camera's 1.99, not the 2
  typed before.
- Destination `13` is the Vox receiver built into the camera.

## Unknown

- Bit 1 of offset 44 and the other bits of offset 34.
- What Noise Reduction offers (greyed out without a paired Vox) and how a
  paired transmitter is reported.
- Whether the spoken commands work; none was tried by voice.
- Whether Voice Locator is refused at 1080P 120 (the help text says so).

## Gimbly

- More tab: both sections with the same controls, order, defaults,
  dependencies (Voice Locator only with Hi, Tiny on; Vox hides Audio Mode
  and Disable Microphone and shows AddTX1/AddTX2, a greyed sleep switch
  and Noise Reduction), help texts in our own words, the notice with
  "Don't show again" (`[VoiceControl] NoticeOff`), and the question before
  the microphone switch. The sleep screen's switch is now live.
- Every value is read from the status block.
- Language names are "Chinese" and "English" and the rows keep their
  English labels: Gimbly's fonts have no Chinese glyphs.
- The Zoom Factor is sent once per change, not per keystroke; a typed
  value is rounded down to a step, like the camera.
- At 120 fps and above Audio Mode only offers Smart Omni (help text).
- Gimbly had no way to reopen a camera that went away. After Disable
  Microphone it now waits for the camera to drop, then looks for the
  OBSBOT camera every 3 s and reopens it, for up to 60 s.

## Tests

- `camera::vendor::tests::voice_control_matches_capture` and
  `audio_matches_capture` use the captured bytes and status blocks.
- `app::view_more::tests::typed_voice_zoom_rounds_down_like_the_camera`.
- `app::tests::a_restarting_camera_is_reopened_once_it_went_away`.
- `settings::tests::round_trips_through_ini` covers the notice setting.

## Checked in Gimbly

2026-11-28 19:05-19:21, same camera and OS, OBSBOT Center closed, with a
USB capture:

- `gimbly --probe --virtual-camera --gimbal DIR wake voice:2:off
  voice:2:on voice:locator:off voice:locator:on voice:lang:zh
  voice:lang:en voice:zoom:33 voice:zoom:0 audiomode:0 audiomode:2
  audiomode:1 sleepmic:off sleepmic:on`: no errors; the status read after
  each step showed the new value.
- Running app: Track Me off and on (notice), Voice Locator, Hi, Tiny off
  hides Voice Locator, Language both ways, Zoom Factor typed 2 (shows
  1.99), arrow up (2.02), typed 1, arrow down at 1.00 stays; all three
  help boxes; Audio Mode Directional and Spatial; Microphone Status
  During Sleep both ways, here and on the sleep screen; Vox, AddTX1, End
  Pairing, back to the built-in mic.
- Disable Microphone with the virtual camera on: the first build lost the
  camera for good; after the fix, off and on each restarted the camera
  and Gimbly reopened it by itself, controls live again within the
  minute.
- The captured blocks and messages matched the official app's byte for
  byte (Gimbly uses channel 1 in framed messages, as everywhere).
- Linux and macOS: compiled by CI only, not run.

The camera was left with every voice command on, English, Voice Locator
on, Zoom Factor 1.00, Spatial, microphone on, sleep microphone on.
