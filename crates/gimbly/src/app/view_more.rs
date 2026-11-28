//! The More tab: Device Sleep and Gesture Control.

use super::view_exposure::slider_row;
use super::view_gimbal::{menu_entry, menu_frame};
use super::{GimblyApp, tips};
use crate::camera::{
    DeviceSleep, Feature, GESTURE_ZOOM, Gesture, GimbalCommand, SLEEP_BACKGROUNDS, SLEEP_TIMES,
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

    fn more_dialogs(&mut self, ctx: &egui::Context) {
        if self.more_ui.background_tips
            && tips(ctx, "sleep-background-tips", SLEEP_BACKGROUND_TIPS, "OK").is_some()
        {
            self.more_ui.background_tips = false;
        }
        if self.more_ui.gesture_tips && tips(ctx, "gesture-tips", GESTURE_TIPS, "OK").is_some() {
            self.more_ui.gesture_tips = false;
        }
        if let Some(mut never) = self.more_ui.notice {
            let done = notice(ctx, &mut never);
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

/// The notice after a gesture is turned on. Returns true once dismissed;
/// `never` is the "Don't show again" box.
fn notice(ctx: &egui::Context, never: &mut bool) -> bool {
    let mut done = false;
    let modal = egui::Modal::new(egui::Id::new("gesture-notice"))
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
                ui.label(egui::RichText::new("Gesture On").size(15.0).color(TEXT));
            });
            ui.label(egui::RichText::new(GESTURE_NOTICE).size(13.0).color(TEXT));
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
}
