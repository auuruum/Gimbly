# Locked Target and the Human Tracking indicator

## Source and method

2026-11-28. The owner supplied screenshots of Gimbly's More and Console
tabs and reported that an open palm started tracking with Locked Target
enabled, but neither Human Tracking button showed that it was active.
The firmware and OS of the screenshot session were not recorded.
No OBSBOT software was used for this fix.

## Observed

- The screenshots show Locked Target enabled and both Single Mode and
  Group Mode unselected. The camera movement was reported by the owner,
  not measured in that session.
- Earlier [Single and Group Mode captures](2026-11-21-group-mode-auto-zoom.md)
  on a Tiny 3 Lite with firmware `6.5.11.1` gave the same AI status byte
  (`2` at selector 6 offset 24) in both modes. The Locked Target setting
  at offset 1 is an enable switch, not an activity indicator.
- The local Tiny 3 Lite (`3564:ff04`, `bcdDevice` `0x0510`, firmware not
  reread) reported Locked Target enabled and AI mode off before testing.

## Inferred and implemented

Gimbly already polls the AI status about once a second. When it sees human
tracking without a mode Gimbly selected and without object framing, the
Console shows **Tracking on** and **Stop**. It doesn't guess Single or
Group. Stop uses the existing Human Tracking stop command; this change
adds no status requests or new message formats. Both mode buttons still
select their respective mode, and object tracking has its own indicators.

## Tests and remaining checks

- `app::tests::human_tracking_started_outside_gimbly_shows_until_it_stops`
  checks the indicator, Stop click, and exclusions for a gesture switch
  alone, known Single/Group modes, and object tracking.
- The running app was checked on CachyOS Linux `7.3.6-3-cachyos` with the
  local Tiny 3 Lite, on native Wayland (Hyprland) and native X11 (Xvfb).
  A test-only override of status offset 24 supplied `0 → 2 → 6 → 0`
  to the existing poll. The indicator appeared for `2`, stayed during
  the transient value `6`, and cleared at `0`. Clicking Stop on X11
  cleared it immediately before the next simulated update.
- In the X11 run, a V4L2 client read 120 frames from the 1080p30 virtual
  camera across a gimbal drag in the live preview.
- The on/off transition from a real palm gesture in the new build and
  runtime on Windows and macOS are still to check. Whether the camera
  chooses Single or Group for a gesture start is unknown.
