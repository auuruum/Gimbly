//! The Console sections that drive the gimbal: AI Mode (placeholder),
//! Preset and View and Gimbal, plus the preset rename dialog.
//!
//! Layout and behaviour follow OBSBOT Center's panel for the Tiny 3 Lite; the code is
//! Gimbly's own.

use super::{GimblyApp, NOT_YET};
use crate::camera::{Command, Fov, GimbalCommand, Preset, SessionState, ZOOM_RATIO_MAX};
use crate::gimbal::{self, PanelPull, SpeedMode};
use crate::joystick::{self, Drag, DragState};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

/// Presets the Preset section shows (OBSBOT Center has three slots).
const PRESET_SLOTS: usize = 3;
/// How long the zoom controls keep showing the value the user picked while
/// the camera zooms towards it.
const ZOOM_HOLD: Duration = Duration::from_millis(1500);
/// Zoom hotkeys and the spin box arrows change the ratio by this much.
pub const ZOOM_STEP: f32 = 0.1;

const SLIDER_TRACK: Color32 = Color32::from_rgb(0x08, 0x0A, 0x0B);
const SPIN_EDGE: Color32 = Color32::from_rgb(0x0F, 0x0F, 0x11);
const SPIN_ARROW_BOX: Color32 = Color32::from_rgb(0x37, 0x3A, 0x3D);
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
    /// Preset being renamed and the text typed so far.
    rename: Option<(u8, String)>,
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
        }
    }
}

impl GimbalUi {
    /// Forget per-camera state when another camera is selected.
    pub fn reset_camera(&mut self) {
        self.zoom_target = None;
        self.rename = None;
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
    fn gimbal_ready(&self) -> bool {
        self.status.vendor_control && self.status.state == SessionState::Streaming
    }

    fn gimbal_tip(&self) -> &'static str {
        if self.status.vendor_control_untested {
            "Gimbal control is not verified on this camera model yet"
        } else if self.status.state == SessionState::Streaming {
            "This camera has no gimbal Gimbly can drive"
        } else {
            "Connect an OBSBOT camera first"
        }
    }

    pub(super) fn gimbal(&mut self, command: GimbalCommand) {
        if let Some(session) = &self.session {
            session.send(Command::Gimbal(command));
        }
    }

    pub(super) fn ai_mode_section(&mut self, ui: &mut Ui) {
        self.gimbal_ui.ai_mode_open = header(
            ui,
            "AI Mode",
            Some(NOT_YET),
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
        band(ui, SECTION, margin, |ui| {
            ui.add_enabled_ui(false, |ui| {
                pill_row(ui, &["Voice Tracking", "Hand Tracking"], None, 16.0);
            })
            .response
            .on_disabled_hover_text(NOT_YET);
        });
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
        let ready = self.gimbal_ready();
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
                                    .on_disabled_hover_text(self.gimbal_tip());
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
        let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON_HEIGHT), Sense::hover());
        let caret = Rect::from_min_max(pos2(rect.right() - 25.0, rect.top()), rect.max);
        let name = Rect::from_min_max(rect.min, pos2(caret.left(), rect.bottom()));
        let name_response = ui.interact(name, ui.id().with(("preset", preset.id)), Sense::click());
        let caret_response = ui.interact(
            caret,
            ui.id().with(("preset-menu", preset.id)),
            Sense::click(),
        );
        let hover = ui.is_enabled() && (name_response.hovered() || caret_response.hovered());
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(6),
            if hover { BUTTON_HOVER } else { BUTTON },
        );
        let galley = painter.layout(preset.name.clone(), font(14.0), TEXT, name.width() - 8.0);
        painter.galley(
            pos2(
                name.center().x - galley.size().x / 2.0 + 2.0,
                name.center().y - galley.size().y / 2.0,
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
        let tip = match preset.position {
            Some(at) => format!(
                "Go to {}
pan {:.1}°, tilt {:.1}°, zoom {:.2}x",
                preset.name, at.yaw, at.pitch, at.zoom
            ),
            None => format!("Go to {}", preset.name),
        };
        if name_response.on_hover_text(tip).clicked() {
            self.gimbal(GimbalCommand::TriggerPreset(preset.id));
        }
        let mut chosen = None;
        egui::Popup::menu(&caret_response)
            .id(ui.id().with(("preset-popup", preset.id)))
            .width(134.0)
            .gap(2.0)
            .frame(
                egui::Frame::new()
                    .fill(FIELD)
                    .stroke(Stroke::new(1.0, FIELD_STROKE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(0, 2)),
            )
            .show(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (label, enabled) in [
                    ("Update PTZ", true),
                    ("Delete", true),
                    ("Rename", true),
                    ("More Settings", false),
                    ("As Initial State", false),
                ] {
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
        match chosen {
            Some("Update PTZ") => self.gimbal(GimbalCommand::UpdatePreset {
                id: preset.id,
                name: preset.name.clone(),
            }),
            Some("Delete") => self.gimbal(GimbalCommand::DeletePreset(preset.id)),
            Some("Rename") => self.gimbal_ui.rename = Some((preset.id, preset.name.clone())),
            _ => {}
        }
    }

    /// The small Rename dialog, centred over the panel.
    pub(super) fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, mut text)) = self.gimbal_ui.rename.take() else {
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
                        .char_limit(protocol_name_limit()),
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
                    self.gimbal_ui.rename = Some((id, text));
                } else {
                    self.gimbal(GimbalCommand::RenamePreset { id, name });
                }
            }
            Some(false) => {}
            None => self.gimbal_ui.rename = Some((id, text)),
        }
    }

    // ---- view and gimbal --------------------------------------------------

    pub(super) fn view_gimbal_section(&mut self, ui: &mut Ui) {
        let ready = self.gimbal_ready();
        let mut mode = self.gimbal_ui.mode;
        let tip = self.gimbal_tip();
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
                    .on_disabled_hover_text(tip)
                    .clicked()
                {
                    mode = other;
                }
            });
            },
        );
        self.gimbal_ui.mode = mode;
        if !self.gimbal_ui.view_open {
            return;
        }
        let width = ui.available_width();
        let (area, _) = ui.allocate_exact_size(vec2(width, 387.0), Sense::hover());
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
        response.on_disabled_hover_text(tip);
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
                ui.add_enabled_ui(ready, |ui| {
                    pill_row(
                        ui,
                        &["Wide", "Medium", "Narrow"],
                        current.and_then(|f| Fov::ALL.iter().position(|&x| x == f)),
                        16.0,
                    )
                })
                .inner
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
        let reset = place(ui, reset_row, Layout::default(), "gimbal-reset", |ui| {
            ui.add_enabled_ui(ready, |ui| {
                pill(ui, "Reset", false, vec2(320.0, BUTTON_HEIGHT))
                    .on_hover_text("Return the gimbal to its initial position")
                    .on_disabled_hover_text(tip)
            })
            .inner
        });
        if reset.clicked() {
            self.gimbal(GimbalCommand::Reset);
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
        if let Some(t) = slider(ui, track, t, true, ACCENT, "gimbal-speed") {
            self.gimbal_ui.speed_level = 1 + (t * levels as f32).round() as u8;
        }
    }

    // ---- zoom ---------------------------------------------------------------

    /// Ratio range the zoom controls cover for the current camera.
    fn zoom_ratio_max(&self) -> Option<f32> {
        if self.status.vendor_control {
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
        held.or(if self.status.vendor_control {
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
        if self.gimbal_ui.zoom_target.map(|(r, _)| r) == Some(ratio) {
            return;
        }
        self.gimbal_ui.zoom_target = Some((ratio, Instant::now()));
        if self.status.vendor_control {
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
            ui.add_enabled_ui(ui_enabled, |ui| {
                slider(ui, track, t, ui_enabled, Color32::WHITE, "zoom")
            })
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

/// Longest preset name that fits one vendor message.
fn protocol_name_limit() -> usize {
    40
}

/// Horizontal slider with a 4-px track and a white round handle; `fill`
/// colours the part left of the handle. Returns the new position (0-1)
/// while it is dragged or clicked.
fn slider(
    ui: &mut Ui,
    rect: Rect,
    t: f32,
    enabled: bool,
    fill: Color32,
    salt: &str,
) -> Option<f32> {
    let response = ui.interact(
        rect,
        ui.id().with(salt),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let radius = 6.5;
    let x0 = rect.left() + radius;
    let x1 = rect.right() - radius;
    let t = t.clamp(0.0, 1.0);
    let x = egui::lerp(x0..=x1, t);
    let cy = rect.center().y;
    let painter = ui.painter();
    let track = Rect::from_min_max(pos2(rect.left(), cy - 2.0), pos2(rect.right(), cy + 2.0));
    painter.rect_filled(track, CornerRadius::same(2), SLIDER_TRACK);
    if fill != Color32::WHITE {
        let left = Rect::from_min_max(track.min, pos2(x, track.bottom()));
        painter.rect_filled(
            left,
            CornerRadius::same(2),
            if enabled { fill } else { TEXT_FAINT },
        );
    }
    painter.circle_filled(
        pos2(x, cy),
        radius,
        if enabled { Color32::WHITE } else { TEXT_FAINT },
    );
    let pointer = response.interact_pointer_pos()?;
    (response.dragged() || response.clicked())
        .then(|| ((pointer.x - x0) / (x1 - x0)).clamp(0.0, 1.0))
}

/// Read-only number field with up/down arrows. Returns +1 or -1 when an
/// arrow is clicked or the mouse wheel turns over it.
fn spin_box(ui: &mut Ui, rect: Rect, text: &str) -> Option<f32> {
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(5),
        SECTION,
        Stroke::new(1.0, SPIN_EDGE),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(13.0),
        if ui.is_enabled() { TEXT } else { TEXT_FAINT },
    );
    let mut step = None;
    for (i, (glyph, direction)) in [(icon::CARET_UP, 1.0), (icon::CARET_DOWN, -1.0)]
        .into_iter()
        .enumerate()
    {
        let arrow = Rect::from_min_size(
            pos2(rect.right() - 15.0, rect.top() + 3.0 + i as f32 * 10.0),
            vec2(11.0, 9.0),
        );
        let response = ui.interact(arrow, ui.id().with(("spin", i)), Sense::click());
        let fill = if ui.is_enabled() && response.hovered() {
            BUTTON_HOVER
        } else {
            SPIN_ARROW_BOX
        };
        ui.painter().rect_filled(arrow, CornerRadius::same(2), fill);
        ui.painter().text(
            arrow.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(8.0),
            TEXT_DIM,
        );
        if response.clicked() {
            step = Some(direction);
        }
    }
    let field = ui.interact(rect, ui.id().with("spin-field"), Sense::hover());
    if field.hovered() && ui.is_enabled() {
        let scroll = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } if delta.y != 0.0 => Some(delta.y),
                _ => None,
            })
        });
        if let Some(scroll) = scroll {
            step = Some(scroll.signum());
        }
    }
    step
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

fn menu_entry(ui: &mut Ui, text: &str) -> egui::Response {
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
