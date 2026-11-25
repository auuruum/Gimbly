# High-fps Recording

## Source and method

2026-11-25. Claude Code drove the **Console → High-fps Recording** block of
OBSBOT Center on the owner's PC with left clicks, right clicks and mouse
hovers, and opened the related **Software Settings → Output** fields. Each
recording the app produced was inspected with `ffprobe` (container, codec,
size, frame rate, per-packet timestamps, metadata). During the session a
USB capture ran with USBPcap on `\\.\USBPcap1` (the hub that listed the
camera), device address 1, snap length 512, descriptors injected. Gimbly was
closed. We analysed the capture with our own script; the capture and the
recordings stay outside the repository.

Clicks, in order (local time, about 2 s or more apart):

1. 18:44:34 REC with **Normal** and **1080P 120**; 18:44:42 pause; 18:44:50
   resume; 18:44:53 stop.
2. 18:45:01 **Slow Motion**, REC; 18:45:07 stop.
3. **720P 120**, Normal, REC: a confirmation appeared. The first attempt was
   left unanswered. 18:46:04 REC again and Yes; 18:46:09 stop.
4. Recording Format set to `mp4`: 18:46:32 REC, 18:46:35 stop (1080P 120).
5. Format `mov`, bitrate High: 18:47:21 REC, 18:47:25 stop.
6. Format `flv`, bitrate Low: 18:47:45 REC, 18:47:49 stop. Format set back
   to `mkv`.

## Setup

- Camera: OBSBOT Tiny 3 Lite, USB `3564:ff04`, firmware `6.5.11.1` (not
  reread; as in the [Tracking Speed capture](2026-11-21-tracking-speed.md)).
- Official app: OBSBOT Center `v2.0.17.532` (General Settings page).
- OS: Windows 10 Pro 19045.
- Software Settings → Output: Resolution `1080P 60`, Recording Bitrate
  `Low`, Recording Format `mkv`, Media File Location `%USERPROFILE%\Videos`.

## Observed: interface

- The block sits at the top of the Console tab: the title with a help icon,
  a dropdown on the right, the pills **Slow Motion** and **Normal**, a wide
  bar with a red REC dot, a separator dot and `00:00:00`, and a square
  button with a folder icon.
- The dropdown lists `1080P 120` and `720P 120`.
- The help icon does nothing on hover. A left click opens a modal titled
  "Tips" with three numbered notes and an OK button: no beauty effect; audio
  goes to a separate file; the resolution and frame rate apply once the
  recording starts.
- Right clicks on every control in the block do nothing. Hovers show no
  tooltips.
- The folder button opens the Media File Location in the file manager.
- Clicking the bar starts a recording. While recording:
  - the dropdown and both pills are disabled (greyed);
  - the REC dot becomes a red square inside a circle (stop);
  - the timer counts whole seconds `HH:MM:SS`;
  - a pause icon follows the timer. Clicking it freezes the timer and the
    icon changes to play; clicking again resumes.
- Clicking the stop square ends the recording. The block returns to its
  idle look. No message or notification follows.
- With `720P 120` selected, REC first shows a "Tips" modal: the High-fps
  resolution is low, the preview quality may suffer, and the preview
  resolution comes back after the recording. It has a single Yes button,
  which starts the recording. It appeared on every 720P start.
- Choosing bitrate High in Software Settings shows its own warning (high
  bitrate needs a fast computer) with a "No more prompts" box, Cancel and
  Done. That belongs to the bitrate setting, not to this block.
- The block has no global hotkey in the Global Hotkeys list.

## Observed: files

All files land in the Media File Location, named
`<camera serial>_<YYYY-MM-DD_HH-MM-SS>.<ext>`, with a `.wav` of the same
name next to each video.

| Run | Container | Video | Frames / duration | Note |
| --- | --- | --- | --- | --- |
| Normal 1080P | mkv | MJPEG 1920×1080, `yuvj422p`, 120 fps, timebase 1 ms | 1159 / 9.66 s | pause cut out |
| Slow Motion 1080P | mkv | same, stamped at 30 fps | 447 / 14.9 s | 4× slower |
| Normal 720P | mkv | MJPEG 1280×720, 120 fps | 503 / 4.19 s | |
| Normal 1080P | mp4 | MJPEG, sample entry `mp4v`, timebase 1/90000 | 2.26 s | |
| Normal 1080P | mov | MJPEG, sample entry `jpeg`, brand `qt  ` | 2.11 s | bitrate High |
| Normal 1080P | mp4 | same as the mp4 row | 2.08 s | format was `flv` |

- The video bitrate was about 141 Mbit/s at 1080P (88 Mbit/s at 720P) for
  both Low and High. The camera's JPEG frames are stored as they arrive;
  nothing is re-encoded.
- Normal timestamps step 8–9 ms (mkv, 1 ms timebase) or exactly 1/120 s
  (mp4). There is no gap at the pause: the paused time is left out.
- Slow Motion keeps all 120 frames per second of capture and stamps them
  1/30 s apart, so playback is 4× slower.
- The `.wav` is PCM 16-bit, 44.1 kHz, stereo. Its length is close to the
  real recording time in both modes (4.75 s for the 3.7 s Slow Motion
  capture), so the audio is not slowed down.
- Each file carries a comment tag naming the recording type, the camera
  serial and a random ID.

## Observed: USB

- While idle, OBSBOT Center kept the camera streaming MJPEG 1080p at 60 fps
  (format index 1, frame index 1, interval 166666 × 100 ns), the Resolution
  setting.
- At REC, about 0.1 s after the click: the app stopped the video interface
  (interface 1, alternate setting 0) and the audio interface (interface 3,
  alternate setting 0). Then it ran a VS probe/commit for format 1 at
  120 fps (interval 83333) with frame index 1 for 1080P or 3 for 720P. The
  camera reported a maximum frame size of 2 073 600 bytes for 1080P and
  921 600 for 720P. Next came interface 3 alternate setting 1 with a
  48 000 Hz sampling-frequency write to endpoint `0x82`, so the camera
  microphone streamed during the recording.
- Pause and resume each stopped and restarted both interfaces with the same
  120 fps commit.
- Stop restarted the stream at 60 fps (the preview setting) and briefly
  opened the audio interface again.
- No vendor (extension unit) write happened at any click. The only vendor
  traffic was the periodic status polls described in earlier notes.

## Inferred

- High-fps Recording is a host-side feature: the app switches the UVC
  stream to the 120 fps MJPEG mode of the chosen size, copies each JPEG
  frame into the container picked in Recording Format (`flv` falls back to
  `mp4`, which can hold MJPEG), and switches back to the preview mode when
  the recording stops.
- Recording Bitrate and Video Encoding do not apply, since there is no
  encoder in this path.
- Slow Motion is a timestamp choice. The factor is 120 / 30 = 4.
- The `.wav` holds the camera microphone, resampled from 48 kHz to
  44.1 kHz on the host.

## Unknown

- Whether the `.wav` comes from the camera microphone or from the system
  default input. The capture shows the camera microphone streaming, but
  the file was not compared with a second microphone.
- Whether the dropdown and mode survive an app restart.
- Behavior when the disk fills up or the camera is unplugged mid-recording.
- Why the timer showed about 11 s for the first run while the file held
  9.66 s of video; the start of the stream after the mode switch may be
  dropped.

## Gimbly behavior

Gimbly follows the observed behavior with its own code: the same block,
dropdown entries, pills, timer, pause and folder button; the notes behind
the help icon; the question before a 720P recording; MJPEG frames copied
without re-encoding into mkv, mp4 or mov from Recording Format; frames
stamped at a constant 1/120 s (Normal) or 1/30 s (Slow Motion), so a pause
leaves no gap. The session is reopened in the 120 fps MJPEG mode for the
recording and in the preview mode afterwards. While recording, the preview
and virtual camera get every second frame (about 60 fps), decoded in
Gimbly. Recording Bitrate stays disabled because it has no effect here.

The camera microphone goes into a `.wav` (16-bit PCM) with the video's
name. Gimbly finds it by name: an audio input whose name contains "OBSBOT"
and the model name. It is captured with WASAPI on Windows, ALSA
(`plughw`, libasound loaded at run time) on Linux and AVFoundation on
macOS. The sound starts with the first recorded video frame and pauses
with the video. A camera without a matching microphone records video
only, with a notice.

Differences: file names follow Gimbly's snapshot style
(`Gimbly_YYYYMMDD_HHMMSS.mkv`) because Gimbly does not read the camera
serial; the help notes are Gimbly's own text; the `.wav` keeps the
microphone's rate (48 kHz stereo on Windows with the Tiny 3 Lite, and
requested on Linux and macOS) instead of 44.1 kHz.

## Test

- `record::mkv::tests`, `record::mp4::tests`: the muxers write files whose
  structure matches the container layout and point at every frame with the
  expected timestamps.
- `record::tests`: recorded time without pauses, the playback rate in
  Normal and Slow Motion, skipped non-JPEG data and the file name.
- `camera::mjpeg::tests`: damaged frames are rejected.
- `audio::tests`: the WAV header and sizes, and which input names count as
  the camera microphone.

Hardware, Windows 10 with the Tiny 3 Lite, 2026-11-25:

- `gimbly --probe --mode WxH@120 --record DIR` for mkv, mp4, mov, Slow
  Motion and 720P. `ffprobe` showed the same codec tags, rates and
  timestamp steps as the official files, and `ffmpeg` decoded every frame.
- The running app with the preview and virtual camera on: REC, pause,
  resume and stop; a 58 s recording held 6991 frames (120 fps, none lost)
  while the joystick was dragged, and the virtual camera received about
  60 frames per second before and during the recording. The 720P question,
  Slow Motion with mp4, and closing the app mid-recording (the mp4 was
  complete) were checked too.
- Sound: the WAV was 48 kHz stereo. A 1 kHz tone played through the PC
  speakers during a probe recording showed up in the WAV (the 1 kHz band
  rose from about −84 dB to −45 dB). In the app, a recording with a 4 s
  pause gave 5.90 s of video and 5.82 s of sound.

Still to check: Linux and macOS at runtime (both compile and pass Clippy).
The macOS path relies on AVFoundation delivering the native JPEG samples
when the output's video settings are empty, and on the audio output
accepting 48 kHz stereo 16-bit PCM settings. On Linux the camera
microphone must not be held by another program.
