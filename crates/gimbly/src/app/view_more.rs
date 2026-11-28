//! The More tab: Device Sleep, Gesture Control, Voice Control, Audio,
//! Mirror Image, View and Gimbal Reversed and Indicator Status.

use super::view_exposure::slider_row;
use super::view_gimbal::{menu_entry, menu_frame, question};
use super::{GimblyApp, tips};
use crate::camera::{
    Audio, AudioInput, AudioMode, DeviceSleep, Feature, GESTURE_ZOOM, Gesture, GimbalCommand,
    SLEEP_BACKGROUNDS, SLEEP_TIMES, STATUS_LIGHT_MAX, StatusLight, VOICE_ZOOM_MAX, VOICE_ZOOM_STEP,
    VoiceCommand, VoiceControl,
};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, IconData, Layout, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::sync::Arc;

const SLEEP_TIME_HINT: &str =
    "While no app is using the video, the camera goes to sleep after this time.";
const SLEEP_BACKGROUND_TIPS: &str = "What the preview and the virtual camera show while the \
    camera sleeps: black, one of Gimbly's own pictures, or a PNG or JPEG picture of your own. \
    Click + to add a picture; right-click it to remove it.";
const GESTURE_TIPS: &str = "Show a gesture to the camera to control it without the app.\n\n\
    1. Locked Target: hold up an open palm to start or stop tracking.\n\
    2. Zoom: point up with one hand to zoom in to the Zoom Factor, or back out.\n\
    3. Dynamic Zoom: hold up both hands and move them apart or together to zoom.";
const GESTURE_NOTICE: &str = "Gesture turned on. If people in the shot often use their hands \
    while talking or presenting, turn it off to avoid accidental triggers.";
const VOICE_TIPS: &str = "Say a command to the camera to control it without the app. \
    Each switch turns one command on or off; Language sets the language the camera listens for.\n\n\
    Position One/Two/Three only works once presets are stored.";
const VOICE_LOCATOR_TIPS: &str = "With Voice Locator on, \"Hi, Tiny\" also wakes a sleeping \
    camera and turns it toward your voice. It works only in landscape and not at 1080P 120.";
const VOICE_NOTICE: &str = "Voice command turned on. Where there is loud background noise, \
    or everyday speech sounds like the command phrases, turn it off to avoid accidental triggers.";
const AUDIO_MODE_TIPS: &str = "1. Pure Audio Mode: left and right channels as recorded, with \
    no processing.\n\n\
    2. Spatial Audio Mode: left and right channels with a stronger sense of direction, AI noise \
    reduction and automatic gain.\n\n\
    3. Smart Omni Mode: picks up sound from every direction and balances voices with the room, \
    for meetings with several people.\n\n\
    4. Directional Mode: picks up the front and suppresses voices and noise around it, for \
    meetings and live streams.\n\n\
    5. Dual-Directional Mode: picks up the front and back and filters the sides, for interviews \
    face to face.\n\n\
    At 1080P 120 only Smart Omni Mode is available.";
const PAIRING_TIPS: &str = "Pairing. With the Vox Wireless Mic transmitter turned off, hold its \
    button for about 6 seconds until the light flashes green quickly, and keep holding it \
    until pairing succeeds.";
const MIRROR_TIPS: &str = "Mirrors the picture left to right in the camera itself, so the \
    preview, the virtual camera and every other app see it mirrored.";
const REVERSED_TIPS: &str = "Reverses which way the camera turns left and right. With it on, \
    the gimbal turns the other way for the same joystick, gesture or tracking move. The picture \
    is not changed.";
const STATUS_LIGHT_QUESTION: &str =
    "With the status light off it can be hard to tell what the camera is doing. Turn it off?";
/// The camera's audio inputs, in the official app's order.
const AUDIO_INPUTS: [(AudioInput, &str); 2] = [
    (AudioInput::Camera, "Cam Built-in Mic"),
    (AudioInput::Vox, "Vox Wireless Mic"),
];
/// Audio Mode only offers Smart Omni at this frame rate and above.
const OMNI_ONLY_FPS: u8 = 120;
/// The sleep pictures' names, in the camera's order.
const BACKGROUNDS: [&str; SLEEP_BACKGROUNDS as usize] = ["Black", "Night", "Dusk", "Gimbly"];
const TILES_PER_ROW: usize = 4;

/// Pop-ups of the More tab.
#[derive(Default)]
pub(super) struct MoreUi {
    pub device_sleep_closed: bool,
    pub gestures_closed: bool,
    background_tips: bool,
    gesture_tips: bool,
    /// The notice after a gesture was turned on, and its "Don't show again" box.
    notice: Option<bool>,
    /// The gesture settings changed and are saved once the pointer is released.
    unsaved: bool,
    pub voice_closed: bool,
    pub audio_closed: bool,
    voice_tips: bool,
    locator_tips: bool,
    audio_mode_tips: bool,
    /// The notice after a voice command was turned on, and its "Don't show again" box.
    voice_notice: Option<bool>,
    /// Turning the microphone on (`true`) or off waits for the user's OK.
    microphone_question: Option<bool>,
    pub indicator_closed: bool,
    mirror_tips: bool,
    reversed_tips: bool,
    /// Turning the status light off waits for the user's OK.
    light_question: bool,
    /// A Vox transmitter is pairing; the dialog ends it.
    pairing: bool,
    /// Thumbnails of the Sleep Background pictures, made when first shown.
    thumbs: Option<Thumbs>,
}

struct Thumbs {
    night: TextureHandle,
    dusk: TextureHandle,
    icon: TextureHandle,
    /// One per [`crate::settings::Settings::sleep_pictures`]; `None` if the
    /// file could not be read.
    custom: Vec<Option<TextureHandle>>,
}

/// What one Sleep Background tile shows.
#[derive(Clone, Copy)]
enum Tile<'a> {
    Black,
    Picture(&'a TextureHandle),
    Icon(&'a TextureHandle),
    Missing,
    Add,
}

/// A tile clicked in the Sleep Background row.
enum TileAction {
    Camera(u8),
    Custom(usize),
    Add,
    Remove(usize),
}

fn thumbnail(ctx: &egui::Context, name: &str, image: &IconData) -> TextureHandle {
    let size = (image.width as usize, image.height as usize);
    let (rgba, w, h) = crate::sleep_pictures::shrink(&image.rgba, size, (192, 108));
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

fn custom_thumbnail(ctx: &egui::Context, path: &str) -> Option<TextureHandle> {
    // shortcut: decodes on the UI thread when the More tab is first shown;
    // move to a worker if large pictures make that noticeable.
    let picture = crate::sleep_pictures::load(path).ok()?;
    Some(thumbnail(ctx, path, &picture))
}

impl GimblyApp {
    pub(super) fn more(&mut self, ui: &mut Ui) {
        self.device_sleep(ui);
        divider(ui);
        self.gestures(ui);
        divider(ui);
        self.voice_control(ui);
        divider(ui);
        self.audio(ui);
        divider(ui);
        self.orientation(ui);
        divider(ui);
        self.indicator(ui);
        divider(ui);
        if self.more_ui.unsaved && !ui.input(|i| i.pointer.any_down()) {
            self.more_ui.unsaved = false;
            self.save_settings();
        }
        self.more_dialogs(ui.ctx());
    }

    fn device_sleep(&mut self, ui: &mut Ui) {
        let open;
        (open, _) = section_header(ui, "Device Sleep", !self.more_ui.device_sleep_closed, None);
        self.more_ui.device_sleep_closed = !open;
        if !open {
            return;
        }
        let shown = self
            .status
            .device_sleep
            .filter(|_| self.ready(Feature::DeviceSleep));
        let sleep = shown.unwrap_or(DeviceSleep {
            time: SLEEP_TIMES[1],
            background: SLEEP_BACKGROUNDS - 1,
            mirror: false,
        });
        let tip = self.gimbal_tip(Feature::DeviceSleep);
        let mut command = None;
        band(ui, CARD, more_margin(), |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_enabled_ui(shown.is_some(), |ui| {
                if plain_switch(ui, "Auto Sleep", sleep.auto_sleep()).clicked() {
                    command = Some(GimbalCommand::SetSleepTime(-sleep.time));
                }
                // Like the official app, the time is hidden while Auto Sleep is off.
                if sleep.auto_sleep() {
                    ui.horizontal(|ui| {
                        ui.set_height(30.0);
                        ui.label(egui::RichText::new("Sleep Time").size(15.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let labels = SLEEP_TIMES.map(sleep_time_label);
                            let labels = labels.each_ref().map(String::as_str);
                            let current = sleep_time_label(sleep.time);
                            if let Some(i) = dropdown(ui, &current, &labels, vec2(108.0, 30.0))
                                && let Some(&time) = SLEEP_TIMES.get(i)
                                && time != sleep.time
                            {
                                command = Some(GimbalCommand::SetSleepTime(time));
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(SLEEP_TIME_HINT)
                                .size(12.0)
                                .color(TEXT_DIM),
                        )
                        .wrap(),
                    );
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(egui::RichText::new("Sleep Background").size(15.0));
                    if help_button(ui).clicked() {
                        self.more_ui.background_tips = true;
                    }
                });
                match self.background_tiles(ui, sleep.background) {
                    Some(TileAction::Camera(index)) => {
                        self.select_sleep_picture(None);
                        if index != sleep.background {
                            command = Some(GimbalCommand::SetSleepBackground(index));
                        }
                    }
                    Some(TileAction::Custom(index)) => self.select_sleep_picture(Some(index)),
                    Some(TileAction::Add) => self.add_sleep_picture(ui.ctx()),
                    Some(TileAction::Remove(index)) => self.remove_sleep_picture(index),
                    None => {}
                }
                if plain_switch(ui, "Sleep Background Mirror", sleep.mirror).clicked() {
                    command = Some(GimbalCommand::SetSleepMirror(!sleep.mirror));
                }
            })
            .response
            .on_disabled_hover_text(&tip);
        });
        if let Some(command) = command {
            self.gimbal(command);
        }
    }

    /// The camera's four tiles, the user's pictures and an add tile, four
    /// per row, each with a thumbnail of what the video shows.
    fn background_tiles(&mut self, ui: &mut Ui, background: u8) -> Option<TileAction> {
        let ctx = ui.ctx().clone();
        let thumbs = self.more_ui.thumbs.get_or_insert_with(|| Thumbs {
            night: thumbnail(&ctx, "sleep-night", &crate::branding::sleep_picture(true)),
            dusk: thumbnail(&ctx, "sleep-dusk", &crate::branding::sleep_picture(false)),
            icon: thumbnail(&ctx, "sleep-icon", &crate::branding::window_icon()),
            custom: self
                .settings
                .sleep_pictures
                .iter()
                .map(|path| custom_thumbnail(&ctx, path))
                .collect(),
        });
        let custom = self.settings.sleep_picture;
        let mut tiles: Vec<(Tile, bool, String)> = [
            Tile::Black,
            Tile::Picture(&thumbs.night),
            Tile::Picture(&thumbs.dusk),
            Tile::Icon(&thumbs.icon),
        ]
        .into_iter()
        .zip(0u8..)
        .map(|(tile, i)| {
            let name = BACKGROUNDS[usize::from(i)].to_string();
            (tile, custom.is_none() && i == background, name)
        })
        .collect();
        for (i, (thumb, path)) in thumbs
            .custom
            .iter()
            .zip(&self.settings.sleep_pictures)
            .enumerate()
        {
            let tile = thumb.as_ref().map_or(Tile::Missing, Tile::Picture);
            let name = std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            tiles.push((tile, custom == Some(i), name));
        }
        if self.settings.sleep_pictures.len() < crate::sleep_pictures::MAX {
            tiles.push((Tile::Add, false, "Add a picture (PNG or JPEG)".into()));
        }

        let gap = 8.0;
        let count = TILES_PER_ROW as f32;
        let width = ((ui.available_width() - gap * (count - 1.0)) / count).max(20.0);
        let cameras = usize::from(SLEEP_BACKGROUNDS);
        let mut action = None;
        ui.spacing_mut().item_spacing.y = gap;
        for (row, chunk) in tiles.chunks(TILES_PER_ROW).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (column, (tile, selected, name)) in chunk.iter().enumerate() {
                    let index = row * TILES_PER_ROW + column;
                    let response = background_tile(ui, *tile, *selected, width);
                    let response = response.on_hover_text(name);
                    if response.clicked() {
                        action = Some(match (*tile, index.checked_sub(cameras)) {
                            (Tile::Add, _) => TileAction::Add,
                            (_, Some(i)) => TileAction::Custom(i),
                            (_, None) => TileAction::Camera(index as u8),
                        });
                    }
                    if let Some(i) = index.checked_sub(cameras)
                        && !matches!(tile, Tile::Add)
                    {
                        egui::Popup::context_menu(&response)
                            .width(134.0)
                            .frame(menu_frame())
                            .show(|ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                if menu_entry(ui, "Remove").clicked() {
                                    action = Some(TileAction::Remove(i));
                                    ui.close();
                                }
                            });
                    }
                }
            });
        }
        action
    }

    /// Shows the user's picture `index` while the camera sleeps, or the
    /// camera's tile again for `None`.
    pub(super) fn select_sleep_picture(&mut self, index: Option<usize>) {
        let path = index.and_then(|i| self.settings.sleep_pictures.get(i));
        let picture = match path.map(|path| crate::sleep_pictures::load(path)) {
            Some(Ok(picture)) => Some(picture),
            Some(Err(error)) => {
                self.notify(error);
                return;
            }
            None => None,
        };
        crate::camera::set_custom_sleep_picture(picture);
        if self.settings.sleep_picture != index {
            self.settings.sleep_picture = index;
            self.save_settings();
        }
    }

    fn add_sleep_picture(&mut self, ctx: &egui::Context) {
        let start = self
            .settings
            .sleep_pictures
            .last()
            .and_then(|p| std::path::Path::new(p).parent())
            .and_then(|p| p.to_str())
            .unwrap_or(&self.settings.media_dir)
            .to_string();
        let Some(path) = crate::os::pick_image(&start) else {
            return;
        };
        let picture: Arc<IconData> = match crate::sleep_pictures::load(&path) {
            Ok(picture) => picture,
            Err(error) => {
                self.notify(error);
                return;
            }
        };
        if let Some(thumbs) = &mut self.more_ui.thumbs {
            thumbs.custom.push(Some(thumbnail(ctx, &path, &picture)));
        }
        self.settings.sleep_pictures.push(path);
        self.settings.sleep_picture = Some(self.settings.sleep_pictures.len() - 1);
        crate::camera::set_custom_sleep_picture(Some(picture));
        self.save_settings();
    }

    fn remove_sleep_picture(&mut self, index: usize) {
        if index >= self.settings.sleep_pictures.len() {
            return;
        }
        self.settings.sleep_pictures.remove(index);
        if let Some(thumbs) = &mut self.more_ui.thumbs
            && index < thumbs.custom.len()
        {
            thumbs.custom.remove(index);
        }
        match self.settings.sleep_picture {
            Some(i) if i == index => {
                self.settings.sleep_picture = None;
                crate::camera::set_custom_sleep_picture(None);
            }
            Some(i) if i > index => self.settings.sleep_picture = Some(i - 1),
            _ => {}
        }
        self.save_settings();
    }

    fn gestures(&mut self, ui: &mut Ui) {
        let (open, help) =
            section_header_help(ui, "Gesture Control", !self.more_ui.gestures_closed);
        self.more_ui.gestures_closed = !open;
        if help {
            self.more_ui.gesture_tips = true;
        }
        if !open {
            return;
        }
        let ready = self.ready(Feature::Gestures);
        let locked = self.status.locked_target.filter(|_| ready);
        let tip = self.gimbal_tip(Feature::Gestures);
        let mut gestures = self.settings.gestures;
        let mut changed = None;
        let mut factor = None;
        band(ui, CARD, more_margin(), |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 18.0);
            ui.add_enabled_ui(ready && locked.is_some(), |ui| {
                let rows = [
                    (
                        Gesture::LockedTarget,
                        icon::HAND_PALM,
                        "Locked Target",
                        "Turn tracking on or off",
                        locked.unwrap_or(false),
                    ),
                    (
                        Gesture::Zoom,
                        icon::HAND_POINTING,
                        "Zoom",
                        "Zoom in or out",
                        gestures.zoom,
                    ),
                ];
                for (gesture, glyph, title, hint, on) in rows {
                    if gesture_row(ui, glyph, title, hint, on).clicked() {
                        changed = Some((gesture, !on));
                    }
                }
                let ratio = gestures.zoom_factor;
                let (dragged, input) =
                    slider_row(ui, "Zoom Factor", zoom_t(ratio), &format!("{ratio:.2} x"));
                factor = dragged
                    .map(|t| GESTURE_ZOOM.start() + t * (GESTURE_ZOOM.end() - GESTURE_ZOOM.start()))
                    .or_else(|| match input? {
                        NumberInput::Step(step) => Some(ratio + step.signum() * 0.01),
                        NumberInput::Typed(text) => parse_zoom(&text),
                    })
                    .map(|r| {
                        (r.clamp(*GESTURE_ZOOM.start(), *GESTURE_ZOOM.end()) * 100.0).round()
                            / 100.0
                    })
                    .filter(|&r| r != ratio);
                let dynamic = gestures.dynamic_zoom;
                if gesture_row(
                    ui,
                    icon::HANDS_CLAPPING,
                    "Dynamic Zoom",
                    "Zoom in or out with both hands",
                    dynamic,
                )
                .clicked()
                {
                    changed = Some((Gesture::DynamicZoom, !dynamic));
                }
                let flip = gestures.direction_flip;
                if plain_switch(ui, "Direction Flip", flip).clicked() {
                    changed = Some((Gesture::DirectionFlip, !flip));
                }
            })
            .response
            .on_disabled_hover_text(&tip);
        });
        if let Some(ratio) = factor {
            gestures.zoom_factor = ratio;
            self.gimbal(GimbalCommand::SetGestureZoom(ratio));
            // The official app zooms the camera to the factor to show it.
            self.set_zoom_ratio(ratio);
        }
        if let Some((gesture, on)) = changed {
            match gesture {
                Gesture::LockedTarget => {}
                Gesture::Zoom => gestures.zoom = on,
                Gesture::DynamicZoom => gestures.dynamic_zoom = on,
                Gesture::DirectionFlip => gestures.direction_flip = on,
            }
            self.gimbal(GimbalCommand::SetGesture(gesture, on));
            // The notice follows the three hand gestures, not Direction Flip.
            if on && gesture != Gesture::DirectionFlip && !gestures.notice_off {
                self.more_ui.notice = Some(false);
            }
        }
        if gestures != self.settings.gestures {
            self.settings.gestures = gestures;
            self.more_ui.unsaved = true;
        }
    }

    fn voice_control(&mut self, ui: &mut Ui) {
        let (open, help) = section_header_help(ui, "Voice Control", !self.more_ui.voice_closed);
        self.more_ui.voice_closed = !open;
        if help {
            self.more_ui.voice_tips = true;
        }
        if !open {
            return;
        }
        let shown = self
            .status
            .voice_control
            .filter(|_| self.ready(Feature::VoiceControl));
        let voice = shown.unwrap_or(VoiceControl {
            english: true,
            commands: 0x7f,
            locator: true,
            zoom: 0,
        });
        let tip = self.gimbal_tip(Feature::VoiceControl);
        let mut command = None;
        band(ui, CARD, more_margin(), |ui| {
            ui.push_id("voice-control", |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
                ui.add_enabled_ui(shown.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(30.0);
                        ui.label(egui::RichText::new("Language").size(15.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            // Gimbly's fonts have no Chinese glyphs, so the names are in English.
                            let labels = ["Chinese", "English"];
                            let current = labels[usize::from(voice.english)];
                            if let Some(i) = dropdown(ui, current, &labels, vec2(108.0, 30.0))
                                && (i == 1) != voice.english
                            {
                                command = Some(GimbalCommand::SetVoiceEnglish(i == 1));
                            }
                        });
                    });
                    for phrase in VoiceCommand::ALL {
                        let on = voice.on(phrase);
                        if plain_switch(ui, voice_label(phrase), on).clicked() {
                            command = Some(GimbalCommand::SetVoiceCommand(phrase, !on));
                        }
                        // Like the official app, Voice Locator shows only while "Hi, Tiny" is on.
                        if phrase == VoiceCommand::HiTiny && on {
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if help_button(ui).clicked() {
                                        self.more_ui.locator_tips = true;
                                    }
                                    let mut locator = voice.locator;
                                    if ui.checkbox(&mut locator, "Voice Locator").changed() {
                                        command = Some(GimbalCommand::SetVoiceLocator(locator));
                                    }
                                });
                            });
                        }
                        if phrase == VoiceCommand::ZoomOut {
                            let (dragged, input) = slider_row(
                                ui,
                                "Zoom Factor",
                                f32::from(voice.zoom) / f32::from(VOICE_ZOOM_MAX),
                                &format!("{:.2} x", voice.zoom_ratio()),
                            );
                            let steps = dragged
                                .map(|t| {
                                    (t.clamp(0.0, 1.0) * f32::from(VOICE_ZOOM_MAX)).round() as u8
                                })
                                .or_else(|| match input? {
                                    NumberInput::Step(step) if step > 0.0 => {
                                        Some(voice.zoom.saturating_add(1).min(VOICE_ZOOM_MAX))
                                    }
                                    NumberInput::Step(_) => Some(voice.zoom.saturating_sub(1)),
                                    NumberInput::Typed(text) => {
                                        parse_zoom(&text).map(voice_zoom_steps)
                                    }
                                })
                                .filter(|&steps| steps != voice.zoom);
                            if let Some(steps) = steps {
                                command = Some(GimbalCommand::SetVoiceZoom(steps));
                            }
                        }
                    }
                })
                .response
                .on_disabled_hover_text(&tip);
            });
        });
        if let Some(command) = command {
            if matches!(command, GimbalCommand::SetVoiceCommand(_, true))
                && !self.settings.voice_notice_off
            {
                self.more_ui.voice_notice = Some(false);
            }
            self.gimbal(command);
        }
    }

    fn audio(&mut self, ui: &mut Ui) {
        let open;
        (open, _) = section_header(ui, "Audio", !self.more_ui.audio_closed, None);
        self.more_ui.audio_closed = !open;
        if !open {
            return;
        }
        let shown = self.status.audio.filter(|_| self.ready(Feature::Audio));
        let audio = shown.unwrap_or(Audio {
            input: AudioInput::Camera,
            mode: AudioMode::Spatial,
            sleep_mic: true,
            microphone: true,
        });
        let omni_only = self
            .status
            .frame_rate
            .is_some_and(|fps| fps >= OMNI_ONLY_FPS);
        let tip = self.gimbal_tip(Feature::Audio);
        let mut command = None;
        band(ui, CARD, more_margin(), |ui| {
            ui.push_id("audio", |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
                ui.add_enabled_ui(shown.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(30.0);
                        ui.label(egui::RichText::new("Audio Input").size(15.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let labels = AUDIO_INPUTS.map(|(_, label)| label);
                            let current = AUDIO_INPUTS
                                .iter()
                                .find(|(input, _)| *input == audio.input)
                                .map_or("", |(_, label)| label);
                            if let Some(i) = dropdown(ui, current, &labels, vec2(150.0, 30.0))
                                && let Some(&(input, _)) = AUDIO_INPUTS.get(i)
                                && input != audio.input
                            {
                                command = Some(GimbalCommand::SetAudioInput(input));
                            }
                        });
                    });
                    match audio.input {
                        AudioInput::Camera => {
                            ui.horizontal(|ui| {
                                ui.set_height(30.0);
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.label(egui::RichText::new("Audio Mode").size(15.0));
                                if help_button(ui).clicked() {
                                    self.more_ui.audio_mode_tips = true;
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let modes: Vec<AudioMode> = AudioMode::ALL
                                        .into_iter()
                                        .filter(|&m| !omni_only || m == AudioMode::SmartOmni)
                                        .collect();
                                    let labels: Vec<&str> =
                                        modes.iter().map(|m| m.name()).collect();
                                    if let Some(i) =
                                        dropdown(ui, audio.mode.name(), &labels, vec2(170.0, 30.0))
                                        && let Some(&mode) = modes.get(i)
                                        && mode != audio.mode
                                    {
                                        command = Some(GimbalCommand::SetAudioMode(mode));
                                    }
                                });
                            });
                            let label = "Microphone Status During Sleep";
                            if plain_switch(ui, label, audio.sleep_mic).clicked() {
                                command = Some(GimbalCommand::SetSleepMic(!audio.sleep_mic));
                            }
                            if plain_switch(ui, "Disable Microphone", !audio.microphone).clicked() {
                                self.more_ui.microphone_question = Some(!audio.microphone);
                            }
                        }
                        AudioInput::Vox => {
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 8.0) / 2.0;
                                for tx in 1..=2u8 {
                                    let label = format!("{} AddTX{tx}", icon::PLUS);
                                    if pill(ui, &label, false, vec2(width, 34.0)).clicked() {
                                        command = Some(GimbalCommand::PairVox(Some(tx)));
                                        self.more_ui.pairing = true;
                                    }
                                }
                            });
                            // Shown greyed out, as in the official app.
                            ui.add_enabled_ui(false, |ui| {
                                plain_switch(ui, "Microphone Status During Sleep", false);
                                ui.horizontal(|ui| {
                                    ui.set_height(30.0);
                                    ui.label(egui::RichText::new("Noise Reduction").size(15.0));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        dropdown(ui, "Off", &["Off"], vec2(90.0, 30.0));
                                    });
                                });
                            });
                        }
                    }
                })
                .response
                .on_disabled_hover_text(&tip);
            });
        });
        if let Some(command) = command {
            self.gimbal(command);
        }
    }

    /// Mirror Image and View and Gimbal Reversed: two rows without a header,
    /// as in the official app.
    fn orientation(&mut self, ui: &mut Ui) {
        let mirror = self.status.mirror.filter(|_| self.ready(Feature::Mirror));
        let reversed = self
            .status
            .reversed
            .filter(|_| self.ready(Feature::Reversed));
        let rows = [
            (Feature::Mirror, "Mirror Image", mirror),
            (Feature::Reversed, "View and Gimbal Reversed", reversed),
        ];
        let tips = rows.map(|(feature, ..)| self.gimbal_tip(feature));
        let mut command = None;
        let mut help = None;
        band(ui, CARD, more_margin(), |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_space(14.0);
            for ((feature, label, on), tip) in rows.into_iter().zip(&tips) {
                ui.horizontal(|ui| {
                    ui.set_height(24.0);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(egui::RichText::new(label).size(15.0));
                    if help_button(ui).clicked() {
                        help = Some(feature);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let switch = ui
                            .add_enabled_ui(on.is_some(), |ui| toggle(ui, on.unwrap_or(false)))
                            .inner
                            .on_disabled_hover_text(tip);
                        if switch.clicked()
                            && let Some(on) = on
                        {
                            command = Some(match feature {
                                Feature::Mirror => GimbalCommand::SetMirror(!on),
                                _ => GimbalCommand::SetReversed(!on),
                            });
                        }
                    });
                });
            }
        });
        match help {
            Some(Feature::Mirror) => self.more_ui.mirror_tips = true,
            Some(_) => self.more_ui.reversed_tips = true,
            None => {}
        }
        if let Some(command) = command {
            self.gimbal(command);
        }
    }

    fn indicator(&mut self, ui: &mut Ui) {
        let open;
        (open, _) = section_header(ui, "Indicator Status", !self.more_ui.indicator_closed, None);
        self.more_ui.indicator_closed = !open;
        if !open {
            return;
        }
        let shown = self
            .status
            .status_light
            .filter(|_| self.ready(Feature::StatusLight));
        let light = shown.unwrap_or(StatusLight {
            on: true,
            brightness: STATUS_LIGHT_MAX,
        });
        let tip = self.gimbal_tip(Feature::StatusLight);
        let mut command = None;
        let mut question = false;
        band(ui, CARD, more_margin(), |ui| {
            ui.push_id("indicator", |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
                ui.add_enabled_ui(shown.is_some(), |ui| {
                    if plain_switch(ui, "Status Light", light.on).clicked() {
                        if light.on {
                            question = true;
                        } else {
                            command = Some(GimbalCommand::SetStatusLight(light.brightness));
                        }
                    }
                    // Like the official app, Brightness is greyed while the light is off.
                    ui.add_enabled_ui(light.on, |ui| {
                        let brightness = light.brightness;
                        let (dragged, input) = slider_row(
                            ui,
                            "Brightness",
                            brightness_t(brightness),
                            &brightness.to_string(),
                        );
                        let new = dragged
                            .map(brightness_from_t)
                            .or_else(|| match input? {
                                NumberInput::Step(step) if step > 0.0 => {
                                    Some((brightness + 1).min(STATUS_LIGHT_MAX))
                                }
                                NumberInput::Step(_) => Some(brightness.saturating_sub(1).max(1)),
                                NumberInput::Typed(text) => parse_brightness(&text),
                            })
                            .filter(|&b| b != brightness);
                        if let Some(b) = new {
                            command = Some(GimbalCommand::SetStatusLight(b));
                        }
                    });
                })
                .response
                .on_disabled_hover_text(&tip);
            });
        });
        if question {
            self.more_ui.light_question = true;
        }
        if let Some(command) = command {
            self.gimbal(command);
        }
    }

    fn more_dialogs(&mut self, ctx: &egui::Context) {
        if self.more_ui.mirror_tips && tips(ctx, "mirror-tips", MIRROR_TIPS, "OK").is_some() {
            self.more_ui.mirror_tips = false;
        }
        if self.more_ui.reversed_tips && tips(ctx, "reversed-tips", REVERSED_TIPS, "OK").is_some() {
            self.more_ui.reversed_tips = false;
        }
        if self.more_ui.light_question
            && let Some(answer) = question(ctx, "status-light-question", STATUS_LIGHT_QUESTION)
        {
            self.more_ui.light_question = false;
            if answer {
                self.gimbal(GimbalCommand::SetStatusLight(0));
            }
        }
        if self.more_ui.background_tips
            && tips(ctx, "sleep-background-tips", SLEEP_BACKGROUND_TIPS, "OK").is_some()
        {
            self.more_ui.background_tips = false;
        }
        if self.more_ui.gesture_tips && tips(ctx, "gesture-tips", GESTURE_TIPS, "OK").is_some() {
            self.more_ui.gesture_tips = false;
        }
        if self.more_ui.voice_tips && tips(ctx, "voice-tips", VOICE_TIPS, "OK").is_some() {
            self.more_ui.voice_tips = false;
        }
        if self.more_ui.locator_tips
            && tips(ctx, "voice-locator-tips", VOICE_LOCATOR_TIPS, "Got it").is_some()
        {
            self.more_ui.locator_tips = false;
        }
        if self.more_ui.audio_mode_tips
            && tips(ctx, "audio-mode-tips", AUDIO_MODE_TIPS, "Got it").is_some()
        {
            self.more_ui.audio_mode_tips = false;
        }
        if self.more_ui.pairing && tips(ctx, "vox-pairing", PAIRING_TIPS, "End Pairing").is_some() {
            self.more_ui.pairing = false;
            self.gimbal(GimbalCommand::PairVox(None));
        }
        if let Some(on) = self.more_ui.microphone_question {
            let text = if on {
                "The camera restarts to turn the microphone on. Turn it on?"
            } else {
                "The camera restarts to turn the microphone off. Turn it off?"
            };
            if let Some(answer) = question(ctx, "microphone-question", text) {
                self.more_ui.microphone_question = None;
                if answer {
                    self.gimbal(GimbalCommand::SetMicrophone(on));
                    self.camera_restart =
                        Some(super::CameraRestart::new(std::time::Instant::now()));
                }
            }
        }
        if let Some(mut never) = self.more_ui.voice_notice {
            let done = notice(
                ctx,
                "voice-notice",
                "Voice Command On",
                VOICE_NOTICE,
                &mut never,
            );
            self.more_ui.voice_notice = Some(never);
            if done {
                self.more_ui.voice_notice = None;
                if never {
                    self.settings.voice_notice_off = true;
                    self.save_settings();
                }
            }
        }
        if let Some(mut never) = self.more_ui.notice {
            let done = notice(
                ctx,
                "gesture-notice",
                "Gesture On",
                GESTURE_NOTICE,
                &mut never,
            );
            self.more_ui.notice = Some(never);
            if done {
                self.more_ui.notice = None;
                if never {
                    self.settings.gestures.notice_off = true;
                    self.save_settings();
                }
            }
        }
    }
}

fn more_margin() -> egui::Margin {
    egui::Margin {
        left: PADDING as i8,
        right: PADDING as i8,
        top: 4,
        bottom: 18,
    }
}

fn sleep_time_label(seconds: i16) -> String {
    let seconds = seconds.unsigned_abs();
    if seconds.is_multiple_of(60) {
        format!("{}min", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn brightness_t(brightness: u8) -> f32 {
    f32::from(brightness.clamp(1, STATUS_LIGHT_MAX) - 1) / f32::from(STATUS_LIGHT_MAX - 1)
}

fn brightness_from_t(t: f32) -> u8 {
    let max = f32::from(STATUS_LIGHT_MAX - 1);
    1 + (t.clamp(0.0, 1.0) * max).round() as u8
}

/// A typed Brightness; outside 1 to 3 it is refused, as in the official app.
fn parse_brightness(text: &str) -> Option<u8> {
    text.trim()
        .parse()
        .ok()
        .filter(|b| (1..=STATUS_LIGHT_MAX).contains(b))
}

fn zoom_t(ratio: f32) -> f32 {
    (ratio - GESTURE_ZOOM.start()) / (GESTURE_ZOOM.end() - GESTURE_ZOOM.start())
}

/// A typed Zoom Factor such as `2.5` or `2.5x`; outside 1 to 4 it is
/// refused, as in the official app. Extra decimals are rounded.
fn parse_zoom(text: &str) -> Option<f32> {
    let text = text.trim().trim_end_matches(['x', 'X']).trim();
    let ratio: f32 = text.parse().ok()?;
    GESTURE_ZOOM
        .contains(&ratio)
        .then(|| (ratio * 100.0).round() / 100.0)
}

/// One Sleep Background tile with its thumbnail.
fn background_tile(ui: &mut Ui, tile: Tile, selected: bool, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let painter = ui.painter();
    let inner = rect.shrink(3.0);
    let fill = if matches!(tile, Tile::Add | Tile::Missing) {
        FIELD
    } else {
        Color32::BLACK
    };
    painter.rect_filled(inner, CornerRadius::same(3), fill);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match tile {
        Tile::Black => {}
        Tile::Picture(texture) => {
            let shown = crate::preview_view::fit(texture.size_vec2(), inner);
            painter.image(texture.id(), shown, uv, Color32::WHITE);
        }
        Tile::Icon(texture) => {
            let shown =
                Rect::from_center_size(inner.center(), egui::Vec2::splat(inner.height() * 0.6));
            painter.image(texture.id(), shown, uv, Color32::WHITE);
        }
        Tile::Missing | Tile::Add => {
            let glyph = if matches!(tile, Tile::Add) {
                icon::PLUS
            } else {
                icon::IMAGE_BROKEN
            };
            painter.text(
                inner.center(),
                Align2::CENTER_CENTER,
                glyph,
                font(16.0),
                TEXT_DIM,
            );
        }
    }
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.5, ACCENT),
            StrokeKind::Inside,
        );
    } else if response.hovered() && ui.is_enabled() {
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, FIELD_STROKE),
            StrokeKind::Inside,
        );
    }
    response
}

/// A label and a switch, without the help icon of `switch_row`; returns the switch.
fn plain_switch(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_height(24.0);
        ui.label(egui::RichText::new(label).size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| toggle(ui, on))
            .inner
    })
    .inner
}

/// A gesture with its icon tile, title, hint and switch; returns the switch.
fn gesture_row(ui: &mut Ui, glyph: &str, title: &str, hint: &str, on: bool) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter().rect_filled(tile, CornerRadius::same(4), FIELD);
        ui.painter().text(
            tile.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(20.0),
            TEXT,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(egui::RichText::new(title).size(15.0));
            ui.label(egui::RichText::new(hint).size(12.0).color(TEXT_DIM));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| toggle(ui, on))
            .inner
    })
    .inner
}

/// The phrase the user says for a voice command.
fn voice_label(command: VoiceCommand) -> &'static str {
    match command {
        VoiceCommand::HiTiny => "Hi, Tiny",
        VoiceCommand::SleepTiny => "Sleep, Tiny",
        VoiceCommand::TrackMe => "Track Me",
        VoiceCommand::UnlockMe => "Unlock Me",
        VoiceCommand::ZoomIn => "Zoom in Closer",
        VoiceCommand::ZoomOut => "Zoom out Further",
        VoiceCommand::Positions => "Position One/Two/Three",
    }
}

/// A typed voice Zoom Factor in steps of [`VOICE_ZOOM_STEP`], rounded down
/// as the camera stores it (2.00 becomes 1.99).
fn voice_zoom_steps(ratio: f32) -> u8 {
    let steps = ((ratio - 1.0) / VOICE_ZOOM_STEP + 1e-3).floor();
    steps.clamp(0.0, f32::from(VOICE_ZOOM_MAX)) as u8
}

/// The notice after a gesture or voice command is turned on. Returns true
/// once dismissed; `never` is the "Don't show again" box.
fn notice(ctx: &egui::Context, id: &str, title: &str, text: &str, never: &mut bool) -> bool {
    let mut done = false;
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
                ui.label(egui::RichText::new(title).size(15.0).color(TEXT));
            });
            ui.label(egui::RichText::new(text).size(13.0).color(TEXT));
            ui.checkbox(never, "Don't show again");
            ui.vertical_centered(|ui| {
                if pill(ui, "Got it", false, vec2(85.0, 31.0)).clicked() {
                    done = true;
                }
            });
        });
    done || modal.should_close()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_times_read_like_the_official_app() {
        let labels: Vec<_> = SLEEP_TIMES.into_iter().map(sleep_time_label).collect();
        assert_eq!(labels, ["30s", "2min", "10min"]);
        // Auto Sleep off keeps the time as a negative number.
        assert_eq!(sleep_time_label(-600), "10min");
        assert_eq!(sleep_time_label(i16::MIN), "32768s");
    }

    #[test]
    fn typed_zoom_factors_stay_in_range() {
        assert_eq!(parse_zoom("2.5"), Some(2.5));
        assert_eq!(parse_zoom(" 3 x"), Some(3.0));
        assert_eq!(parse_zoom("1.234"), Some(1.23));
        assert_eq!(parse_zoom("4"), Some(4.0));
        assert_eq!(parse_zoom("5"), None);
        assert_eq!(parse_zoom("0.5"), None);
        assert_eq!(parse_zoom("NaN"), None);
        assert_eq!(parse_zoom("abc"), None);
        assert_eq!(zoom_t(1.0), 0.0);
        assert_eq!(zoom_t(4.0), 1.0);
    }

    #[test]
    fn brightness_steps_like_the_official_app() {
        assert_eq!(brightness_t(1), 0.0);
        assert_eq!(brightness_t(3), 1.0);
        assert_eq!(brightness_t(0), 0.0);
        assert_eq!(brightness_from_t(0.0), 1);
        assert_eq!(brightness_from_t(0.5), 2);
        assert_eq!(brightness_from_t(1.0), 3);
        assert_eq!(brightness_from_t(f32::NAN), 1);
        assert_eq!(brightness_from_t(-4.0), 1);
        // Typed 5 and 0 were refused in the official app.
        assert_eq!(parse_brightness(" 2 "), Some(2));
        assert_eq!(parse_brightness("5"), None);
        assert_eq!(parse_brightness("0"), None);
        assert_eq!(parse_brightness("x"), None);
    }

    #[test]
    fn typed_voice_zoom_rounds_down_like_the_camera() {
        // Typed values and the steps the official app sent (2026-11-28 capture).
        assert_eq!(voice_zoom_steps(1.0), 0);
        assert_eq!(voice_zoom_steps(1.23), 7);
        assert_eq!(voice_zoom_steps(2.0), 33);
        assert_eq!(voice_zoom_steps(2.5), 50);
        assert_eq!(voice_zoom_steps(4.0), VOICE_ZOOM_MAX);
        assert_eq!(voice_zoom_steps(9.0), VOICE_ZOOM_MAX);
        assert_eq!(voice_zoom_steps(f32::NAN), 0);
        assert_eq!(
            voice_label(VoiceCommand::Positions),
            "Position One/Two/Three"
        );
    }
}
