//! The control panel window; the video preview lives in `preview_window`.

mod view_exposure;
mod view_gimbal;
mod view_image;
mod view_more;
mod view_white_balance;

use crate::branding::Branding;
use crate::camera::{
    self, AiMode, AutoZoom, CaptureConfig, Command, DeviceInfo, FOCUS_MAX, Feature, FocusMode,
    GimbalCommand, ObjectFraming, PreviewFrame, Session, SessionState, SessionStatus, TrackingMode,
    TrackingSpeed,
};
use crate::hotkeys::GlobalHotkeys;
use crate::os;
use crate::preview_window::{PreviewAction, PreviewInput, PreviewWindow};
use crate::record;
use crate::settings::{CloseAction, HighFps, HotkeyAction, Settings};
use crate::settings_ui::{self, Change, SettingsWindow};
use crate::snapshot;
use crate::theme::{self, *};
use crate::vcam;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Rect, ResizeDirection, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, ViewportCommand, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const NOT_YET: &str = "Not implemented in Gimbly yet";
const HIGH_FPS_NOTES: &str = "High-fps Recording\n\n\
    1. The file holds the camera's own frames: rotation, mirror and the portrait crop are not applied.\n\
    2. The camera microphone is saved separately, as a WAV file with the same name.\n\
    3. The camera switches to the chosen resolution and frame rate when the recording starts, \
    and back when it stops.";
const LOW_RES_NOTE: &str = "720P 120 has a lower resolution, so the preview looks softer while \
    recording. The preview resolution comes back when the recording stops.";
const TOAST_FOR: Duration = Duration::from_secs(6);

/// Where a snapshot was saved, or why it failed.
type SnapshotResult = Result<PathBuf, String>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Console,
    Image,
    More,
}

pub struct GimblyApp {
    branding: Branding,
    devices: Vec<DeviceInfo>,
    selected: Option<usize>,
    session: Option<Session>,
    status: SessionStatus,
    preview: Option<(u64, egui::TextureHandle)>,
    preview_window: PreviewWindow,
    /// A full-size frame was requested from the capture thread.
    snapshot_requested: bool,
    /// Snapshots are encoded on a helper thread, which reports back here.
    snapshot_results: (Sender<SnapshotResult>, Receiver<SnapshotResult>),
    always_on_top: bool,
    wayland: bool,
    tab: Tab,
    ai_open: bool,
    focus_open: bool,
    /// Manual focus position the user dragged to, until the camera reports it.
    focus_target: Option<u8>,
    exposure_open: bool,
    /// Exposure values the user picked, until the camera reports them.
    exposure_target: Option<(camera::Exposure, Instant)>,
    curve_ui: view_exposure::CurveUi,
    white_balance_open: bool,
    /// The white balance the user picked, until the camera reports it.
    white_balance_target: Option<(camera::WhiteBalance, Instant)>,
    image_open: bool,
    /// The Image values the user picked, until the camera reports them.
    image_target: Option<(camera::ImageAdjust, Instant)>,
    more_ui: view_more::MoreUi,
    /// UVC zoom value the user asked for (cameras without vendor control);
    /// the camera's own reading lags behind while it zooms smoothly.
    zoom_target: Option<i32>,
    gimbal_ui: view_gimbal::GimbalUi,
    /// Requested by the user. SessionStatus reports whether frames are flowing.
    virtual_camera: bool,
    virtual_camera_announced: bool,
    virtual_camera_hint: Option<String>,
    #[cfg(target_os = "linux")]
    virtual_camera_setup: vcam::setup::Setup,
    #[cfg(target_os = "linux")]
    serial_access: crate::serial_access::SerialAccess,
    /// Where the toolbar's sleep button sits, so the sleep overlay can keep
    /// it reachable.
    sleep_button: Rect,
    toast: Option<(String, Instant)>,
    /// The session was opened for High-fps Recording.
    recording: bool,
    /// The High-fps Recording notes are open.
    high_fps_tips: bool,
    /// Asking before a 720P recording.
    confirm_low_res: bool,
    settings: Settings,
    settings_window: SettingsWindow,
    hotkeys: Option<GlobalHotkeys>,
    /// "Turn On/Off Shortcut Keys" pauses every other shortcut.
    hotkeys_paused: bool,
    /// Registrations are dropped while a new shortcut is being recorded.
    hotkeys_suspended: bool,
}

impl GimblyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let branding = Branding::new(&cc.egui_ctx);
        let settings = Settings::load();
        let settings_window = SettingsWindow::new(&settings, branding.icon.clone());
        let wayland = os::is_wayland_window(cc);
        let mut preview_window = PreviewWindow::new(branding.icon.clone());
        preview_window.wayland = wayland;
        let mut app = Self {
            branding,
            devices: Vec::new(),
            selected: None,
            session: None,
            status: SessionStatus::default(),
            preview: None,
            preview_window,
            snapshot_requested: false,
            snapshot_results: channel(),
            always_on_top: false,
            wayland,
            tab: Tab::Console,
            ai_open: true,
            focus_open: true,
            focus_target: None,
            exposure_open: true,
            exposure_target: None,
            curve_ui: Default::default(),
            white_balance_open: true,
            white_balance_target: None,
            image_open: true,
            image_target: None,
            more_ui: Default::default(),
            zoom_target: None,
            gimbal_ui: view_gimbal::GimbalUi::default(),
            virtual_camera: false,
            virtual_camera_announced: false,
            virtual_camera_hint: None,
            #[cfg(target_os = "linux")]
            virtual_camera_setup: Default::default(),
            #[cfg(target_os = "linux")]
            serial_access: Default::default(),
            sleep_button: Rect::NOTHING,
            toast: None,
            recording: false,
            high_fps_tips: false,
            confirm_low_res: false,
            settings,
            settings_window,
            hotkeys: None,
            hotkeys_paused: false,
            hotkeys_suspended: false,
        };
        app.publish_virtual_camera_format();
        app.refresh_devices(&cc.egui_ctx);
        app.refresh_hotkeys(&cc.egui_ctx);
        if std::env::args().any(|a| a == "--virtual-camera") {
            app.toggle_virtual_camera(&cc.egui_ctx);
        }
        // `--settings` opens Software Settings, `--settings=hotkeys` a page.
        if let Some(arg) = std::env::args().find(|a| a.starts_with("--settings")) {
            let page = arg
                .strip_prefix("--settings=")
                .and_then(settings_ui::Page::from_arg);
            app.settings_window
                .open_page(page.unwrap_or(settings_ui::Page::Output));
        }
        app
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.notify(format!("Could not save settings: {e}"));
        }
    }

    /// Tells the virtual camera which format to offer first.
    fn publish_virtual_camera_format(&mut self) {
        #[cfg(windows)]
        {
            let (w, h, fps) = self.settings.resolution.mode();
            let (w, h) = self.settings.capture_config().virtual_size((w, h));
            if let Some(format) = gimbly_vcam_shm::PreferredFormat::validated(w, h, fps)
                && let Err(e) = gimbly_vcam_shm::write_preferred_format(format)
            {
                self.notify(format!("Could not store the virtual camera format: {e}"));
            }
        }
    }

    /// (Re)registers global shortcuts from the current settings.
    fn refresh_hotkeys(&mut self, ctx: &egui::Context) {
        self.hotkeys = None;
        if !self.settings.hotkeys_enabled || self.hotkeys_suspended {
            return;
        }
        let bindings: Vec<_> = self
            .settings
            .hotkeys
            .iter()
            .filter(|(action, _)| action.implemented())
            .filter_map(|(action, combo)| combo.map(|c| (*action, c)))
            .collect();
        let ctx = ctx.clone();
        let (hotkeys, failures) =
            GlobalHotkeys::start(bindings, self.wayland, move || ctx.request_repaint());
        self.hotkeys = Some(hotkeys);
        if !failures.is_empty() {
            self.notify(failures.join("\n"));
        }
    }

    fn handle_hotkey(&mut self, action: HotkeyAction, ctx: &egui::Context) {
        if action == HotkeyAction::ToggleShortcuts {
            self.hotkeys_paused = !self.hotkeys_paused;
            self.notify(if self.hotkeys_paused {
                "Shortcut keys paused"
            } else {
                "Shortcut keys on"
            });
            return;
        }
        if self.hotkeys_paused {
            return;
        }
        let ratio = self.shown_zoom_ratio().unwrap_or(1.0);
        match action {
            HotkeyAction::ToggleVirtualCamera => self.toggle_virtual_camera(ctx),
            HotkeyAction::Device1
            | HotkeyAction::Device2
            | HotkeyAction::Device3
            | HotkeyAction::Device4 => {
                let slot = action as usize - HotkeyAction::Device1 as usize;
                if slot < self.devices.len() && self.selected != Some(slot) {
                    self.select(Some(slot), ctx);
                }
            }
            HotkeyAction::ZoomIn => self.set_zoom_ratio(ratio + view_gimbal::ZOOM_STEP),
            HotkeyAction::ZoomOut => self.set_zoom_ratio(ratio - view_gimbal::ZOOM_STEP),
            HotkeyAction::ZoomMax => self.set_zoom_ratio(f32::MAX),
            HotkeyAction::ZoomMin => self.set_zoom_ratio(1.0),
            HotkeyAction::GimbalReset => self.gimbal(GimbalCommand::Reset),
            HotkeyAction::Preset1 | HotkeyAction::Preset2 | HotkeyAction::Preset3 => {
                let slot = action as usize - HotkeyAction::Preset1 as usize;
                let preset = self.status.presets.as_ref().and_then(|p| p.get(slot));
                match preset {
                    Some(preset) => self.gimbal(GimbalCommand::TriggerPreset(preset.id)),
                    None => self.notify(format!("Preset position {} is not set", slot + 1)),
                }
            }
            HotkeyAction::Snapshot => self.take_snapshot(),
            _ => {}
        }
    }

    /// Reopens the camera after the capture mode or orientation changed.
    fn output_changed(&mut self, ctx: &egui::Context) {
        self.publish_virtual_camera_format();
        self.select(self.selected, ctx);
    }

    fn take_snapshot(&mut self) {
        match &self.session {
            Some(session) if self.status.state == SessionState::Streaming => {
                session.send(Command::Snapshot);
                self.snapshot_requested = true;
            }
            _ => self.notify("Snapshot needs a streaming camera"),
        }
    }

    /// Encodes a snapshot frame off the UI thread.
    fn save_snapshot(&mut self, frame: PreviewFrame, ctx: &egui::Context) {
        let dir = PathBuf::from(&self.settings.media_dir);
        let time = os::local_time();
        let done = self.snapshot_results.0.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = done.send(snapshot::save(&frame, &dir, time));
            ctx.request_repaint();
        });
    }

    fn open_media_folder(&mut self) {
        let dir = &self.settings.media_dir;
        let result = std::fs::create_dir_all(dir)
            .map_err(|e| format!("Could not create {dir}: {e}"))
            .and_then(|()| os::open_folder(dir));
        if let Err(e) = result {
            self.notify(e);
        }
    }

    /// Reopens the camera in the High-fps mode and starts recording into
    /// the media folder (see `docs/clean-room/2026-11-25-high-fps-recording.md`).
    fn start_recording(&mut self, ctx: &egui::Context) {
        let high_fps = self.settings.high_fps;
        let (width, height, fps) = high_fps.mode();
        if self.session.is_none() || self.status.state != SessionState::Streaming {
            self.notify("High-fps Recording needs a streaming camera");
            return;
        }
        let supported = self
            .status
            .modes
            .iter()
            .any(|m| m.codec == "MJPG" && m.covers(width, height, fps));
        if !supported {
            self.notify(format!("This camera cannot record {}", high_fps.label()));
            return;
        }
        let config = CaptureConfig {
            width,
            height,
            fps,
            compressed: true,
            ..self.settings.capture_config()
        };
        let spec = record::Spec {
            dir: PathBuf::from(&self.settings.media_dir),
            time: os::local_time(),
            container: self.settings.record_format.container(),
            slow_motion: self.settings.slow_motion,
        };
        self.open_camera(self.selected, config, ctx);
        if let Some(session) = &self.session {
            session.send(Command::Record(spec));
            self.recording = true;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let view = settings_ui::View {
            devices: &self.devices,
            modes: &self.status.modes,
            capture_format: self.status.format.as_deref(),
        };
        let changes = self.settings_window.show(ctx, &mut self.settings, view);
        for change in &changes {
            match change {
                Change::Output => self.output_changed(ctx),
                Change::Hotkeys => {
                    self.hotkeys_paused = false;
                    self.refresh_hotkeys(ctx);
                }
                Change::Saved => {}
            }
        }
        if !changes.is_empty() {
            self.save_settings();
        }
        let suspended = self.settings_window.is_recording_hotkey();
        if suspended != self.hotkeys_suspended {
            self.hotkeys_suspended = suspended;
            self.refresh_hotkeys(ctx);
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }

    fn refresh_devices(&mut self, ctx: &egui::Context) {
        match camera::list_devices() {
            Ok(devices) => self.devices = devices,
            Err(e) => {
                self.devices.clear();
                self.notify(e);
            }
        }
        let preferred = self
            .devices
            .iter()
            .position(DeviceInfo::is_obsbot)
            .or((!self.devices.is_empty()).then_some(0));
        self.select(preferred, ctx);
    }

    fn select(&mut self, index: Option<usize>, ctx: &egui::Context) {
        self.virtual_camera_announced = false;
        self.open_camera(index, self.settings.capture_config(), ctx);
    }

    /// Back to the preview mode; the old session finishes the file.
    fn stop_recording(&mut self, ctx: &egui::Context) {
        self.open_camera(self.selected, self.settings.capture_config(), ctx);
    }

    fn open_camera(&mut self, index: Option<usize>, config: CaptureConfig, ctx: &egui::Context) {
        // Dropping the old session releases the camera (and finishes a
        // recording) before reopening.
        self.session = None;
        self.recording = false;
        self.status = SessionStatus::default();
        self.preview = None;
        self.zoom_target = None;
        self.gimbal_ui.reset_camera();
        self.snapshot_requested = false;
        self.selected = index.filter(|&i| i < self.devices.len());
        if let Some(i) = self.selected {
            let ctx = ctx.clone();
            self.session = Some(Session::start(
                self.devices[i].clone(),
                config,
                self.virtual_camera,
                move || ctx.request_repaint(),
            ));
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.snapshot_results.1.try_recv() {
            self.notify(match result {
                Ok(path) => format!("Snapshot saved to {}", path.display()),
                Err(e) => e,
            });
        }
        let Some(session) = &self.session else { return };
        let (status, frame) = session.status_and_preview(self.preview_window.open);
        self.status = status;
        let snapshot = self
            .snapshot_requested
            .then(|| session.take_snapshot())
            .flatten();
        if let Some(snapshot) = snapshot {
            self.snapshot_requested = false;
            self.save_snapshot(snapshot, ctx);
        }
        if let Some(frame) = frame {
            let image = egui::ColorImage::new(frame.size, frame.pixels);
            match &mut self.preview {
                Some((epoch, texture)) => {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *epoch = self.status.video_epoch;
                }
                None => {
                    self.preview = Some((
                        self.status.video_epoch,
                        ctx.load_texture("camera-preview", image, egui::TextureOptions::LINEAR),
                    ));
                }
            }
        }
        if self.zoom_target.is_none() {
            self.zoom_target = self.status.zoom;
        }
        if let Some(error) = self
            .status
            .error
            .take()
            .filter(|_| self.status.state != SessionState::Failed)
        {
            // Non-fatal errors (zoom, virtual camera) are shown once.
            self.notify(error);
            // The recording could not start or stopped writing.
            if self.recording && self.status.recording.is_none() {
                self.stop_recording(ctx);
            }
        }
        if let Some(error) = self.status.virtual_camera_error.take() {
            self.virtual_camera = false;
            self.virtual_camera_hint = None;
            self.notify(error);
        } else if self.virtual_camera
            && self.status.virtual_camera
            && !self.virtual_camera_announced
        {
            self.virtual_camera_announced = true;
            let mut message = format!(
                "Virtual camera on. Choose \"{}\" in your video app.",
                vcam::FRIENDLY_NAME
            );
            if let Some(hint) = self.virtual_camera_hint.take() {
                message.push('\n');
                message.push_str(&hint);
            }
            self.notify(message);
        }
        // Frames drive repaints while the preview is open; otherwise refresh
        // the status slowly.
        let wait = if self.status.state == SessionState::Opening {
            100
        } else {
            500
        };
        ctx.request_repaint_after(Duration::from_millis(wait));
    }

    fn toggle_virtual_camera(&mut self, ctx: &egui::Context) {
        if self.virtual_camera {
            self.set_virtual_camera(false);
            return;
        }
        #[cfg(target_os = "linux")]
        self.virtual_camera_setup.check(ctx);
        #[cfg(not(target_os = "linux"))]
        {
            let _ = ctx;
            match vcam::register() {
                Ok(hint) => {
                    self.virtual_camera_hint = hint;
                    self.set_virtual_camera(true);
                }
                Err(error) => self.notify(format!("Virtual camera unavailable: {error}")),
            }
        }
    }

    fn set_virtual_camera(&mut self, on: bool) {
        self.virtual_camera = on;
        self.virtual_camera_announced = false;
        if let Some(session) = &self.session {
            session.send(Command::SetVirtualCamera(on));
        } else if on {
            self.notify("Virtual camera ready. Connect a camera to start sending video.");
        }
        if !on {
            self.virtual_camera_hint = None;
        }
    }

    fn virtual_camera_tip(&self) -> &'static str {
        #[cfg(target_os = "linux")]
        if self.virtual_camera_setup.pending() {
            return "Checking Linux virtual camera setup";
        }
        match (self.virtual_camera, self.status.virtual_camera) {
            (true, true) => "Stop Virtual Camera",
            (true, false) => "Waiting for video. Click to turn off Virtual Camera",
            (false, _) => "Start Virtual Camera",
        }
    }

    fn set_sleep(&mut self, sleep: bool) {
        if let Some(session) = &self.session {
            session.send(Command::SetSleep(sleep));
        }
    }

    fn sleep_tip(&self) -> String {
        if self.status.vendor_control {
            return if self.supports(Feature::Sleep) {
                "Sleep/Resume".into()
            } else {
                self.gimbal_tip(Feature::Sleep)
            };
        }
        if self.status.vendor_control_untested {
            "Sleep/Resume: not verified on this camera model yet"
        } else if self.status.state == SessionState::Streaming {
            "Sleep/Resume: this camera does not support it"
        } else {
            "Sleep/Resume: connect an OBSBOT camera first"
        }
        .into()
    }

    fn set_zoom(&mut self, value: i32) {
        let Some(range) = self.status.zoom_range else {
            return;
        };
        let value = range.snap(value);
        if self.zoom_target != Some(value) {
            self.zoom_target = Some(value);
            if let Some(session) = &self.session {
                session.send(Command::SetZoom(value));
            }
        }
    }

    // ---- control panel --------------------------------------------------

    fn title_bar(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let painter = ui.painter();
        painter.image(
            self.branding.logo.id(),
            Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 6.0), vec2(96.0, 24.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let buttons = Rect::from_min_max(
            pos2(rect.left() + 120.0, rect.top()),
            pos2(rect.right() - 6.0, rect.bottom()),
        );
        place(
            ui,
            buttons,
            Layout::right_to_left(Align::Center),
            "title-buttons",
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_button(ui, icon::X, 26.0, 16.0, false)
                    .on_hover_text("Close")
                    .clicked()
                {
                    // "Close the Software: Minimize" keeps Gimbly (and the
                    // virtual camera) running; closing from the taskbar still
                    // quits.
                    let command = match self.settings.close_action {
                        CloseAction::Minimize => ViewportCommand::Minimized(true),
                        CloseAction::FullyClosed => ViewportCommand::Close,
                    };
                    ui.ctx().send_viewport_cmd(command);
                }
                if icon_button(ui, icon::MINUS, 26.0, 16.0, false)
                    .on_hover_text("Minimize")
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                let pinned = ui.add_enabled_ui(!self.wayland, |ui| {
                    icon_button(ui, icon::PUSH_PIN, 26.0, 15.0, self.always_on_top)
                        .on_hover_text("Always on top")
                        .on_disabled_hover_text("Wayland leaves window stacking to the compositor")
                        .clicked()
                });
                if pinned.inner {
                    self.always_on_top = !self.always_on_top;
                    let level = if self.always_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    };
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::WindowLevel(level));
                }
                if icon_button(ui, icon::GEAR_SIX, 26.0, 16.0, self.settings_window.open)
                    .on_hover_text("Software Settings")
                    .clicked()
                {
                    self.settings_window.open = !self.settings_window.open;
                }
                ui.add_enabled_ui(false, |ui| {
                    icon_button(ui, icon::RECORD, 26.0, 16.0, false)
                        .on_disabled_hover_text(NOT_YET);
                });
            },
        );
    }

    fn device_area(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 154.0), Sense::hover());
        let painter = ui.painter().clone();

        // Device avatar.
        let avatar = pos2(rect.left() + 43.0, rect.top() + 42.0);
        painter.circle(avatar, 24.0, CARD, Stroke::new(1.0, SEPARATOR_LIGHT));
        painter.text(
            avatar,
            Align2::CENTER_CENTER,
            icon::WEBCAM,
            font(24.0),
            TEXT_DIM,
        );
        let avatar_rect = Rect::from_center_size(avatar, Vec2::splat(48.0));
        let tip = match self.selected.map(|i| &self.devices[i]) {
            Some(d) => match &self.status.format {
                Some(f) => format!("{}\n{f}", d.name),
                None => d.name.clone(),
            },
            None => "No camera connected".to_string(),
        };
        ui.interact(avatar_rect, ui.id().with("avatar"), Sense::hover())
            .on_hover_text(tip);

        // Device slots 1-4 and picture-in-picture buttons.
        let container = Rect::from_min_size(
            pos2(rect.left() + 119.0, rect.top() + 14.0),
            vec2(224.0, 106.0),
        );
        painter.rect_filled(container, CornerRadius::same(10), CARD);
        for slot in 0..4 {
            let r = Rect::from_min_size(
                container.min + vec2(10.0 + slot as f32 * 53.0, 7.0),
                Vec2::splat(44.0),
            );
            let device = self.devices.get(slot);
            let selected = self.selected == Some(slot);
            let response = ui.interact(
                r,
                ui.id().with(("slot", slot)),
                if device.is_some() {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let fill = if selected {
                ACCENT
            } else if device.is_some() && response.hovered() {
                BUTTON_HOVER
            } else {
                SLOT
            };
            painter.rect_filled(r, CornerRadius::same(6), fill);
            let text_color = if selected {
                ON_ACCENT
            } else if device.is_some() {
                TEXT
            } else {
                TEXT_FAINT
            };
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                (slot + 1).to_string(),
                font(16.0),
                text_color,
            );
            match device {
                Some(d) => {
                    if response.on_hover_text(&d.name).clicked() && !selected {
                        self.select(Some(slot), &ui.ctx().clone());
                    }
                }
                None => {
                    response.on_hover_text("No camera in this slot");
                }
            }
        }
        for (i, label) in ["Add PiP1", "Add PiP2"].into_iter().enumerate() {
            let r = Rect::from_min_size(
                container.min + vec2(6.0 + i as f32 * 108.0, 66.0),
                vec2(102.0, 34.0),
            );
            painter.rect_filled(r, CornerRadius::same(6), PIP_BUTTON.gamma_multiply(0.6));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                font(14.0),
                TEXT_DIM,
            );
            ui.interact(r, ui.id().with(("pip", i)), Sense::hover())
                .on_hover_text(NOT_YET);
        }
    }

    fn separator(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 2.0), Sense::hover());
        let top = Rect::from_min_size(rect.min, vec2(width, 1.0));
        ui.painter()
            .rect_filled(top, CornerRadius::ZERO, SEPARATOR_DARK);
        ui.painter().rect_filled(
            top.translate(vec2(0.0, 1.0)),
            CornerRadius::ZERO,
            SEPARATOR_LIGHT,
        );
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 48.0), Sense::hover());
        let step = (width - 54.0) / 7.0;
        let slot = |i: usize| {
            Rect::from_center_size(
                pos2(rect.left() + 27.0 + i as f32 * step, rect.center().y),
                vec2(36.0, 36.0),
            )
        };
        // The first button opens the video preview, set apart by a line.
        let preview_open = self.preview_window.open;
        let tip = if preview_open {
            "Close Video Preview"
        } else {
            "Video Preview"
        };
        let clicked = place(ui, slot(0), Layout::default(), ("toolbar", 0), |ui| {
            icon_button(ui, icon::CAMERA, 36.0, 20.0, preview_open)
                .on_hover_text(tip)
                .clicked()
        });
        if clicked {
            self.preview_window.open = !preview_open;
        }
        let line = pos2(
            (slot(0).center().x + slot(1).center().x) / 2.0,
            rect.center().y,
        );
        ui.painter().rect_filled(
            Rect::from_center_size(line, vec2(1.0, 16.0)),
            CornerRadius::ZERO,
            SEPARATOR_LIGHT,
        );

        self.sleep_button = slot(1);
        let sleep_tip = self.sleep_tip();
        let clicked = place(ui, slot(1), Layout::default(), ("toolbar", 1), |ui| {
            ui.add_enabled_ui(
                self.status.vendor_control && self.supports(Feature::Sleep),
                |ui| {
                    icon_button(ui, icon::POWER, 36.0, 20.0, false)
                        .on_hover_text(&sleep_tip)
                        .on_disabled_hover_text(&sleep_tip)
                        .clicked()
                },
            )
            .inner
        });
        if clicked {
            self.set_sleep(!self.status.sleeping);
        }

        let disabled = [
            (2, icon::MONITOR, "View mode"),
            (4, icon::ARROWS_CLOCKWISE, "Reset gimbal"),
            (5, icon::WAVEFORM, "Audio"),
            (6, icon::CHART_PIE_SLICE, "Performance"),
            (7, icon::CARET_DOWN, "More tools"),
        ];
        for (i, glyph, name) in disabled {
            place(ui, slot(i), Layout::default(), ("toolbar", i), |ui| {
                ui.disable();
                icon_button(ui, glyph, 36.0, 20.0, false)
                    .on_disabled_hover_text(format!("{name}: {NOT_YET}"));
            });
        }
        let clicked = place(ui, slot(3), Layout::default(), ("toolbar", 3), |ui| {
            icon_button(
                ui,
                icon::WEBCAM,
                36.0,
                20.0,
                self.virtual_camera && self.status.virtual_camera,
            )
            .on_hover_text(self.virtual_camera_tip())
            .clicked()
        });
        if clicked {
            self.toggle_virtual_camera(ui.ctx());
        }
    }

    fn card(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, CARD);

        // Tabs.
        let (tabs, _) = ui.allocate_exact_size(vec2(rect.width(), 58.0), Sense::hover());
        for (tab, label, x) in [
            (Tab::Console, "Console", 75.0),
            (Tab::Image, "Image", 175.0),
            (Tab::More, "More", 258.0),
        ] {
            let center = pos2(tabs.left() + x, tabs.top() + 26.0);
            let hit = Rect::from_center_size(center, vec2(80.0, 40.0));
            let response = ui.interact(hit, ui.id().with(label), Sense::click());
            let active = self.tab == tab;
            let color = if active || response.hovered() {
                TEXT
            } else {
                TEXT_DIM
            };
            ui.painter()
                .text(center, Align2::CENTER_CENTER, label, font(15.0), color);
            if active {
                let bar =
                    Rect::from_center_size(pos2(center.x, tabs.top() + 44.0), vec2(20.0, 3.0));
                ui.painter().rect_filled(bar, CornerRadius::same(1), ACCENT);
            }
            if response.clicked() {
                self.tab = tab;
            }
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                match self.tab {
                    // Set replaces the console with the Hand Tracking range
                    // panel; a camera that goes away closes it.
                    Tab::Console if self.gimbal_ui.hand_range_open && self.gimbal_ready() => {
                        self.hand_range_panel(ui)
                    }
                    Tab::Console if self.gimbal_ui.zone_open && self.gimbal_ready() => {
                        self.zone_panel(ui)
                    }
                    Tab::Console => {
                        self.gimbal_ui.hand_range_open = false;
                        self.gimbal_ui.zone_open = false;
                        self.console(ui)
                    }
                    Tab::Image => self.image(ui),
                    Tab::More => self.more(ui),
                }
            });
    }

    /// The High-fps Recording block at the top of the console.
    fn high_fps(&mut self, ui: &mut Ui) {
        let recording = self.recording;
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.label(egui::RichText::new("High-fps Recording").size(15.0));
            if help_button(ui).clicked() {
                self.high_fps_tips = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let labels: Vec<&str> = HighFps::ALL.iter().map(|m| m.label()).collect();
                let current = self.settings.high_fps.label();
                let chosen = ui
                    .add_enabled_ui(!recording, |ui| {
                        dropdown(ui, current, &labels, vec2(131.0, 30.0))
                    })
                    .inner;
                if let Some(&mode) = chosen.and_then(|i| HighFps::ALL.get(i)) {
                    self.settings.high_fps = mode;
                    self.save_settings();
                }
            });
        });
        ui.add_space(16.0);
        let selected = (!recording).then_some(usize::from(!self.settings.slow_motion));
        let chosen = ui
            .add_enabled_ui(!recording, |ui| {
                pill_row(ui, &["Slow Motion", "Normal"], selected, 16.0)
            })
            .inner;
        if let Some(i) = chosen {
            self.settings.slow_motion = i == 0;
            self.save_settings();
        }
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if self.record_bar(ui, vec2(ui.available_width() - 84.0, 46.0)) {
                if recording {
                    self.stop_recording(ui.ctx());
                } else if self.settings.high_fps == HighFps::Hd120 {
                    self.confirm_low_res = true;
                } else {
                    self.start_recording(ui.ctx());
                }
            }
            let (folder, response) = ui.allocate_exact_size(vec2(68.0, 46.0), Sense::click());
            let fill = if response.hovered() {
                BUTTON_HOVER
            } else {
                BUTTON
            };
            ui.painter()
                .rect_filled(folder, CornerRadius::same(6), fill);
            ui.painter().text(
                folder.center(),
                Align2::CENTER_CENTER,
                icon::FOLDER_SIMPLE,
                font(22.0),
                TEXT,
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.open_media_folder();
            }
        });
    }

    /// The REC / stop bar with the timer and, while recording, the pause
    /// button. Returns true when the bar itself was clicked.
    fn record_bar(&mut self, ui: &mut Ui, size: Vec2) -> bool {
        let (bar, response) = ui.allocate_exact_size(size, Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let fill = if response.hovered() {
            BUTTON_HOVER
        } else {
            BUTTON
        };
        let painter = ui.painter().clone();
        painter.rect_filled(bar, CornerRadius::same(6), fill);
        let state = self.status.recording.clone().filter(|_| self.recording);
        let elapsed = state.as_ref().map_or(0, |s| s.elapsed.as_secs());
        let timer = format!(
            "{:02}:{:02}:{:02}",
            elapsed / 3600,
            elapsed / 60 % 60,
            elapsed % 60
        );
        let dot = pos2(bar.left() + 64.0, bar.center().y);
        if !self.recording {
            painter.circle(dot, 13.0, ACCENT, Stroke::new(2.0, Color32::WHITE));
            painter.text(dot, Align2::CENTER_CENTER, "REC", font(9.0), ON_ACCENT);
            painter.circle_filled(pos2(dot.x + 22.0, dot.y), 2.0, TEXT);
            painter.text(
                pos2(dot.x + 30.0, dot.y),
                Align2::LEFT_CENTER,
                timer,
                font(14.0),
                TEXT,
            );
            return response.clicked();
        }
        painter.circle_stroke(dot, 13.0, Stroke::new(1.5, TEXT));
        painter.rect_filled(
            Rect::from_center_size(dot, Vec2::splat(10.0)),
            CornerRadius::same(2),
            ACCENT,
        );
        let text = painter.text(
            pos2(dot.x + 22.0, dot.y),
            Align2::LEFT_CENTER,
            timer,
            font(14.0),
            TEXT,
        );
        let paused = state.is_some_and(|s| s.paused);
        let pause = Rect::from_center_size(pos2(text.right() + 26.0, dot.y), Vec2::splat(30.0));
        let pause_response = ui
            .interact(pause, ui.id().with("record-pause"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if pause_response.hovered() {
            painter.rect_filled(pause, CornerRadius::same(5), SLEEP_BUTTON_HOVER);
        }
        let glyph = if paused { icon::PLAY } else { icon::PAUSE };
        painter.text(
            pause.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(16.0),
            TEXT,
        );
        if pause_response.clicked() {
            if let Some(session) = &self.session {
                session.send(Command::PauseRecording(!paused));
            }
            return false;
        }
        response.clicked()
    }

    /// The High-fps notes and the question before a 720P recording.
    fn high_fps_dialogs(&mut self, ctx: &egui::Context) {
        if self.high_fps_tips && tips(ctx, "high-fps-tips", HIGH_FPS_NOTES, "OK").is_some() {
            self.high_fps_tips = false;
        }
        if self.confirm_low_res
            && let Some(start) = tips(ctx, "high-fps-low-res", LOW_RES_NOTE, "Yes")
        {
            self.confirm_low_res = false;
            if start {
                self.start_recording(ctx);
            }
        }
    }

    /// The Image tab: HDR, Focus, Exposure, Anti-Flicker, White Balance and Image.
    fn image(&mut self, ui: &mut Ui) {
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 14,
            bottom: 14,
        };
        let hdr = self.status.hdr;
        // Like the official app: HDR only while the camera runs at 30 fps or less.
        let slow = self.status.frame_rate.is_some_and(|fps| fps <= 30);
        let ready = self.ready(Feature::Hdr) && hdr.is_some() && slow;
        let tip = if self.ready(Feature::Hdr) && !slow {
            "HDR needs a 30 fps resolution, such as 4K 30 or 1080P 30".to_owned()
        } else {
            self.gimbal_tip(Feature::Hdr)
        };
        let mut clicked = false;
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            ui.add_enabled_ui(ready, |ui| {
                let (switch, _) = switch_row(
                    ui,
                    "HDR",
                    "Keeps detail in bright and dark parts of the picture, such as a window behind you",
                    None,
                    hdr.unwrap_or(false),
                );
                clicked = switch.on_disabled_hover_text(&tip).clicked();
            });
        });
        if clicked && let Some(on) = hdr {
            self.gimbal(GimbalCommand::SetHdr(!on));
        }
        divider(ui);
        self.focus(ui);
        divider(ui);
        self.exposure(ui);
        divider(ui);
        self.anti_flicker(ui);
        divider(ui);
        self.white_balance(ui);
        divider(ui);
        self.image_adjust(ui);
        divider(ui);
    }

    /// The Focus section: Auto Focus, its mode, or the manual position.
    fn focus(&mut self, ui: &mut Ui) {
        let focus = self.status.focus;
        let ready = self.ready(Feature::Focus) && focus.is_some();
        let tip = self.gimbal_tip(Feature::Focus);
        let reset;
        (self.focus_open, reset) = section_header(
            ui,
            "Focus",
            self.focus_open,
            ready.then_some("Reset to Auto Focus on faces"),
        );
        // Like the official app: reset turns Auto Focus on in Face mode and
        // keeps the manual position.
        if reset && let Some(focus) = focus {
            if !focus.auto {
                self.gimbal(GimbalCommand::SetAutoFocus(true));
            }
            if focus.mode != FocusMode::Face {
                self.gimbal(GimbalCommand::SetFocusMode(FocusMode::Face));
            }
        }
        if !self.focus_open {
            return;
        }
        if self.focus_target == focus.map(|f| f.position) {
            self.focus_target = None;
        }
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 4,
            bottom: 18,
        };
        let auto = focus.is_none_or(|f| f.auto);
        let mut command = None;
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_enabled_ui(ready, |ui| {
                let (switch, _) = switch_row(
                    ui,
                    "Auto Focus",
                    "Keeps the picture sharp by itself. Turn it off to set the focus by hand",
                    None,
                    auto,
                );
                if switch.on_disabled_hover_text(&tip).clicked() {
                    command = Some(GimbalCommand::SetAutoFocus(!auto));
                }
                ui.horizontal(|ui| {
                    ui.set_height(30.0);
                    if auto {
                        ui.label(egui::RichText::new("Auto Focus Mode").size(15.0));
                        help(
                            ui,
                            "Global focuses on the whole picture, Face on faces first",
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let selected = focus.map(|f| usize::from(f.mode == FocusMode::Face));
                            match segmented(ui, &["Global", "Face"], selected) {
                                Some(0) => {
                                    command = Some(GimbalCommand::SetFocusMode(FocusMode::Global))
                                }
                                Some(_) => {
                                    command = Some(GimbalCommand::SetFocusMode(FocusMode::Face))
                                }
                                None => {}
                            }
                        });
                    } else {
                        ui.label(egui::RichText::new("Manual Focus").size(15.0));
                        let position = self.focus_target.or(focus.map(|f| f.position)).unwrap_or(0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (track, _) =
                                ui.allocate_exact_size(vec2(160.0, 20.0), egui::Sense::hover());
                            let t = f32::from(position) / f32::from(FOCUS_MAX);
                            if let Some(t) = slider(ui, track, t, ready, "manual-focus") {
                                let value = (t * f32::from(FOCUS_MAX)).round() as u8;
                                if value != position {
                                    self.focus_target = Some(value);
                                    command = Some(GimbalCommand::SetFocusPosition(value));
                                }
                            }
                        });
                    }
                });
            });
        });
        if let Some(command) = command {
            self.gimbal(command);
        }
    }

    fn console(&mut self, ui: &mut Ui) {
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 6,
            bottom: 22,
        };
        band(ui, CARD, margin, |ui| self.high_fps(ui));

        divider(ui);
        (self.ai_open, _) = section_header(ui, "AI Tracking", self.ai_open, None);
        if self.ai_open {
            let margin = egui::Margin {
                left: PADDING as i8,
                right: PADDING as i8,
                top: 18,
                bottom: 26,
            };
            let ai_mode = self.status.ai_mode;
            let ready = self.ready(Feature::HumanTracking) && ai_mode.is_some();
            // The camera reports on/off only; highlight the mode Gimbly set.
            let mode = self
                .status
                .tracking_mode
                .filter(|_| ai_mode == Some(AiMode::Human));
            let hand_running = ai_mode == Some(AiMode::Hand);
            let object = self.status.object_framing;
            let zoom = self.status.auto_zoom;
            let zoom_ready = self.ready(Feature::AutoZoom) && zoom.is_some();
            let speed = self.status.tracking_speed;
            let speed_ready = self.ready(Feature::TrackingSpeed) && speed.is_some();
            let tip = self.gimbal_tip(Feature::HumanTracking);
            let zoom_tip = self.gimbal_tip(Feature::AutoZoom);
            let speed_tip = self.gimbal_tip(Feature::TrackingSpeed);
            let sound_tip = self.gimbal_tip(Feature::SoundSource);
            let zone_tip = if hand_running || object.is_some() {
                "Not available during Hand Tracking or object tracking".to_owned()
            } else {
                self.gimbal_tip(Feature::ZoneTracking)
            };
            let mut picked_zone = None;
            let mut open_zone = false;
            let object_ready = self.ready(Feature::ObjectTracking);
            let object_tip = self.gimbal_tip(Feature::ObjectTracking);
            let mut picked_mode = None;
            let mut picked_zoom = None;
            let mut picked_speed = None;
            let mut picked_object = None;
            let mut picked_sound = None;
            band(ui, SECTION, margin, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                group_label(ui, Some(icon::PERSON_SIMPLE), "Human Tracking");
                ui.add_space(12.0);
                let gap = 16.0;
                let size = vec2((ui.available_width() - gap) / 2.0, BUTTON_HEIGHT);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (m, label, hint) in [
                        (
                            TrackingMode::Single,
                            "Single Mode",
                            "Follow one person; click again to stop",
                        ),
                        (
                            TrackingMode::Group,
                            "Group Mode",
                            "Keep everyone in view; click again to stop",
                        ),
                    ] {
                        let clicked = ui
                            .add_enabled_ui(ready, |ui| {
                                pill(ui, label, mode == Some(m), size)
                                    .on_hover_text(hint)
                                    .on_disabled_hover_text(&tip)
                            })
                            .inner
                            .clicked();
                        if clicked {
                            picked_mode = Some((mode != Some(m)).then_some(m));
                        }
                    }
                });
                ui.add_space(22.0);
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Auto Zoom").size(15.0));
                    help(ui, "How tightly tracking frames the person");
                });
                ui.add_space(12.0);
                let width = (ui.available_width() - 2.0 * 16.0) / 3.0;
                for row in AutoZoom::ALL.chunks(3) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        for &z in row {
                            let enabled = zoom_ready
                                && !(mode == Some(TrackingMode::Group) && z.single_only());
                            let clicked = ui
                                .add_enabled_ui(enabled, |ui| {
                                    pill(
                                        ui,
                                        auto_zoom_label(z),
                                        zoom == Some(z),
                                        vec2(width, BUTTON_HEIGHT),
                                    )
                                    .on_disabled_hover_text(
                                        if zoom_ready {
                                            "Not available in Group Mode"
                                        } else {
                                            zoom_tip.as_str()
                                        },
                                    )
                                })
                                .inner
                                .clicked();
                            if clicked {
                                picked_zoom = Some(z);
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
                ui.add_space(10.0);
                let sound = self.status.sound_source;
                let sound_ready = self.ready(Feature::SoundSource) && sound.is_some();
                // The official app shows Zone Tracking off and disabled during
                // Hand and object tracking.
                let zone_off = hand_running || object.is_some();
                let zone = self.status.zone_tracking.filter(|_| !zone_off);
                let zone_ready = self.ready(Feature::ZoneTracking) && zone.is_some();
                let clicks = ai_tracking(
                    ui,
                    (zone, zone_ready, &zone_tip),
                    (sound, sound_ready, &sound_tip),
                );
                if clicks.sound {
                    picked_sound = sound.map(|on| !on);
                }
                if clicks.zone {
                    picked_zone = zone.map(|on| !on);
                }
                open_zone = clicks.zone_settings;
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(icon::CUBE).size(16.0));
                    ui.label(egui::RichText::new("Object Tracking").size(15.0));
                    ui.label(
                        egui::RichText::new("(Select object in preview)")
                            .size(12.0)
                            .color(TEXT_DIM),
                    );
                });
                ui.add_space(12.0);
                let framings = [ObjectFraming::Normal, ObjectFraming::CloseUp];
                let row = ui.add_enabled_ui(object_ready && object.is_some(), |ui| {
                    pill_row(
                        ui,
                        &["Normal", "Close-up"],
                        object.and_then(|o| framings.iter().position(|&f| f == o)),
                        16.0,
                    )
                });
                row.response.on_disabled_hover_text(if object_ready {
                    "Select an object with the Object Tracking button in the preview"
                } else {
                    object_tip.as_str()
                });
                if let Some(i) = row.inner {
                    picked_object = framings.get(i).copied();
                }
                ui.add_space(22.0);
                group_label(ui, Some(icon::GAUGE), "Tracking Speed");
                ui.add_space(12.0);
                let row = ui.add_enabled_ui(speed_ready, |ui| {
                    pill_row(
                        ui,
                        &["Slow", "Standard", "Fast"],
                        speed.and_then(|s| TrackingSpeed::ALL.iter().position(|&x| x == s)),
                        16.0,
                    )
                });
                row.response.on_disabled_hover_text(&speed_tip);
                picked_speed = row.inner;
            });
            if let Some(mode) = picked_mode {
                self.gimbal(GimbalCommand::SetHumanTracking(mode));
            }
            if let Some(zoom) = picked_zoom {
                self.gimbal(GimbalCommand::SetAutoZoom(zoom));
            }
            if let Some(i) = picked_speed {
                self.gimbal(GimbalCommand::SetTrackingSpeed(TrackingSpeed::ALL[i]));
            }
            if let Some(on) = picked_sound {
                self.gimbal(GimbalCommand::SetSoundSource(on));
            }
            if let Some(on) = picked_zone {
                self.gimbal(GimbalCommand::SetZoneTracking(on));
            }
            if open_zone {
                self.gimbal_ui.zone_open = true;
                self.gimbal(GimbalCommand::OpenZone);
            }
            if let Some(framing) = picked_object {
                // Clicking the highlighted framing stops object tracking.
                self.gimbal(if object == Some(framing) {
                    GimbalCommand::CancelTarget
                } else {
                    GimbalCommand::SetObjectFraming(framing)
                });
            }
        }

        divider(ui);
        self.ai_mode_section(ui);
        divider(ui);
        self.preset_section(ui);
        divider(ui);
        self.view_gimbal_section(ui);
    }

    /// Covers the controls while the camera sleeps, like OBSBOT Center does.
    /// Only the toolbar's sleep button and the overlay's own controls stay
    /// usable.
    fn sleep_overlay(&mut self, ctx: &egui::Context, rect: Rect) {
        let mut wake = false;
        egui::Area::new(egui::Id::new("sleep-overlay"))
            .fixed_pos(rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                // Swallows clicks and scrolling meant for the controls below.
                ui.allocate_rect(rect, Sense::click_and_drag());
                let painter = ui.painter().clone();
                painter.rect_filled(rect, CornerRadius::ZERO, SLEEP_SHADE);

                // Vertical positions follow OBSBOT Center's proportions and
                // keep their spacing when the window is short.
                let at = |fraction: f32| rect.top() + rect.height() * fraction;
                let text_y = at(0.28);
                let button_y = at(0.526).max(text_y + 90.0);
                let label_y = at(0.722).max(button_y + 56.0);
                let toggle_y = label_y + 40.0;

                let mut job = egui::text::LayoutJob::simple(
                    "Device is sleeping".into(),
                    font(40.0),
                    TEXT,
                    rect.width() - 80.0,
                );
                job.halign = Align::Center;
                let galley = painter.layout_job(job);
                painter.galley(
                    pos2(rect.center().x, text_y - galley.size().y / 2.0),
                    galley,
                    TEXT,
                );

                let button =
                    Rect::from_center_size(pos2(rect.center().x, button_y), vec2(160.0, 40.0));
                let response = ui.interact(button, ui.id().with("resume"), Sense::click());
                let fill = if response.hovered() {
                    SLEEP_BUTTON_HOVER
                } else {
                    SLEEP_BUTTON
                };
                painter.rect_filled(button, CornerRadius::same(6), fill);
                painter.text(
                    button.center(),
                    Align2::CENTER_CENTER,
                    "Resume",
                    font(15.0),
                    TEXT,
                );
                wake |= response.clicked();

                painter.text(
                    pos2(rect.center().x, label_y),
                    Align2::CENTER_CENTER,
                    "MIC Status During Sleep",
                    font(15.0),
                    TEXT,
                );
                let toggle_rect =
                    Rect::from_center_size(pos2(rect.center().x, toggle_y), vec2(40.0, 22.0));
                place(ui, toggle_rect, Layout::default(), "sleep-mic", |ui| {
                    ui.disable();
                    toggle(ui, true).on_disabled_hover_text(NOT_YET);
                });

                // The toolbar button stays on top so it can wake the camera.
                let tip = self.sleep_tip();
                wake |= place(
                    ui,
                    self.sleep_button,
                    Layout::default(),
                    "sleep-button",
                    |ui| {
                        icon_button(ui, icon::POWER, 36.0, 20.0, false)
                            .on_hover_text(tip)
                            .clicked()
                    },
                );
            });
        if wake {
            self.set_sleep(false);
        }
    }

    fn resize_grip(&self, ui: &mut Ui) {
        let full = ui.max_rect();
        let grip = Rect::from_min_max(pos2(full.left(), full.bottom() - 5.0), full.max);
        let response = ui.interact(grip, ui.id().with("resize-grip"), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(ResizeDirection::South));
        }
    }

    // ---- preview window ---------------------------------------------------

    fn preview_window(&mut self, ctx: &egui::Context) {
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, since)| since.elapsed() <= TOAST_FOR)
            .map(|(message, _)| message.clone());
        let input = PreviewInput {
            texture: self
                .preview
                .as_ref()
                .map(|(epoch, texture)| (*epoch, texture)),
            status: &self.status,
            device: self.selected.map(|i| self.devices[i].name.as_str()),
            resolution: self.settings.resolution,
            preview_mode: self.settings.preview_mode,
            virtual_camera: self.virtual_camera && self.status.virtual_camera,
            virtual_camera_tip: self.virtual_camera_tip(),
            transform: self.settings.transform,
            toast: toast.as_deref(),
            gimbal: self.status.vendor_control && self.supports(Feature::Gimbal),
            object_tracking: self.status.vendor_control && self.supports(Feature::ObjectTracking),
            composition_lines: self.status.vendor_control && self.supports(Feature::Composition),
            tracking: self.status.ai_mode == Some(AiMode::Human),
            target: self.status.target,
            composition: self.status.composition,
            hint: !self.settings.preview_hint_seen,
        };
        let actions = self.preview_window.show(ctx, input);
        for action in actions {
            match action {
                PreviewAction::Retry => self.refresh_devices(ctx),
                PreviewAction::ToggleVirtualCamera => self.toggle_virtual_camera(ctx),
                PreviewAction::Snapshot => self.take_snapshot(),
                PreviewAction::OpenFolder => self.open_media_folder(),
                PreviewAction::SetResolution(resolution) => {
                    self.settings.resolution = resolution;
                    self.save_settings();
                    self.output_changed(ctx);
                }
                PreviewAction::SetTransform(transform) => {
                    self.settings.transform = transform;
                    self.save_settings();
                    if let Some(session) = &self.session {
                        session.send(Command::SetTransform(transform));
                    }
                }
                PreviewAction::Gimbal(drag) => self.joystick_drag(drag, true),
                PreviewAction::Command(command) => self.gimbal(command),
                PreviewAction::Notify(message) => self.notify(message),
                PreviewAction::HintSeen => {
                    self.settings.preview_hint_seen = true;
                    self.save_settings();
                }
                PreviewAction::SetPreviewMode(mode) => {
                    self.settings.preview_mode = mode;
                    self.save_settings();
                    self.output_changed(ctx);
                }
            }
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some((message, since)) = &self.toast else {
            return;
        };
        if since.elapsed() > TOAST_FOR {
            self.toast = None;
            return;
        }
        toast(ctx, message, 24.0, PANEL_WIDTH - 60.0);
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn auto_zoom_label(zoom: AutoZoom) -> &'static str {
    match zoom {
        AutoZoom::Off => "Off",
        AutoZoom::FullBody => "Full Body",
        AutoZoom::UpperBody => "Upper Body",
        AutoZoom::CloseUp => "Close-up",
        AutoZoom::Headless => "Headless",
        AutoZoom::LowerBody => "Lower Body",
    }
}

/// What was clicked in [`ai_tracking`].
struct AiTrackingClicks {
    zone: bool,
    zone_settings: bool,
    sound: bool,
}

/// The Zone Tracking and Sound Source Assisted Tracking rows; each takes
/// (state, ready, why it is disabled).
fn ai_tracking(
    ui: &mut Ui,
    (zone, zone_ready, zone_tip): (Option<bool>, bool, &str),
    (sound, sound_ready, sound_tip): (Option<bool>, bool, &str),
) -> AiTrackingClicks {
    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
    let (zone_switch, zone_settings) = ui
        .add_enabled_ui(zone_ready, |ui| {
            let (switch, settings) = switch_row(
                ui,
                "Zone Tracking",
                "Track only within a set range; the gimbal returns to a set position when                  the person leaves it. The gear opens its settings.",
                Some("Zone Tracking settings"),
                zone.unwrap_or(false),
            );
            (switch.on_disabled_hover_text(zone_tip), settings)
        })
        .inner;
    ui.add_space(18.0);
    let (sound_switch, _) = ui
        .add_enabled_ui(sound_ready, |ui| {
            switch_row(
                ui,
                "Sound Source Assisted Tracking",
                "Use the microphones to help find who is speaking",
                None,
                sound.unwrap_or(false),
            )
        })
        .inner;
    AiTrackingClicks {
        zone: zone_switch.clicked(),
        zone_settings,
        sound: sound_switch.on_disabled_hover_text(sound_tip).clicked(),
    }
}

/// A labelled switch, with a gear button when `settings` names its tooltip.
/// Returns the switch and whether the gear was clicked.
fn switch_row(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    settings: Option<&str>,
    on: bool,
) -> (Response, bool) {
    ui.horizontal(|ui| {
        ui.set_height(24.0);
        ui.label(egui::RichText::new(label).size(15.0));
        help(ui, hint);
        let gear = settings.is_some_and(|tip| {
            icon_button(ui, icon::GEAR_SIX, 20.0, 14.0, false)
                .on_hover_text(tip)
                .clicked()
        });
        let switch = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| toggle(ui, on))
            .inner;
        (switch, gear)
    })
    .inner
}

/// A "Tips" modal with one button: `Some(true)` for the button,
/// `Some(false)` when dismissed.
fn tips(ctx: &egui::Context, id: &str, text: &str, button: &str) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(
            egui::Frame::new()
                .fill(CARD)
                .stroke(Stroke::new(1.0, FIELD_STROKE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(270.0);
            ui.spacing_mut().item_spacing = vec2(8.0, 14.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Tips").size(15.0).color(TEXT));
            });
            ui.label(egui::RichText::new(text).size(13.0).color(TEXT));
            ui.vertical_centered(|ui| {
                if pill(ui, button, false, vec2(85.0, 31.0)).clicked() {
                    answer = Some(true);
                }
            });
        });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

impl eframe::App for GimblyApp {
    /// Runs every frame and also while the window is minimised, so global
    /// shortcuts keep working in the background.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "linux")]
        match self.virtual_camera_setup.poll() {
            Some(vcam::setup::Outcome::Ready) => self.set_virtual_camera(true),
            Some(vcam::setup::Outcome::NeedsSetup) => {
                // A preview or global shortcut can ask for setup while the
                // main panel (which owns the dialog) is minimized.
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Focus);
            }
            None => {}
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(error) = self.hotkeys.as_ref().and_then(GlobalHotkeys::take_error) {
            self.notify(error);
        }
        let actions = self
            .hotkeys
            .as_ref()
            .map(GlobalHotkeys::poll)
            .unwrap_or_default();
        for action in actions {
            // Keep zoom steps relative to the camera's latest state.
            if let Some(session) = &self.session {
                self.status = session.status();
            }
            self.handle_hotkey(action, ctx);
        }
    }

    /// The process ends right after this, before the camera thread could
    /// clean up, so leave the Hand and Zone Tracking edit states here.
    fn on_exit(&mut self) {
        // Let the camera thread finish the recording file.
        if self.recording
            && let Some(session) = self.session.take()
        {
            session.close(Duration::from_secs(5));
            return;
        }
        let Some(session) = &self.session else { return };
        let editing =
            |status: &camera::SessionStatus| status.hand_range.is_some() || status.zone.is_some();
        if !editing(&session.status()) {
            return;
        }
        for command in [
            GimbalCommand::CloseHandRange { save: false },
            GimbalCommand::CloseZone { save: false },
        ] {
            session.send(Command::Gimbal(command));
        }
        let until = Instant::now() + Duration::from_secs(1);
        while editing(&session.status()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);

        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, WINDOW);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        self.title_bar(ui);
        self.device_area(ui);
        let controls_top = ui.cursor().top();
        self.separator(ui);
        self.toolbar(ui);
        self.card(ui);
        self.resize_grip(ui);
        if self.status.sleeping && self.status.vendor_control {
            let controls = Rect::from_min_max(pos2(full.left(), controls_top), full.max);
            self.sleep_overlay(&ctx, controls.shrink2(vec2(1.0, 0.0)));
        }
        ui.painter().rect_stroke(
            full,
            CornerRadius::ZERO,
            Stroke::new(1.0, WINDOW_EDGE),
            StrokeKind::Inside,
        );

        self.rename_dialog(&ctx);
        self.hand_range_dialog(&ctx);
        self.zone_dialog(&ctx);
        self.high_fps_dialogs(&ctx);
        self.preview_window(&ctx);
        self.settings_window(&ctx);
        #[cfg(target_os = "linux")]
        self.virtual_camera_setup.show(&ctx);
        #[cfg(target_os = "linux")]
        self.serial_access
            .show(&ctx, self.status.serial_denied.as_ref());
        self.toast(&ctx);
    }
}
