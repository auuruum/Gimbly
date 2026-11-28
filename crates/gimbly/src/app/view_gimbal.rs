//! The Console sections that drive the gimbal: AI Mode, Preset and View and
//! Gimbal, plus the rename dialog and the Hand Tracking and Zone Tracking
//! panels.
//!
//! Layout and behaviour follow OBSBOT Center's panel for the Tiny 3 Lite; the code is
//! Gimbly's own.

use super::{GimblyApp, NOT_YET};
use crate::camera::{
    AiMode, Command, Feature, Fov, GimbalCommand, Hand, PRESET_NAME_MAX, PRESET_SLOTS, Preset,
    RangeField, SessionState, TrackingRange, VoiceRange, ZONE_AREAS, ZOOM_RATIO_MAX,
};
use crate::gimbal::{self, PanelPull, SpeedMode};
use crate::joystick::{self, Drag, DragState};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

/// How long the zoom controls keep showing the value the user picked while
/// the camera zooms towards it.
const ZOOM_HOLD: Duration = Duration::from_millis(1500);
/// Zoom hotkeys and the spin box arrows change the ratio by this much.
pub const ZOOM_STEP: f32 = 0.1;

const SIDE_TAB: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x2F);

/// UI state of the gimbal sections.
pub struct GimbalUi {
    pub ai_mode_open: bool,
    pub preset_open: bool,
    pub view_open: bool,
    /// Coarse (normal) or Fine (a third of the speed) joystick.
    pub mode: SpeedMode,
    /// The Gimbal Speed side panel is shown.
    pub speed_panel: bool,
    /// Gimbal Speed slider, 1 (slow) to 5 (fast).
    pub speed_level: u8,
    joystick: DragState,
    /// The panel joystick's pull curve state.
    pull: PanelPull,
    /// Zoom ratio the user picked and when, shown until the camera catches up.
    zoom_target: Option<(f32, Instant)>,
    /// Preset or area being renamed and the text typed so far.
    rename: Option<(Renaming, String)>,
    /// The Hand Tracking range panel replaces the console.
    pub hand_range_open: bool,
    /// The Zone Tracking panel replaces the console.
    pub zone_open: bool,
    /// The Zone Tracking panel shows its Non-tracking Area tab.
    zone_areas: bool,
}

impl Default for GimbalUi {
    fn default() -> Self {
        Self {
            ai_mode_open: true,
            preset_open: true,
            view_open: true,
            mode: SpeedMode::Coarse,
            speed_panel: false,
            speed_level: gimbal::SPEED_LEVEL_MAX,
            joystick: DragState::default(),
            pull: PanelPull::default(),
            zoom_target: None,
            rename: None,
            hand_range_open: false,
            zone_open: false,
            zone_areas: false,
        }
    }
}

impl GimbalUi {
    /// Forget per-camera state when another camera is selected.
    pub fn reset_camera(&mut self) {
        self.zoom_target = None;
        self.rename = None;
        self.hand_range_open = false;
        self.zone_open = false;
    }
}

/// Header row shared by the gimbal sections: title, optional help, an
/// optional extra control and the collapse caret. Returns the new open state.
fn header(
    ui: &mut Ui,
    title: &str,
    help_tip: Option<&str>,
    open: bool,
    extra: impl FnOnce(&mut Ui),
) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 60.0), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, CARD);
    let title_rect = ui.painter().text(
        pos2(rect.left() + PADDING, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        font(16.0),
        TEXT,
    );
    if let Some(tip) = help_tip {
        let at = Rect::from_min_size(
            pos2(title_rect.right() + 6.0, rect.center().y - 9.0),
            vec2(16.0, 18.0),
        );
        place(ui, at, Layout::default(), (title, "help"), |ui| {
            help(ui, tip)
        });
    }
    let caret = if open {
        icon::CARET_UP
    } else {
        icon::CARET_DOWN
    };
    ui.painter().text(
        pos2(rect.right() - PADDING - 10.0, rect.center().y),
        Align2::CENTER_CENTER,
        caret,
        font(18.0),
        TEXT,
    );
    // The extra control sits left of the caret, like the Coarse button.
    let extra_rect = Rect::from_min_max(
        pos2(rect.left() + 200.0, rect.top()),
        pos2(rect.right() - PADDING - 26.0, rect.bottom()),
    );
    place(
        ui,
        extra_rect,
        Layout::right_to_left(Align::Center),
        (title, "extra"),
        extra,
    );
    if response.clicked() { !open } else { open }
}

impl GimblyApp {
    pub(super) fn gimbal_ready(&self) -> bool {
        self.status.vendor_control && self.status.state == SessionState::Streaming
    }

    /// The camera is ready and `feature` was checked on its model.
    pub(super) fn ready(&self, feature: Feature) -> bool {
        self.gimbal_ready() && self.supports(feature)
    }

    pub(super) fn supports(&self, feature: Feature) -> bool {
        self.status.model.is_some_and(|m| m.supports(feature))
    }

    /// Why `feature` is unavailable, for its disabled controls.
    pub(super) fn gimbal_tip(&self, feature: Feature) -> String {
        match self.status.model {
            _ if self.status.vendor_control_untested => {
                "Gimbal control is not verified on this camera model yet".into()
            }
            Some(model) if !model.supports(feature) => {
                format!("{}: not verified on the {} yet", feature.name(), model.name)
            }
            _ if self.status.state == SessionState::Streaming => {
                "This camera has no gimbal Gimbly can drive".into()
            }
            _ => "Connect an OBSBOT camera first".into(),
        }
    }

    /// Sends `command` if its feature was checked on this camera's model;
    /// hotkeys and the preview come through here too.
    pub(super) fn gimbal(&mut self, command: GimbalCommand) {
        if let Some(model) = self.status.model
            && !model.allows(&command)
        {
            let feature = command.features().first().map_or("This", |f| f.name());
            self.notify(format!("{feature}: not verified on the {} yet", model.name));
            return;
        }
        if let Some(session) = &self.session {
            session.send(Command::Gimbal(command));
        }
    }

    pub(super) fn ai_mode_section(&mut self, ui: &mut Ui) {
        let tip = "Voice Tracking turns to the person speaking; Hand Tracking follows a \
                   raised hand. Click the highlighted mode again to stop it. Only one AI \
                   mode runs at a time.";
        self.gimbal_ui.ai_mode_open = header(
            ui,
            "AI Mode",
            Some(tip),
            self.gimbal_ui.ai_mode_open,
            |_| {},
        );
        if !self.gimbal_ui.ai_mode_open {
            return;
        }
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 20,
            bottom: 20,
        };
        let mode = self.status.ai_mode;
        let ready = self.gimbal_ready()
            && mode.is_some()
            && (self.supports(Feature::VoiceTracking) || self.supports(Feature::HandTracking));
        let tip = self.gimbal_tip(Feature::VoiceTracking);
        let (voice_range, hand) = (self.status.voice_range, self.status.hand);
        let mut picked = None;
        let mut open_range = false;
        band(ui, SECTION, margin, |ui| {
            ui.add_enabled_ui(ready, |ui| {
                let modes = [AiMode::Voice, AiMode::Hand];
                let selected = modes.iter().position(|&m| Some(m) == mode);
                let row = ui.add_enabled_ui(ready, |ui| {
                    pill_row(ui, &["Voice Tracking", "Hand Tracking"], selected, 16.0)
                });
                row.response.on_disabled_hover_text(&tip);
                if let Some(i) = row.inner {
                    let clicked = modes[i];
                    let next = if mode == Some(clicked) {
                        AiMode::Off
                    } else {
                        clicked
                    };
                    picked = Some(GimbalCommand::SetAiMode(next));
                }
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                if mode == Some(AiMode::Voice) {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.set_height(26.0);
                        ui.label(egui::RichText::new("Voice Tracking Range").size(15.0));
                        help(
                            ui,
                            "Omnidirectional follows voices from all around the camera; \
                             Forward only voices in front of it",
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for (range, label, full) in [
                                (VoiceRange::Forward, "Forward", "Forward"),
                                (VoiceRange::Omni, "Omni", "Omnidirectional"),
                            ] {
                                let size = vec2(70.0, 26.0);
                                if pill(ui, label, voice_range == Some(range), size)
                                    .on_hover_text(full)
                                    .clicked()
                                {
                                    picked = Some(GimbalCommand::SetVoiceRange(range));
                                }
                            }
                        });
                    });
                }
                if mode == Some(AiMode::Hand) {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.set_height(26.0);
                        ui.label(egui::RichText::new("Hand Tracking").size(15.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            open_range = small_button(ui, "Set", vec2(60.0, 25.0))
                                .on_hover_text("Set the range Hand Tracking may turn within")
                                .clicked();
                        });
                    });
                    ui.add_space(12.0);
                    let hands = [Hand::Left, Hand::Right];
                    let selected = hands.iter().position(|&h| Some(h) == hand);
                    if let Some(i) = pill_row(ui, &["Left Hand", "Right Hand"], selected, 16.0) {
                        picked = Some(GimbalCommand::SetHand(hands[i]));
                    }
                }
            });
        });
        if let Some(command) = picked {
            self.gimbal(command);
        }
        if open_range {
            self.gimbal_ui.hand_range_open = true;
            self.gimbal(GimbalCommand::OpenHandRange);
        }
    }

    /// Replaces the console while Set is open: Cancel and Done, the four
    /// limits, the return position and the gimbal controls to aim with.
    pub(super) fn hand_range_panel(&mut self, ui: &mut Ui) {
        let range = self.status.hand_range;
        let mut close = None;
        let mut edits = RangeEdits::default();
        band(ui, SECTION, panel_margin(), |ui| {
            close = cancel_done(ui, range.is_some());
            ui.add_space(12.0);
            edits = range_body(
                ui,
                range,
                "Aim the gimbal, then right-click a limit and choose Update. Hand \
                 Tracking only follows within this range. Click a limit to turn there.",
                "Put the four limits back to 45°, -45°, 30° and -30°",
            );
        });
        self.gimbal_controls(ui, GimbalCommand::ResetWithZoom);
        if range.is_some() {
            if let Some(field) = edits.update {
                self.gimbal(GimbalCommand::UpdateHandRange(field));
            }
            if let Some(limits) = edits.reset {
                self.gimbal(GimbalCommand::ResetHandRange { limits });
            }
            self.go_to(edits.go);
        }
        if let Some(save) = close {
            self.gimbal_ui.hand_range_open = false;
            self.gimbal(GimbalCommand::CloseHandRange { save });
        }
    }

    /// Replaces the console while the Zone Tracking settings are open: the
    /// Tracking Range tab (as in the Hand Tracking panel, plus auto-tracking)
    /// and the Non-tracking Area tab.
    pub(super) fn zone_panel(&mut self, ui: &mut Ui) {
        let zone = self.status.zone.clone();
        let areas_tab = self.gimbal_ui.zone_areas;
        let mut close = None;
        let mut tab = None;
        let mut edits = RangeEdits::default();
        let mut command = None;
        let mut rename = None;
        band(ui, SECTION, panel_margin(), |ui| {
            close = cancel_done(ui, zone.is_some());
            ui.add_space(10.0);
            tab = tabs(
                ui,
                &["Tracking Range", "Non-tracking Area"],
                areas_tab.into(),
            );
            ui.add_space(14.0);
            if !areas_tab {
                edits = range_body(
                    ui,
                    zone.as_ref().map(|zone| zone.range),
                    "Aim the gimbal, then right-click a limit and choose Update. Tracking \
                     stays within this range. Click a limit to turn there.",
                    "Put the four limits back to the camera's defaults",
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.set_height(26.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(egui::RichText::new("Auto-tracking new target").size(15.0));
                    help(
                        ui,
                        "When the tracked person leaves the range, follow the next person \
                         who enters it",
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let on = zone.as_ref().is_some_and(|zone| zone.auto_target);
                        let clicked = ui
                            .add_enabled_ui(zone.is_some(), |ui| toggle(ui, on))
                            .inner
                            .clicked();
                        if clicked {
                            command = Some(GimbalCommand::SetZoneAutoTarget(!on));
                        }
                    });
                });
            } else {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Store up to three views. At a stored view the frame stays \
                             fixed instead of following.",
                        )
                        .size(13.0)
                        .color(TEXT_DIM),
                    )
                    .wrap(),
                );
                ui.add_space(14.0);
                let areas = zone.as_ref().map_or(&[][..], |zone| &zone.areas[..]);
                let gap = 12.0;
                let width =
                    (ui.available_width() - gap * (ZONE_AREAS as f32 - 1.0)) / ZONE_AREAS as f32;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.add_enabled_ui(zone.is_some(), |ui| {
                        for slot in 0..ZONE_AREAS as u8 {
                            let Some(area) = areas.iter().find(|area| area.id == slot) else {
                                let add = pill(ui, "Add", false, vec2(width, BUTTON_HEIGHT))
                                    .on_hover_text("Store the current view as an area");
                                if add.clicked() {
                                    command = Some(GimbalCommand::AddArea);
                                }
                                continue;
                            };
                            let (yaw, pitch, zoom) = area.position();
                            let tip = format!(
                                "Go to {}\npan {yaw:.1}°, tilt {:.1}°, zoom {zoom:.2}x",
                                area.name,
                                -pitch + 0.0
                            );
                            let entries =
                                [("Update PTZ", true), ("Delete", true), ("Rename", true)];
                            let (clicked, chosen) = slot_button(
                                ui,
                                ("area", area.id),
                                width,
                                &area.name,
                                &tip,
                                &entries,
                            );
                            if clicked {
                                let to = GimbalCommand::MoveTo {
                                    pitch: Some(pitch),
                                    yaw: Some(yaw),
                                };
                                edits.go = Some((to, Some(zoom)));
                            }
                            match chosen {
                                Some("Update PTZ") => {
                                    command = Some(GimbalCommand::UpdateArea(area.id))
                                }
                                Some("Delete") => {
                                    command = Some(GimbalCommand::DeleteArea(area.id))
                                }
                                Some("Rename") => rename = Some((area.id, area.name.clone())),
                                _ => {}
                            }
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
        // The Non-tracking Area Reset turns to the camera's initial position.
        let reset = match zone.as_ref().and_then(|zone| zone.initial) {
            Some((yaw, pitch)) if areas_tab => GimbalCommand::MoveTo {
                pitch: Some(pitch),
                yaw: Some(yaw),
            },
            _ if areas_tab => GimbalCommand::Reset,
            _ => GimbalCommand::ResetWithZoom,
        };
        self.gimbal_controls(ui, reset);
        if let Some(i) = tab {
            self.gimbal_ui.zone_areas = i == 1;
        }
        if zone.is_some() {
            if let Some(field) = edits.update {
                self.gimbal(GimbalCommand::UpdateZone(field));
            }
            if let Some(limits) = edits.reset {
                self.gimbal(GimbalCommand::ResetZone { limits });
            }
            if let Some(command) = command {
                self.gimbal(command);
            }
            if let Some((id, name)) = rename {
                self.gimbal_ui.rename = Some((Renaming::Area(id), name));
            }
            self.go_to(edits.go);
        }
        if let Some(save) = close {
            self.gimbal_ui.zone_open = false;
            self.gimbal(GimbalCommand::CloseZone { save });
        }
    }

    /// Turns the gimbal (and zooms) to a range value or area that was clicked.
    fn go_to(&mut self, go: Option<(GimbalCommand, Option<f32>)>) {
        if let Some((to, zoom)) = go.filter(|_| self.gimbal_ready()) {
            if let Some(ratio) = zoom {
                self.zoom_target_clear();
                self.gimbal(GimbalCommand::ZoomRatio(ratio));
            }
            self.gimbal(to);
        }
    }

    // ---- presets ----------------------------------------------------------

    pub(super) fn preset_section(&mut self, ui: &mut Ui) {
        let tip = "Saves gimbal positions with their zoom. Turn AI tracking and AI modes \
                   off while setting presets. Landscape presets must lie within ±140° \
                   pan and ±70° tilt.";
        self.gimbal_ui.preset_open =
            header(ui, "Preset", Some(tip), self.gimbal_ui.preset_open, |_| {});
        if !self.gimbal_ui.preset_open {
            return;
        }
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 20,
            bottom: 20,
        };
        let ready = self.ready(Feature::Presets) && self.status.presets.is_some();
        let presets = self.status.presets.clone().unwrap_or_default();
        band(ui, SECTION, margin, |ui| {
            let gap = 16.0;
            let width =
                (ui.available_width() - gap * (PRESET_SLOTS as f32 - 1.0)) / PRESET_SLOTS as f32;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_enabled_ui(ready, |ui| {
                    for slot in 0..PRESET_SLOTS {
                        match presets.get(slot) {
                            Some(preset) => self.preset_button(ui, preset, width),
                            None if slot == presets.len() => {
                                let add = pill(ui, "Add", false, vec2(width, BUTTON_HEIGHT))
                                    .on_hover_text("Add preset position")
                                    .on_disabled_hover_text(self.gimbal_tip(Feature::Presets));
                                if add.clicked() {
                                    self.gimbal(GimbalCommand::AddPreset);
                                }
                            }
                            None => {
                                ui.allocate_exact_size(vec2(width, BUTTON_HEIGHT), Sense::hover());
                            }
                        }
                    }
                });
            });
        });
    }

    /// One preset: its name triggers it, the caret opens its menu.
    fn preset_button(&mut self, ui: &mut Ui, preset: &Preset, width: f32) {
        let tip = match preset.position {
            Some(at) => format!(
                "Go to {}
pan {:.1}°, tilt {:.1}°, zoom {:.2}x",
                preset.name, at.yaw, at.pitch, at.zoom
            ),
            None => format!("Go to {}", preset.name),
        };
        let entries = [
            ("Update PTZ", true),
            ("Delete", true),
            ("Rename", true),
            ("More Settings", false),
            ("As Initial State", false),
        ];
        let (clicked, chosen) = slot_button(
            ui,
            ("preset", preset.id),
            width,
            &preset.name,
            &tip,
            &entries,
        );
        if clicked {
            self.gimbal(GimbalCommand::TriggerPreset(preset.id));
        }
        match chosen {
            Some("Update PTZ") => self.gimbal(GimbalCommand::UpdatePreset { id: preset.id }),
            Some("Delete") => self.gimbal(GimbalCommand::DeletePreset(preset.id)),
            Some("Rename") => {
                self.gimbal_ui.rename = Some((Renaming::Preset(preset.id), preset.name.clone()))
            }
            _ => {}
        }
    }

    /// The small Rename dialog, centred over the panel.
    pub(super) fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some((target, mut text)) = self.gimbal_ui.rename.take() else {
            return;
        };
        let mut done = None;
        let modal = egui::Modal::new(egui::Id::new("preset-rename"))
            .backdrop_color(Color32::from_black_alpha(120))
            .frame(
                egui::Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, FIELD_STROKE))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::same(11)),
            )
            .show(ctx, |ui| {
                ui.set_width(178.0);
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                ui.label(
                    egui::RichText::new("Please input name")
                        .size(13.0)
                        .color(TEXT_DIM),
                );
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(178.0)
                        .char_limit(PRESET_NAME_MAX),
                );
                edit.request_focus();
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let ok = pill(ui, "OK", true, vec2(77.0, 31.0));
                    let cancel = pill(ui, "Cancel", false, vec2(77.0, 31.0));
                    if ok.clicked() || enter {
                        done = Some(true);
                    } else if cancel.clicked() {
                        done = Some(false);
                    }
                });
            });
        if modal.should_close() && done.is_none() {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let name = text.trim().to_string();
                if name.is_empty() {
                    self.notify("Please input name");
                    self.gimbal_ui.rename = Some((target, text));
                } else {
                    self.gimbal(match target {
                        Renaming::Preset(id) => GimbalCommand::RenamePreset { id, name },
                        Renaming::Area(id) => GimbalCommand::RenameArea { id, name },
                    });
                }
            }
            Some(false) => {}
            None => self.gimbal_ui.rename = Some((target, text)),
        }
    }

    /// Asks, as the official app does, whether to widen the Hand Tracking
    /// range to a return position that lies outside it.
    pub(super) fn hand_range_dialog(&mut self, ctx: &egui::Context) {
        if self.status.hand_range_proposal.is_none() || !self.gimbal_ui.hand_range_open {
            return;
        }
        if let Some(widen) = question(ctx, "hand-range-widen", WIDEN_QUESTION) {
            // Hide it at once; the camera thread clears the proposal shortly.
            self.status.hand_range_proposal = None;
            self.gimbal(GimbalCommand::ConfirmHandRange(widen));
        }
    }

    /// The Zone Tracking panel's questions: widen the range to a new return
    /// position, or move the return position inside new limits (No keeps
    /// the old limit).
    pub(super) fn zone_dialog(&mut self, ctx: &egui::Context) {
        let Some((_, widen)) = self
            .status
            .zone_proposal
            .filter(|_| self.gimbal_ui.zone_open)
        else {
            return;
        };
        let text = if widen {
            WIDEN_QUESTION
        } else {
            "The return position lies outside the new range. Move it inside?"
        };
        if let Some(accept) = question(ctx, "zone-range-question", text) {
            self.status.zone_proposal = None;
            self.gimbal(GimbalCommand::ConfirmZone(accept));
        }
    }

    // ---- view and gimbal --------------------------------------------------

    pub(super) fn view_gimbal_section(&mut self, ui: &mut Ui) {
        let ready = self.ready(Feature::Gimbal);
        let mut mode = self.gimbal_ui.mode;
        let tip = self.gimbal_tip(Feature::Gimbal);
        self.gimbal_ui.view_open = header(
            ui,
            "View and Gimbal",
            None,
            self.gimbal_ui.view_open,
            |ui| {
                ui.add_enabled_ui(ready, |ui| {
                let (label, other) = match mode {
                    SpeedMode::Coarse => ("Coarse", SpeedMode::Fine),
                    SpeedMode::Fine => ("Fine", SpeedMode::Coarse),
                };
                if small_button(ui, label, vec2(74.0, 25.0))
                    .on_hover_text("Joystick speed: Coarse moves at normal speed, Fine at a third of it for precise framing")
                    .on_disabled_hover_text(&tip)
                    .clicked()
                {
                    mode = other;
                }
            });
            },
        );
        self.gimbal_ui.mode = mode;
        if self.gimbal_ui.view_open {
            self.gimbal_controls(ui, GimbalCommand::Reset);
        }
    }

    /// The View and Gimbal body: joystick, zoom, field of view and the Reset
    /// button, which sends `reset`. The Hand Tracking range panel reuses it.
    fn gimbal_controls(&mut self, ui: &mut Ui, reset: GimbalCommand) {
        let ready = self.ready(Feature::Gimbal);
        let tip = self.gimbal_tip(Feature::Gimbal);
        let fov_ready = self.ready(Feature::FieldOfView);
        let fov_tip = self.gimbal_tip(Feature::FieldOfView);
        let width = ui.available_width();
        let height = 387.0;
        let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        ui.painter().rect_filled(area, CornerRadius::ZERO, SECTION);
        let top = area.top();
        let left = area.left();

        // Side tab and Gimbal Speed panel; opening it moves the joystick right.
        let tab = Rect::from_min_size(pos2(left, top + 79.0), vec2(18.0, 58.0));
        let tab_response = ui.interact(tab, ui.id().with("gimbal-speed-tab"), Sense::click());
        let fill = if tab_response.hovered() {
            BUTTON_HOVER
        } else {
            SIDE_TAB
        };
        ui.painter().rect_filled(
            tab,
            CornerRadius {
                nw: 0,
                sw: 0,
                ne: 4,
                se: 4,
            },
            fill,
        );
        let chevron = if self.gimbal_ui.speed_panel {
            icon::CARET_LEFT
        } else {
            icon::CARET_RIGHT
        };
        ui.painter().text(
            tab.center(),
            Align2::CENTER_CENTER,
            chevron,
            font(16.0),
            TEXT,
        );
        if tab_response.on_hover_text("Gimbal Speed").clicked() {
            self.gimbal_ui.speed_panel = !self.gimbal_ui.speed_panel;
        }
        let joystick_x = if self.gimbal_ui.speed_panel {
            self.speed_panel(
                ui,
                Rect::from_min_size(pos2(left + 30.0, top + 71.0), vec2(136.0, 74.0)),
            );
            left + 263.0
        } else {
            left + 195.0
        };

        let pad = Rect::from_center_size(pos2(joystick_x, top + 108.0), joystick::PANEL_SIZE);
        let (response, drag) = joystick::panel(ui, pad, &mut self.gimbal_ui.joystick, ready);
        response.on_disabled_hover_text(&tip);
        self.joystick_drag(drag, false);
        if self.gimbal_ui.joystick.dragging() {
            ui.ctx().request_repaint_after(joystick::REPEAT_EVERY);
        }

        let zoom_row = Rect::from_min_size(pos2(left + PADDING, top + 216.0), vec2(320.0, 25.0));
        self.zoom_row(ui, zoom_row);

        let fov_row = Rect::from_min_size(
            pos2(left + PADDING, top + 261.0),
            vec2(320.0, BUTTON_HEIGHT),
        );
        let current = self.status.fov;
        let picked = place(
            ui,
            fov_row,
            Layout::left_to_right(Align::Min),
            "fov",
            |ui| {
                let row = ui.add_enabled_ui(fov_ready, |ui| {
                    pill_row(
                        ui,
                        &["Wide", "Medium", "Narrow"],
                        current.and_then(|f| Fov::ALL.iter().position(|&x| x == f)),
                        16.0,
                    )
                });
                row.response.on_disabled_hover_text(&fov_tip);
                row.inner
            },
        );
        if let Some(i) = picked {
            self.zoom_target_clear();
            self.gimbal(GimbalCommand::SetFov(Fov::ALL[i]));
        }

        let reset_row = Rect::from_min_size(
            pos2(left + PADDING, top + 317.0),
            vec2(320.0, BUTTON_HEIGHT),
        );
        let with_zoom = reset == GimbalCommand::ResetWithZoom;
        let hover = if with_zoom {
            "Return the gimbal to its initial position and the zoom to 1.0x"
        } else {
            "Return the gimbal to its initial position"
        };
        let clicked = place(ui, reset_row, Layout::default(), "gimbal-reset", |ui| {
            ui.add_enabled_ui(ready, |ui| {
                pill(ui, "Reset", false, vec2(320.0, BUTTON_HEIGHT))
                    .on_hover_text(hover)
                    .on_disabled_hover_text(&tip)
            })
            .inner
        });
        if clicked.clicked() {
            self.zoom_target_clear();
            self.gimbal(reset);
        }
    }

    /// Sends what a joystick drag asks for: a start marker, repeated speeds
    /// while held, then a stop.
    pub(super) fn joystick_drag(&mut self, drag: Drag, preview: bool) {
        if drag.started {
            if !preview {
                self.gimbal_ui.pull = PanelPull::default();
            }
            self.gimbal(GimbalCommand::Drag(true));
        }
        if let Some(offset) = drag.offset {
            if !preview && drag.moved {
                self.gimbal_ui.pull.moved(offset.length());
            }
            if drag.send {
                let ui_state = &self.gimbal_ui;
                let (pitch, yaw) = if preview {
                    gimbal::preview_speed(offset, drag.fine)
                } else {
                    gimbal::panel_speed(
                        offset,
                        ui_state.pull.ratio(),
                        ui_state.mode,
                        ui_state.speed_level,
                    )
                };
                self.gimbal(GimbalCommand::Speed { pitch, yaw });
            }
        }
        if drag.ended {
            self.gimbal(GimbalCommand::Speed {
                pitch: 0.0,
                yaw: 0.0,
            });
            self.gimbal(GimbalCommand::Drag(false));
        }
    }

    fn speed_panel(&mut self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter();
        painter.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            "Gimbal Speed",
            font(15.0),
            TEXT,
        );
        painter.text(
            pos2(rect.left(), rect.top() + 33.0),
            Align2::LEFT_CENTER,
            "Slow",
            font(12.0),
            TEXT_DIM,
        );
        painter.text(
            pos2(rect.right(), rect.top() + 33.0),
            Align2::RIGHT_CENTER,
            "Fast",
            font(12.0),
            TEXT_DIM,
        );
        let track = Rect::from_min_size(
            pos2(rect.left(), rect.top() + 58.0),
            vec2(rect.width(), 16.0),
        );
        let levels = gimbal::SPEED_LEVEL_MAX - 1;
        let t = (self.gimbal_ui.speed_level - 1) as f32 / levels as f32;
        if let Some(t) = slider(ui, track, t, true, "gimbal-speed") {
            self.gimbal_ui.speed_level = 1 + (t * levels as f32).round() as u8;
        }
    }

    // ---- zoom ---------------------------------------------------------------

    /// Zoom goes through vendor control; otherwise through the UVC control.
    fn vendor_zoom(&self) -> bool {
        self.status.vendor_control && self.supports(Feature::Zoom)
    }

    /// Ratio range the zoom controls cover for the current camera.
    fn zoom_ratio_max(&self) -> Option<f32> {
        if self.vendor_zoom() {
            Some(ZOOM_RATIO_MAX)
        } else {
            self.status
                .zoom_range
                .filter(|r| r.min > 0)
                .map(|r| r.max as f32 / r.min as f32)
        }
    }

    /// The zoom ratio to show: the user's pick while the camera catches up,
    /// otherwise what the camera reports.
    pub(super) fn shown_zoom_ratio(&self) -> Option<f32> {
        let held = self
            .gimbal_ui
            .zoom_target
            .filter(|(_, since)| since.elapsed() < ZOOM_HOLD)
            .map(|(ratio, _)| ratio);
        held.or(if self.vendor_zoom() {
            self.status.zoom_ratio
        } else {
            let range = self.status.zoom_range.filter(|r| r.min > 0)?;
            Some(self.status.zoom? as f32 / range.min as f32)
        })
    }

    fn zoom_target_clear(&mut self) {
        self.gimbal_ui.zoom_target = None;
    }

    pub(super) fn set_zoom_ratio(&mut self, ratio: f32) {
        let Some(max) = self.zoom_ratio_max() else {
            return;
        };
        let ratio = (ratio.clamp(1.0, max) * 100.0).round() / 100.0;
        if self.shown_zoom_ratio() == Some(ratio) {
            return;
        }
        self.gimbal_ui.zoom_target = Some((ratio, Instant::now()));
        if self.vendor_zoom() {
            self.gimbal(GimbalCommand::ZoomRatio(ratio));
        } else if let Some(range) = self.status.zoom_range {
            let value = (ratio * range.min as f32).round() as i32;
            self.set_zoom(value);
        }
    }

    fn zoom_row(&mut self, ui: &mut Ui, rect: Rect) {
        let max = self.zoom_ratio_max();
        let ratio = self.shown_zoom_ratio();
        let enabled = max.is_some() && ratio.is_some();
        let max = max.unwrap_or(ZOOM_RATIO_MAX);
        let ratio = ratio.unwrap_or(1.0);
        if self
            .gimbal_ui
            .zoom_target
            .is_some_and(|(_, since)| since.elapsed() < ZOOM_HOLD)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }

        let track = Rect::from_min_size(rect.min + vec2(0.0, 5.0), vec2(240.0, 15.0));
        let t = (ratio - 1.0) / (max - 1.0).max(f32::EPSILON);
        let ui_enabled = ui.is_enabled() && enabled;
        let picked = place(ui, track, Layout::default(), "zoom-slider", |ui| {
            ui.add_enabled_ui(ui_enabled, |ui| slider(ui, track, t, ui_enabled, "zoom"))
                .inner
        });
        if let Some(t) = picked {
            self.set_zoom_ratio(1.0 + t * (max - 1.0));
        }

        let spin = Rect::from_min_size(pos2(rect.right() - 74.0, rect.top()), vec2(74.0, 25.0));
        let step = place(ui, spin, Layout::default(), "zoom-spin", |ui| {
            ui.add_enabled_ui(ui_enabled, |ui| {
                spin_box(ui, spin, &format!("{ratio:.2} x"))
            })
            .inner
        });
        if let Some(direction) = step {
            self.set_zoom_ratio(ratio + direction * ZOOM_STEP);
        }
    }
}

/// The small rounded button in a section header, e.g. Coarse/Fine.
fn small_button(ui: &mut Ui, text: &str, size: Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.is_enabled() && response.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(5),
        if hover { BUTTON_HOVER } else { BUTTON },
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font(14.0),
        if ui.is_enabled() { TEXT } else { TEXT_FAINT },
    );
    response
}

/// The popup frame of the preset and range menus.
pub(super) fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(FIELD)
        .stroke(Stroke::new(1.0, FIELD_STROKE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(0, 2))
}

/// A Hand Tracking range value: a click turns the gimbal to it and its
/// right-click menu holds Update. Returns (clicked, Update chosen).
fn value_with_update(ui: &mut Ui, rect: Rect, text: &str, field: RangeField) -> (bool, bool) {
    let response = ui.interact(rect, ui.id().with(("hand-range", field)), Sense::click());
    let fill = if response.hovered() {
        BUTTON_HOVER
    } else {
        BUTTON
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, text, font(14.0), TEXT);
    let response = response.on_hover_text(
        "Click to turn the gimbal there. Right-click to take it from the gimbal position",
    );
    let clicked = response.clicked() && ui.is_enabled();
    let mut chosen = false;
    egui::Popup::context_menu(&response)
        .width(134.0)
        .frame(menu_frame())
        .show(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if menu_entry(ui, "Update").clicked() {
                chosen = true;
                ui.close();
            }
        });
    (clicked, chosen)
}

/// Dim explanatory text with a reset icon on its right. Returns true when the
/// icon was clicked.
fn note_with_reset(ui: &mut Ui, text: &str, tip: &str) -> bool {
    ui.horizontal(|ui| {
        let icon_size = 22.0;
        ui.add_sized(
            vec2(ui.available_width() - icon_size - 8.0, 0.0),
            egui::Label::new(egui::RichText::new(text).size(13.0).color(TEXT_DIM)).wrap(),
        );
        icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, icon_size, 14.0, false)
            .on_hover_text(tip)
            .clicked()
    })
    .inner
}

pub(super) fn menu_entry(ui: &mut Ui, text: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    if ui.is_enabled() && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, BUTTON_HOVER);
    }
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(14.0),
        if ui.is_enabled() { TEXT } else { TEXT_FAINT },
    );
    response
}

/// What a stored-view name is being renamed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renaming {
    Preset(u8),
    /// A Non-tracking Area in the open Zone Tracking panel.
    Area(u8),
}

const WIDEN_QUESTION: &str =
    "The return position lies outside the tracking range. Widen the range to include it?";

/// A small modal with No and OK; `Some(true)` for OK.
fn question(ctx: &egui::Context, id: &str, text: &str) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(
            egui::Frame::new()
                .fill(CARD)
                .stroke(Stroke::new(1.0, FIELD_STROKE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.set_width(240.0);
            ui.spacing_mut().item_spacing = vec2(8.0, 12.0);
            ui.label(egui::RichText::new(text).size(13.0).color(TEXT));
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                if pill(ui, "No", false, vec2(85.0, 31.0)).clicked() {
                    answer = Some(false);
                }
                if pill(ui, "OK", true, vec2(85.0, 31.0)).clicked() {
                    answer = Some(true);
                }
            });
        });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

fn panel_margin() -> egui::Margin {
    egui::Margin {
        left: PADDING as i8,
        right: PADDING as i8,
        top: 14,
        bottom: 20,
    }
}

/// The Cancel and Done row of a settings panel; `Some(true)` for Done,
/// which waits until the panel's values are `ready`.
fn cancel_done(ui: &mut Ui, ready: bool) -> Option<bool> {
    let mut close = None;
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        if small_button(ui, "Cancel", vec2(74.0, 25.0)).clicked() {
            close = Some(false);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let done = ui
                .add_enabled_ui(ready, |ui| small_button(ui, "Done", vec2(74.0, 25.0)))
                .inner
                .on_disabled_hover_text("Reading the settings from the camera");
            if done.clicked() {
                close = Some(true);
            }
        });
    });
    close
}

/// Text tabs with an accent bar under the selected one; returns a clicked
/// tab.
fn tabs(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let width = ui.available_width() / labels.len().max(1) as f32;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, label) in labels.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
            let color = if i == selected || response.hovered() {
                TEXT
            } else {
                TEXT_DIM
            };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                *label,
                font(14.0),
                color,
            );
            if i == selected {
                let bar = Rect::from_center_size(
                    pos2(rect.center().x, rect.bottom() - 2.0),
                    vec2(20.0, 3.0),
                );
                ui.painter().rect_filled(bar, CornerRadius::same(1), ACCENT);
            }
            if response.clicked() && i != selected {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// What the user did in [`range_body`].
#[derive(Default)]
struct RangeEdits {
    /// Update chosen on a value.
    update: Option<RangeField>,
    /// A reset icon: `true` for the limits, `false` for the return position.
    reset: Option<bool>,
    /// A clicked value to turn (and zoom) to.
    go: Option<(GimbalCommand, Option<f32>)>,
}

/// The range box with its four limits and the return position line, shared
/// by the Hand Tracking and Zone Tracking panels. `range` is `None` while
/// it is read.
fn range_body(
    ui: &mut Ui,
    range: Option<TrackingRange>,
    note: &str,
    limits_tip: &str,
) -> RangeEdits {
    let mut edits = RangeEdits::default();
    if note_with_reset(ui, note, limits_tip) {
        edits.reset = Some(true);
    }
    ui.add_space(14.0);

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 170.0), Sense::hover());
    let frame = rect.shrink2(vec2(52.0, 26.0));
    let corners = [
        frame.left_top(),
        frame.right_top(),
        frame.right_bottom(),
        frame.left_bottom(),
        frame.left_top(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, TEXT_FAINT),
        4.0,
        3.0,
    ));
    let limits = range.map(|r| r.limits);
    // Shown as the official app does: pan as is, tilt negated
    // (`+ 0.0` turns -0.0 into 0.0).
    let shown = |key: usize, sign: f32| limits.map(|l| sign * l[key] + 0.0);
    let labels = [
        (2, pos2(frame.center().x, rect.top() + 12.0), shown(2, -1.0)),
        (
            3,
            pos2(frame.center().x, rect.bottom() - 12.0),
            shown(3, -1.0),
        ),
        (1, pos2(rect.left() + 26.0, frame.center().y), shown(1, 1.0)),
        (
            0,
            pos2(rect.right() - 26.0, frame.center().y),
            shown(0, 1.0),
        ),
    ];
    for (key, at, value) in labels {
        let text = value.map_or("--".to_owned(), |v| format!("{v:.1}°"));
        let field = RangeField::Limit(key);
        let (clicked, chosen) = value_with_update(
            ui,
            Rect::from_center_size(at, vec2(62.0, 30.0)),
            &text,
            field,
        );
        if chosen {
            edits.update = Some(field);
        }
        // Limits are kept in the camera's sign, as the gimbal takes them.
        let limit = limits.and_then(|l| l.get(key).copied());
        if let Some(limit) = limit.filter(|_| clicked) {
            let (pitch, yaw) = if key < 2 {
                (None, Some(limit))
            } else {
                (Some(limit), None)
            };
            edits.go = Some((GimbalCommand::MoveTo { pitch, yaw }, None));
        }
    }

    ui.add_space(10.0);
    if note_with_reset(
        ui,
        "After exceeding the tracking range, the gimbal returns to the set position.",
        "Put the return position back to P 0°, T 0°, Z 1.0x",
    ) {
        edits.reset = Some(false);
    }
    ui.add_space(8.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let text = range.map_or("P: --° | T: --° | Z: --".to_owned(), |r| {
        let (yaw, pitch, zoom) = r.return_position();
        format!(
            "P: {:.2}° | T: {:.2}° | Z: {zoom:.2}",
            yaw + 0.0,
            -pitch + 0.0
        )
    });
    let (clicked, chosen) = value_with_update(ui, line, &text, RangeField::Return);
    if chosen {
        edits.update = Some(RangeField::Return);
    }
    if let Some((yaw, pitch, zoom)) = range.filter(|_| clicked).map(|r| r.return_position()) {
        let to = GimbalCommand::MoveTo {
            pitch: Some(pitch),
            yaw: Some(yaw),
        };
        edits.go = Some((to, Some(zoom)));
    }
    edits
}

/// A stored view (preset or area): its name recalls it, the caret opens a
/// menu of `entries` (label, enabled). Returns (name clicked, entry chosen).
fn slot_button(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    width: f32,
    name: &str,
    tip: &str,
    entries: &[(&'static str, bool)],
) -> (bool, Option<&'static str>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON_HEIGHT), Sense::hover());
    let caret = Rect::from_min_max(pos2(rect.right() - 25.0, rect.top()), rect.max);
    let name_rect = Rect::from_min_max(rect.min, pos2(caret.left(), rect.bottom()));
    let name_response = ui.interact(name_rect, ui.id().with(("slot", salt)), Sense::click());
    let caret_response = ui.interact(caret, ui.id().with(("slot-menu", salt)), Sense::click());
    let hover = ui.is_enabled() && (name_response.hovered() || caret_response.hovered());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(6),
        if hover { BUTTON_HOVER } else { BUTTON },
    );
    let galley = painter.layout(name.to_owned(), font(14.0), TEXT, name_rect.width() - 8.0);
    painter.galley(
        pos2(
            name_rect.center().x - galley.size().x / 2.0 + 2.0,
            name_rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        TEXT,
    );
    painter.text(
        caret.center() - vec2(4.0, 0.0),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        font(15.0),
        TEXT,
    );
    let clicked = name_response.on_hover_text(tip).clicked();
    let mut chosen = None;
    egui::Popup::menu(&caret_response)
        .id(ui.id().with(("slot-popup", salt)))
        .width(134.0)
        .gap(2.0)
        .frame(menu_frame())
        .show(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for &(label, enabled) in entries {
                let item = ui
                    .add_enabled_ui(enabled, |ui| menu_entry(ui, label))
                    .inner
                    .on_disabled_hover_text(NOT_YET);
                if item.clicked() {
                    chosen = Some(label);
                    ui.close();
                }
            }
        });
    (clicked, chosen)
}
