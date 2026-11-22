# Contributing to Gimbly

Thanks for helping. This page is the short version; [AGENTS.md](AGENTS.md)
has the full rules and applies to everyone, human or AI.

## Build and run

You need stable Rust 1.95 or newer. Close other applications using the
physical camera before running Gimbly.

### Windows

```powershell
cargo build --workspace --release
.\target\release\gimbly.exe
```

Build the workspace so the DirectShow DLL is next to `gimbly.exe`. The
app registers it for the current user when you enable Virtual Camera.
The DirectShow device is available to 64-bit clients.

### Linux Wayland and X11

Install your distribution's Wayland client development libraries
(`libwayland-dev` on Debian and Ubuntu) for building. Run Gimbly in either
a native Wayland session or an X11 session. V4L2 camera capture requires
read/write access to the camera's `/dev/video*` device.

```bash
cargo build --release -p gimbly
./target/release/gimbly
```

For development, `cargo run -p gimbly` also optimizes MJPEG decoding and
frame conversion. Gimbly uses the XDG FileChooser portal on Linux and
native X11 registration for X11 hotkeys.
Wayland global hotkeys use the desktop's GlobalShortcuts portal. Install
the desktop entry and icon so the portal can identify Gimbly. The executable
must be on the desktop session's PATH, or you can set `Exec=` in the desktop
file to the executable's absolute path.

```bash
install -Dm755 target/release/gimbly "$HOME/.local/bin/gimbly"
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
install -Dm644 packaging/linux/io.github.auuruum.gimbly.desktop \
  "$data_dir/applications/io.github.auuruum.gimbly.desktop"
install -Dm644 assets/logo/gimbly-icon.svg \
  "$data_dir/icons/hicolor/scalable/apps/io.github.auuruum.gimbly.svg"
```

Use the compositor's shortcut settings if it manages the actual keys.
Hyprland users can list registered actions with `hyprctl globalshortcuts`
and bind them with Hyprland's `global` dispatcher. Wayland also leaves
window stacking and placement to the compositor. The pin button is
disabled in native Wayland windows.
[GlobalShortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) ·
[Hyprland global hotkeys](https://wiki.hypr.land/configuring/core/binds/globals/)

The camera reports the tracked person's or object's box on its USB serial
port (`/dev/ttyACM*`). Most distributions let only the `uucp` or `dialout`
group open it, so without access the preview shows no tracking box. Install
Gimbly's udev rule to give the signed-in user access, then unplug and
replug the camera or run the trigger command:

```bash
sudo install -Dm644 packaging/linux/70-gimbly.rules /etc/udev/rules.d/70-gimbly.rules
```

```bash
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=tty
```

Adding your user to the port's group and signing in again also works.

For Virtual Camera, install `v4l2loopback` for the running kernel if your
distribution does not include it. Click **Virtual Camera** in Gimbly, then
**Create virtual camera…** if the setup dialog opens. Approve the system
prompt. Gimbly starts sending video once its input camera has frames.
When the icon lights up, select **Gimbly Virtual Camera** in the receiving app.

If your desktop has no polkit authentication agent, use **Copy terminal
command** in the dialog, run the command, and click **Check again**. To set up
the device manually with an unloaded driver, run:

```bash
sudo modprobe v4l2loopback devices=1 exclusive_caps=1 'card_label=Gimbly Virtual Camera'
```

If the module is already loaded without a Gimbly device, install your
distribution's `v4l2loopback` utilities and add a dedicated node:

```bash
sudo v4l2loopback-ctl add -x 1 -n 'Gimbly Virtual Camera'
```

The setup dialog adds the device for the running session. The driver docs
describe how to load the module with these options at boot. Some receiving
apps refresh the camera list only after reopening the camera selector.
[v4l2loopback setup](https://github.com/v4l2loopback/v4l2loopback#options)

### macOS

Build on macOS 13 or later with the Xcode Command Line Tools installed.
Allow Gimbly to use the camera when prompted. Login item management uses
System Settings. For an ad hoc app bundle with capture and controls, run:

```bash
./packaging/macos/bundle.sh
open target/Gimbly.app
```

The virtual camera needs the signed Core Media I/O system extension.
Build with a Developer ID Application identity and its Team ID. Provide
provisioning profiles with `GIMBLY_APP_PROFILE` and
`GIMBLY_EXTENSION_PROFILE` when the signing configuration requires them.
Increase `GIMBLY_BUILD_NUMBER` when updating an installed extension.

```bash
GIMBLY_SIGN_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
GIMBLY_TEAM_ID='YOURTEAMID' \
./packaging/macos/bundle.sh target/signed/Gimbly.app
```

Copy the signed bundle to `/Applications`. Turn on Virtual Camera in
Gimbly and approve its camera extension in System Settings, then turn on
Virtual Camera again. Use Apple's signing and notarization workflow for
distribution. [Apple camera extension setup](https://developer.apple.com/documentation/coremediaio/creating-a-camera-extension-with-core-media-i-o)

## Make a change

1. Branch from the latest `main`.

   ```bash
   git fetch
   git switch --no-track -c feature/short-name origin/main
   ```

   Prefixes: `feature/`, `fix/`, `docs/`, `refactor/`, `chore/`.

2. Keep the change small. Add tests for logic that works without a camera.
3. Run the local checks before asking for review.

   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
   cargo build --workspace --all-features --locked
   cargo test --workspace --all-features --locked --no-fail-fast
   ```

4. Share the task branch and validation results with the owner for approval
   to merge into `main`. If the owner requests a pull request, target `main`.

GitHub Actions checks formatting on Linux and runs Clippy, builds, and tests
on Linux, Windows, and macOS. CI runs on pushes to `main`, pull requests
targeting `main`, and manual dispatch. Test native Wayland, native X11, and
camera behavior separately when your change affects them.

## Rules that matter most

- **Clean room.** Never decompile, disassemble, or debug OBSBOT software or
  firmware, and never use the OBSBOT SDK. Learn from the official app's
  visible behavior and from your own camera, then write what you found in
  [docs/clean-room](docs/clean-room/README.md).
- **Never crash.** No `unwrap`, `expect`, or `panic!` on data from the
  camera, a file, or the user.
- **Never block the video.** The UI and frame loop must not wait on camera
  writes.
- **All four targets.** Windows, macOS, Linux Wayland, and Linux X11.
  Report missing OS or compositor capabilities clearly in the app.
- **Icons** are SVG or egui-phosphor, never emoji.

## Tested a camera?

Add a row to [docs/devices.md](docs/devices.md) with the camera, firmware,
OS, what you tried, and the result. Only list what you ran on real hardware.

## License

By contributing you agree that your work is released under the
[MIT license](LICENSE).
