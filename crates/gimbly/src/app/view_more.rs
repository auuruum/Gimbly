//! The More tab: Device Sleep and Gesture Control.

use super::view_exposure::slider_row;
use super::{GimblyApp, switch_row, tips};
use crate::camera::{
    DeviceSleep, Feature, GESTURE_ZOOM, Gesture, GimbalCommand, SLEEP_BACKGROUNDS, SLEEP_TIMES,
};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Sense, Stroke, StrokeKind, Ui, vec2,
};
use egui_phosphor::regular as icon;

const SLEEP_TIME_HINT: &str =
    "While no app is using the video, the camera goes to sleep after this time.";
const SLEEP_BACKGROUND_TIPS: &str = "What the camera sends while it sleeps: black frames or one \
    of the pictures stored in the camera. Gimbly cannot add your own pictures or videos yet.";
const GESTURE_TIPS: &str = "Show a gesture to the camera to control it without the app.\n\n\
    1. Locked Target: hold up an open palm to start or stop tracking.\n\
    2. Zoom: point up with one hand to zoom in to the Zoom Factor, or back out.\n\
    3. Dynamic Zoom: hold up both hands and move them apart or together to zoom.";
const GESTURE_NOTICE: &str = "Gesture turned on. If people in the shot often use their hands \
    while talking or presenting, turn it off to avoid accidental triggers.";
/// The sleep pictures' names, in the camera's order.
const BACKGROUNDS: [(&str, &str); SLEEP_BACKGROUNDS as usize] = [
    ("Black", ""),
    ("Picture 1", icon::IMAGE),
    ("Animation", icon::FILM_STRIP),
    ("Picture 2", icon::IMAGE_SQUARE),
];

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
                ui.horizontal(|ui| {
                    ui.set_height(24.0);
                    ui.label(egui::RichText::new("Auto Sleep").size(15.0));
                    let switch = ui
                        .with_layout(Layout::right_to_left(Align::Center), |ui| {
                            toggle(ui, sleep.auto_sleep())
                        })
                        .inner;
                    if switch.clicked() {
                        command = Some(GimbalCommand::SetSleepTime(-sleep.time));
                    }
                });
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
                if let Some(index) = background_tiles(ui, sleep.background)
                    && index != sleep.background
                {
                    command = Some(GimbalCommand::SetSleepBackground(index));
                }
                ui.horizontal(|ui| {
                    ui.set_height(24.0);
                    ui.label(egui::RichText::new("Sleep Background Mirror").size(15.0));
                    let switch = ui
                        .with_layout(Layout::right_to_left(Align::Center), |ui| {
                            toggle(ui, sleep.mirror)
                        })
                        .inner;
                    if switch.clicked() {
                        command = Some(GimbalCommand::SetSleepMirror(!sleep.mirror));
                    }
                });
            })
            .response
            .on_disabled_hover_text(&tip);
        });
        if let Some(command) = command {
            self.gimbal(command);
        }
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
                let (switch, _) = switch_row(
                    ui,
                    "Direction Flip",
                    "Swaps left and right for the gestures",
                    None,
                    flip,
                );
                if switch.clicked() {
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

/// The four sleep pictures; returns the one clicked. Gimbly draws its own
/// stand-ins, not the camera's pictures.
fn background_tiles(ui: &mut Ui, selected: u8) -> Option<u8> {
    let gap = 8.0;
    let count = f32::from(SLEEP_BACKGROUNDS);
    let width = ((ui.available_width() - gap * (count - 1.0)) / count).max(20.0);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (index, (name, glyph)) in (0..SLEEP_BACKGROUNDS).zip(BACKGROUNDS) {
            let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
            let fill = if index == 0 { Color32::BLACK } else { FIELD };
            let painter = ui.painter();
            painter.rect_filled(rect.shrink(3.0), CornerRadius::same(3), fill);
            if !glyph.is_empty() {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    glyph,
                    font(16.0),
                    TEXT_DIM,
                );
            }
            if index == selected {
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
            if response.on_hover_text(name).clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
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
