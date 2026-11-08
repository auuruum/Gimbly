//! The Video Preview window: a frameless window with its own title bar, the
//! camera picture and a control bar underneath.
//!
//! The layout follows OBSBOT Center's preview window; the drawing code
//! is Gimbly's own.

use crate::branding;
use crate::camera::{SessionState, SessionStatus, Transform};
use crate::joystick::{self, Drag, OverlayState};
use crate::preview_view;
use crate::settings::{PreviewMode, Resolution};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, Ui, UiBuilder, Vec2, ViewportCommand, pos2, vec2,
};
use egui_phosphor::regular as icon;

const NOT_YET: &str = "Not implemented in Gimbly yet";
const TITLE_HEIGHT: f32 = 39.0;
const BAR_HEIGHT: f32 = 43.0;
const BUTTON: f32 = 24.0;
const GLYPH: f32 = 16.0;
/// Width of the window so a 16:9 picture fills the video area.
const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 540.0 + TITLE_HEIGHT + BAR_HEIGHT;

/// What the window shows this frame.
pub struct PreviewInput<'a> {
    pub texture: Option<&'a TextureHandle>,
    pub status: &'a SessionStatus,
    pub device: Option<&'a str>,
    pub resolution: Resolution,
    pub preview_mode: PreviewMode,
    pub virtual_camera: bool,
    pub transform: Transform,
    pub toast: Option<&'a str>,
    /// Dragging the picture turns the gimbal.
    pub gimbal: bool,
}

/// Something the user asked for that the app has to carry out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewAction {
    Retry,
    ToggleVirtualCamera,
    Snapshot,
    OpenFolder,
    SetResolution(Resolution),
    SetPreviewMode(PreviewMode),
    /// Rotate or mirror the camera picture (preview and virtual camera).
    SetTransform(Transform),
    /// The gimbal drag overlay moved.
    Gimbal(Drag),
}

pub struct PreviewWindow {
    pub open: bool,
    icon: TextureHandle,
    /// Where the window first opens: beside the control panel.
    position: Option<Pos2>,
    composition_lines: bool,
    on_top: bool,
    gimbal_drag: OverlayState,
}

impl PreviewWindow {
    pub fn new(icon: TextureHandle) -> Self {
        Self {
            open: false,
            icon,
            position: None,
            composition_lines: false,
            on_top: false,
            gimbal_drag: OverlayState::default(),
        }
    }

    /// Opens or closes the window; `ctx` is the control panel's.
    pub fn show(&mut self, ctx: &egui::Context, input: PreviewInput<'_>) -> Vec<PreviewAction> {
        if !self.open {
            let drag = self.gimbal_drag.cancel();
            return if drag.ended {
                vec![PreviewAction::Gimbal(drag)]
            } else {
                Vec::new()
            };
        }
        let position = *self.position.get_or_insert_with(|| {
            // Left of the panel when it fits (the panel usually sits at the
            // screen's right edge), otherwise right of it.
            ctx.input(|i| i.viewport().outer_rect)
                .map_or(pos2(80.0, 80.0), |panel| {
                    if panel.left() >= WIDTH + 8.0 {
                        pos2(panel.left() - WIDTH - 8.0, panel.top())
                    } else {
                        pos2(panel.right() + 8.0, panel.top())
                    }
                })
        });
        let level = if self.on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        let builder = egui::ViewportBuilder::default()
            .with_title("Video Preview")
            .with_icon(branding::window_icon())
            .with_position(position)
            .with_inner_size([WIDTH, HEIGHT])
            .with_min_inner_size([480.0, 270.0 + TITLE_HEIGHT + BAR_HEIGHT])
            .with_decorations(false)
            .with_resizable(true)
            .with_window_level(level);
        let mut actions = Vec::new();
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("video-preview"),
            builder,
            |ui, _class| {
                close = self.contents(ui, &input, &mut actions);
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            },
        );
        if close {
            self.open = false;
            let drag = self.gimbal_drag.cancel();
            if drag.ended {
                actions.push(PreviewAction::Gimbal(drag));
            }
        }
        actions
    }

    /// Draws the window. Returns true when the user closed it.
    fn contents(
        &mut self,
        ui: &mut Ui,
        input: &PreviewInput<'_>,
        actions: &mut Vec<PreviewAction>,
    ) -> bool {
        let ctx = ui.ctx().clone();
        let (fullscreen, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.fullscreen.unwrap_or(false), v.maximized.unwrap_or(false))
        });
        if fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }

        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, CARD);
        let mut close = false;
        let video = if fullscreen {
            full
        } else {
            let title = Rect::from_min_size(full.min, vec2(full.width(), TITLE_HEIGHT));
            let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - BAR_HEIGHT), full.max);
            close = self.title_bar(ui, title, maximized);
            self.control_bar(ui, bar, input, actions);
            Rect::from_min_max(title.left_bottom(), bar.right_top())
        };
        self.video(ui, video, input, actions);
        if fullscreen {
            let exit = Rect::from_min_size(
                pos2(full.right() - 44.0, full.top() + 12.0),
                Vec2::splat(32.0),
            );
            ui.painter().rect_filled(
                exit.expand(4.0),
                CornerRadius::same(8),
                Color32::from_black_alpha(160),
            );
            place(ui, exit, Layout::default(), "exit-fullscreen", |ui| {
                if icon_button(ui, icon::CORNERS_IN, 32.0, 20.0, false)
                    .on_hover_text("Exit full screen (Esc)")
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
            });
        } else {
            ui.painter().rect_stroke(
                full,
                CornerRadius::ZERO,
                Stroke::new(1.0, WINDOW_EDGE),
                StrokeKind::Inside,
            );
            if !maximized {
                resize_edges(ui, full, 5.0);
            }
        }
        if let Some(message) = input.toast {
            toast(&ctx, message, BAR_HEIGHT + 16.0, 420.0);
        }
        close
    }

    fn title_bar(&mut self, ui: &mut Ui, rect: Rect, maximized: bool) -> bool {
        let response = ui.interact(rect, ui.id().with("title-drag"), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if response.double_clicked() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        let painter = ui.painter();
        let logo = Rect::from_min_size(
            pos2(rect.left() + 12.0, rect.center().y - 8.0),
            Vec2::splat(16.0),
        );
        painter.image(
            self.icon.id(),
            logo,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let title = painter.text(
            pos2(logo.right() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Video Preview",
            font(15.0),
            TEXT,
        );
        let help_rect = Rect::from_min_size(
            pos2(title.right() + 6.0, rect.center().y - 9.0),
            vec2(16.0, 18.0),
        );
        place(ui, help_rect, Layout::default(), "preview-help", |ui| {
            help(
                ui,
                "While the preview, recording or the virtual camera is on, Gimbly \
                 keeps the camera open, so other apps cannot use it directly. Choose \
                 the virtual camera in those apps instead.",
            );
        });

        let buttons = Rect::from_min_max(
            pos2(rect.center().x, rect.top()),
            pos2(rect.right() - 10.0, rect.bottom()),
        );
        let mut close = false;
        place(
            ui,
            buttons,
            Layout::right_to_left(Align::Center),
            "preview-title-buttons",
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, icon::X, BUTTON, 16.0, false)
                    .on_hover_text("Close")
                    .clicked()
                {
                    close = true;
                }
                let (glyph, tip) = if maximized {
                    (icon::COPY, "Restore")
                } else {
                    (icon::SQUARE, "Maximize")
                };
                if icon_button(ui, glyph, BUTTON, 14.0, false)
                    .on_hover_text(tip)
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                if icon_button(ui, icon::MINUS, BUTTON, 16.0, false)
                    .on_hover_text("Minimize")
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                if icon_button(ui, icon::CORNERS_OUT, BUTTON, 16.0, false)
                    .on_hover_text("Full screen")
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::Fullscreen(true));
                }
                if icon_button(ui, icon::PUSH_PIN, BUTTON, 15.0, self.on_top)
                    .on_hover_text("Sticky on top")
                    .clicked()
                {
                    self.on_top = !self.on_top;
                }
            },
        );
        close
    }

    fn control_bar(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        input: &PreviewInput<'_>,
        actions: &mut Vec<PreviewAction>,
    ) {
        let inner = rect.shrink2(vec2(10.0, 0.0));
        place(
            ui,
            inner,
            Layout::left_to_right(Align::Center),
            "preview-left",
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                self.resolution_button(ui, input, actions);
                ui.add_space(6.0);
                // These change the picture itself, so the virtual camera,
                // snapshots and the preview all turn together.
                let t = input.transform;
                if icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, BUTTON, GLYPH, false)
                    .on_hover_text("Rotate 90° counter-clockwise")
                    .clicked()
                {
                    actions.push(PreviewAction::SetTransform(t.counter_clockwise()));
                }
                if icon_button(ui, icon::ARROW_CLOCKWISE, BUTTON, GLYPH, false)
                    .on_hover_text("Rotate 90° clockwise")
                    .clicked()
                {
                    actions.push(PreviewAction::SetTransform(t.clockwise()));
                }
                if icon_button(ui, icon::FLIP_HORIZONTAL, BUTTON, GLYPH, t.mirror)
                    .on_hover_text("Mirror")
                    .clicked()
                {
                    actions.push(PreviewAction::SetTransform(Transform::new(
                        t.degrees(),
                        !t.mirror,
                    )));
                }
                ui.add_enabled_ui(false, |ui| {
                    icon_button(ui, icon::SCAN, BUTTON, GLYPH, false)
                        .on_disabled_hover_text(format!("Object tracking: {NOT_YET}"));
                });
                if icon_button(ui, icon::GRID_NINE, BUTTON, GLYPH, self.composition_lines)
                    .on_hover_text("Composition lines")
                    .clicked()
                {
                    self.composition_lines = !self.composition_lines;
                }
                ui.add_enabled_ui(false, |ui| {
                    icon_button(ui, "ME", BUTTON, 12.0, false).on_disabled_hover_text(NOT_YET);
                });
            },
        );

        place(
            ui,
            inner,
            Layout::right_to_left(Align::Center),
            "preview-right",
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_enabled_ui(false, |ui| {
                    dropdown_icon(ui, icon::MICROPHONE)
                        .on_disabled_hover_text(format!("Microphone: {NOT_YET}"));
                });
                bar_separator(ui);
                ui.add_enabled_ui(false, |ui| {
                    for (glyph, name) in [
                        (icon::STICKER, "Materials"),
                        (icon::INFINITY, "Loop"),
                        (icon::USER_CIRCLE_DASHED, "Virtual human"),
                        (icon::SMILEY, "Beauty"),
                    ] {
                        icon_button(ui, glyph, BUTTON, GLYPH, false)
                            .on_disabled_hover_text(format!("{name}: {NOT_YET}"));
                    }
                });
                bar_separator(ui);
                if icon_button(ui, icon::FOLDER_SIMPLE, BUTTON, GLYPH, false)
                    .on_hover_text("Open media folder")
                    .clicked()
                {
                    actions.push(PreviewAction::OpenFolder);
                }
                let streaming = input.status.state == SessionState::Streaming;
                ui.add_enabled_ui(streaming, |ui| {
                    if icon_button(ui, icon::CAMERA, BUTTON, GLYPH, false)
                        .on_hover_text("Snapshot")
                        .on_disabled_hover_text("Snapshot needs a streaming camera")
                        .clicked()
                    {
                        actions.push(PreviewAction::Snapshot);
                    }
                });
                let tip = if input.virtual_camera {
                    "Stop Virtual Camera"
                } else {
                    "Start Virtual Camera"
                };
                if icon_button(ui, icon::WEBCAM, BUTTON, GLYPH, input.virtual_camera)
                    .on_hover_text(tip)
                    .clicked()
                {
                    actions.push(PreviewAction::ToggleVirtualCamera);
                }
                ui.add_enabled_ui(false, |ui| {
                    dropdown_icon(ui, icon::VIDEO_CAMERA)
                        .on_disabled_hover_text(format!("Preview recording: {NOT_YET}"));
                });
            },
        );
    }

    /// Aspect badge and capture mode, opening a menu of modes.
    fn resolution_button(
        &mut self,
        ui: &mut Ui,
        input: &PreviewInput<'_>,
        actions: &mut Vec<PreviewAction>,
    ) {
        let (rect, response) = ui.allocate_exact_size(vec2(96.0, BUTTON), Sense::click());
        let hover = response.hovered();
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(4),
            if hover { BUTTON_HOVER } else { SECTION },
        );
        let (w, h) = input.status.output.unwrap_or_else(|| {
            let (w, h, _) = input.resolution.mode();
            if input.preview_mode == PreviewMode::Portrait {
                (h * 9 / 16, h)
            } else {
                (w, h)
            }
        });
        let badge = Rect::from_min_size(
            pos2(rect.left() + 6.0, rect.center().y - 6.0),
            vec2(20.0, 12.0),
        );
        painter.rect_stroke(
            badge,
            CornerRadius::same(2),
            Stroke::new(1.0, TEXT_DIM),
            StrokeKind::Inside,
        );
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            preview_view::aspect_label(w, h),
            font(7.5),
            TEXT_DIM,
        );
        painter.text(
            pos2(badge.right() + 6.0, rect.center().y),
            Align2::LEFT_CENTER,
            input.resolution.label(),
            font(12.5),
            TEXT,
        );
        let tip = match &input.status.format {
            Some(f) if input.status.state == SessionState::Streaming => {
                format!("{f}\n{:.1} fps measured", input.status.measured_fps)
            }
            Some(f) => f.clone(),
            None => "Resolution".to_string(),
        };
        let response = response.on_hover_text(tip);

        let modes = &input.status.modes;
        egui::Popup::from_toggle_button_response(&response)
            .id(ui.id().with("resolution-menu"))
            .width(170.0)
            .gap(4.0)
            .frame(
                egui::Frame::new()
                    .fill(FIELD)
                    .stroke(Stroke::new(1.0, FIELD_STROKE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(0, 4)),
            )
            .show(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                menu_heading(ui, "Resolution");
                for &resolution in Resolution::ALL {
                    let (w, h, fps) = resolution.mode();
                    // Unknown modes (camera not open yet) stay selectable.
                    let available = modes.is_empty() || modes.iter().any(|m| m.covers(w, h, fps));
                    let picked = ui
                        .add_enabled_ui(available, |ui| {
                            menu_item(ui, resolution.label(), resolution == input.resolution)
                        })
                        .inner
                        .on_disabled_hover_text("This camera does not offer this mode");
                    if picked.clicked() && resolution != input.resolution {
                        actions.push(PreviewAction::SetResolution(resolution));
                    }
                }
                menu_heading(ui, "Orientation");
                for (mode, label) in [
                    (PreviewMode::Landscape, "Landscape 16:9"),
                    (PreviewMode::Portrait, "Portrait 9:16"),
                ] {
                    if menu_item(ui, label, mode == input.preview_mode).clicked()
                        && mode != input.preview_mode
                    {
                        actions.push(PreviewAction::SetPreviewMode(mode));
                    }
                }
            });
    }

    fn video(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        input: &PreviewInput<'_>,
        actions: &mut Vec<PreviewAction>,
    ) {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, Color32::BLACK);
        // A sleeping camera keeps streaming its own still picture; show ours.
        let asleep = input.status.sleeping && input.status.vendor_control;
        let live = input.status.state == SessionState::Streaming && !asleep;
        if let (Some(texture), true) = (input.texture, live) {
            let shown = preview_view::fit(texture.size_vec2(), rect);
            ui.painter().image(
                texture.id(),
                shown,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            if self.composition_lines {
                let [w, h] = texture.size();
                let picture = preview_view::picture_rect(shown, w, h, input.transform);
                let stroke = Stroke::new(1.0, Color32::from_white_alpha(140));
                for line in preview_view::composition_lines(picture) {
                    ui.painter().line_segment(line, stroke);
                }
            }
            let drag = joystick::overlay(ui, rect, &mut self.gimbal_drag, input.gimbal);
            if drag != Drag::default() {
                actions.push(PreviewAction::Gimbal(drag));
            }
            return;
        }
        let drag = self.gimbal_drag.cancel();
        if drag.ended {
            actions.push(PreviewAction::Gimbal(drag));
        }
        let status = input.status;
        let message = match (input.device, status.state) {
            (Some(_), _) if asleep => "The camera is asleep".to_string(),
            (None, _) => "No camera connected".to_string(),
            (Some(_), SessionState::Failed) => status
                .error
                .clone()
                .unwrap_or_else(|| "The camera stopped".into()),
            (Some(name), SessionState::Opening) => format!("Opening {name}..."),
            (Some(_), _) => "The camera stopped".to_string(),
        };
        let height = if asleep { 220.0 } else { 140.0 };
        let inner = Rect::from_center_size(rect.center(), vec2(rect.width().min(520.0), height));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                if asleep {
                    ui.add(egui::Image::new((self.icon.id(), Vec2::splat(112.0))));
                    ui.add_space(16.0);
                }
                ui.label(
                    egui::RichText::new(icon::VIDEO_CAMERA_SLASH)
                        .size(36.0)
                        .color(TEXT_FAINT),
                );
                ui.label(egui::RichText::new(message).color(TEXT_DIM));
                if input.device.is_none() || status.state == SessionState::Failed {
                    ui.add_space(8.0);
                    if pill(ui, "Retry", false, vec2(96.0, 32.0)).clicked() {
                        actions.push(PreviewAction::Retry);
                    }
                }
            },
        );
    }
}

/// Icon button with the small caret OBSBOT uses for buttons that have a menu.
fn dropdown_icon(ui: &mut Ui, glyph: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON + 10.0, BUTTON), Sense::click());
    let color = if ui.is_enabled() {
        TEXT
    } else {
        Color32::from_gray(0xE0)
    };
    let painter = ui.painter();
    painter.text(
        pos2(rect.left() + BUTTON / 2.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        font(GLYPH),
        color,
    );
    painter.text(
        pos2(rect.right() - 4.0, rect.bottom() - 6.0),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        font(9.0),
        color,
    );
    response
}

fn bar_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(5.0, 18.0), Sense::hover());
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(1.0, 18.0)),
        CornerRadius::ZERO,
        SEPARATOR_LIGHT,
    );
}

fn menu_heading(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(12.0),
        TEXT_DIM,
    );
}

fn menu_item(ui: &mut Ui, text: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if ui.is_enabled() && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, BUTTON_HOVER);
    }
    let color = match (ui.is_enabled(), selected) {
        (false, _) => TEXT_FAINT,
        (true, true) => ACCENT_HOVER,
        (true, false) => TEXT,
    };
    ui.painter().text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(14.0),
        color,
    );
    if selected {
        ui.painter().text(
            pos2(rect.right() - 14.0, rect.center().y),
            Align2::CENTER_CENTER,
            icon::CHECK,
            font(14.0),
            ACCENT_HOVER,
        );
    }
    response
}
