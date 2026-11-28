//! Small custom-painted widgets that give the control panel its look.

use crate::theme::*;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2,
    vec2,
};
use egui_phosphor::regular as icon;

pub const BUTTON_HEIGHT: f32 = 40.0;
const SLIDER_TRACK: Color32 = Color32::from_rgb(0x08, 0x0A, 0x0B);
const SPIN_EDGE: Color32 = Color32::from_rgb(0x0F, 0x0F, 0x11);
const SPIN_ARROW_BOX: Color32 = Color32::from_rgb(0x37, 0x3A, 0x3D);

fn hovered(ui: &Ui, response: &Response) -> bool {
    ui.is_enabled() && response.hovered()
}

/// Rounded rectangular button with an accent fill when selected.
pub fn pill(ui: &mut Ui, text: &str, selected: bool, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = hovered(ui, &response);
        let fill = match (selected, hover) {
            (true, false) => ACCENT,
            (true, true) => ACCENT_HOVER,
            (false, false) => BUTTON,
            (false, true) => BUTTON_HOVER,
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        let color = if selected { ON_ACCENT } else { TEXT };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            font(14.0),
            color,
        );
    }
    response
}

/// A row of equally wide pills. Returns the index that was clicked.
pub fn pill_row(ui: &mut Ui, labels: &[&str], selected: Option<usize>, gap: f32) -> Option<usize> {
    let width = (ui.available_width() - gap * (labels.len() as f32 - 1.0)) / labels.len() as f32;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (i, label) in labels.iter().enumerate() {
            if pill(ui, label, selected == Some(i), vec2(width, BUTTON_HEIGHT)).clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// iOS-style switch with an accent fill when on.
pub fn toggle(ui: &mut Ui, on: bool) -> Response {
    let size = vec2(40.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, on);
        let fill = lerp_color(TOGGLE_OFF, ACCENT, t);
        let radius = rect.height() / 2.0;
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius as u8), fill);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if on { ON_ACCENT } else { Color32::WHITE };
        ui.painter()
            .circle_filled(pos2(x, rect.center().y), radius - 3.0, knob);
    }
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Square icon button. `active` draws the rounded accent highlight used for
/// features that are switched on.
pub fn icon_button(ui: &mut Ui, glyph: &str, size: f32, glyph_size: f32, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = hovered(ui, &response);
        if active {
            let bg = Rect::from_center_size(rect.center(), Vec2::splat(glyph_size + 8.0));
            ui.painter().rect_filled(
                bg,
                CornerRadius::same(5),
                if hover { ACCENT_HOVER } else { ACCENT },
            );
        } else if hover {
            let bg = Rect::from_center_size(rect.center(), Vec2::splat(glyph_size + 8.0));
            ui.painter()
                .rect_filled(bg, CornerRadius::same(5), BUTTON_HOVER);
        }
        let color = match (active, hover) {
            (true, _) => ON_ACCENT,
            (false, true) => TEXT,
            (false, false) => Color32::from_gray(0xE0),
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(glyph_size),
            color,
        );
    }
    response
}

/// Small grey question-mark icon with a tooltip.
pub fn help(ui: &mut Ui, tip: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon::QUESTION,
        font(14.0),
        TEXT_DIM,
    );
    response.on_hover_text(tip);
}

/// The question-mark icon as a button that opens a longer explanation.
pub fn help_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::click());
    let color = if hovered(ui, &response) {
        TEXT
    } else {
        TEXT_DIM
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon::QUESTION,
        font(14.0),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Label used above control groups, optionally preceded by an icon.
pub fn group_label(ui: &mut Ui, glyph: Option<&str>, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(g) = glyph {
            ui.label(egui::RichText::new(g).size(16.0).color(TEXT));
        }
        ui.label(egui::RichText::new(text).size(15.0).color(TEXT));
    });
}

/// Full-width collapsible section header, with a reset icon after the
/// title when `reset` names its tooltip. Returns the toggled open state and
/// whether the reset icon was clicked.
pub fn section_header(ui: &mut Ui, title: &str, open: bool, reset: Option<&str>) -> (bool, bool) {
    let (open, reset, _) = header(ui, title, open, reset, false);
    (open, reset)
}

/// A [`section_header`] with a question-mark button after the title instead
/// of the reset icon. Returns the open state and whether the button was clicked.
pub fn section_header_help(ui: &mut Ui, title: &str, open: bool) -> (bool, bool) {
    let (open, _, help) = header(ui, title, open, None, true);
    (open, help)
}

fn header(
    ui: &mut Ui,
    title: &str,
    open: bool,
    reset: Option<&str>,
    help: bool,
) -> (bool, bool, bool) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 60.0), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, CARD);
    let title = ui.painter().text(
        pos2(rect.left() + PADDING, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        font(16.0),
        TEXT,
    );
    let caret = if open {
        icon::CARET_UP
    } else {
        icon::CARET_DOWN
    };
    ui.painter().text(
        pos2(rect.right() - PADDING - 8.0, rect.center().y),
        Align2::CENTER_CENTER,
        caret,
        font(18.0),
        TEXT,
    );
    let reset = reset.is_some_and(|tip| {
        let icon_rect = Rect::from_center_size(
            pos2(title.right() + 16.0, rect.center().y),
            Vec2::splat(22.0),
        );
        let response = ui.interact(icon_rect, response.id.with("reset"), Sense::click());
        let color = if hovered(ui, &response) {
            TEXT
        } else {
            TEXT_DIM
        };
        ui.painter().text(
            icon_rect.center(),
            Align2::CENTER_CENTER,
            icon::ARROW_COUNTER_CLOCKWISE,
            font(14.0),
            color,
        );
        response.on_hover_text(tip).clicked()
    });
    let help = help && {
        let icon_rect = Rect::from_center_size(
            pos2(title.right() + 14.0, rect.center().y),
            Vec2::splat(20.0),
        );
        let response = ui.interact(icon_rect, response.id.with("help"), Sense::click());
        let color = if hovered(ui, &response) {
            TEXT
        } else {
            TEXT_DIM
        };
        ui.painter().text(
            icon_rect.center(),
            Align2::CENTER_CENTER,
            icon::QUESTION,
            font(14.0),
            color,
        );
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    };
    (
        if response.clicked() && !reset && !help {
            !open
        } else {
            open
        },
        reset,
        help,
    )
}

/// Two or more small pills on a dark strip, like a segmented switch.
/// Returns the index that was clicked.
pub fn segmented(ui: &mut Ui, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let size = vec2(52.0, 26.0);
    let strip = vec2(size.x * labels.len() as f32 + 4.0, size.y + 4.0);
    let (rect, _) = ui.allocate_exact_size(strip, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(6), BUTTON);
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let min = rect.min + vec2(2.0 + size.x * i as f32, 2.0);
        let response = ui.put(Rect::from_min_size(min, size), |ui: &mut Ui| {
            pill(ui, label, selected == Some(i), size)
        });
        if response.clicked() && selected != Some(i) {
            clicked = Some(i);
        }
    }
    clicked
}

/// Horizontal slider with a 4-point track, an accent fill up to the handle,
/// and a white round handle. Returns the new position (0–1) on drag or click.
pub fn slider(ui: &mut Ui, rect: Rect, t: f32, enabled: bool, salt: &str) -> Option<f32> {
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
    let left = Rect::from_min_max(track.min, pos2(x, track.bottom()));
    painter.rect_filled(
        left,
        CornerRadius::same(2),
        if enabled { ACCENT } else { TEXT_FAINT },
    );
    painter.circle_filled(
        pos2(x, cy),
        radius,
        if enabled { Color32::WHITE } else { TEXT_FAINT },
    );
    let pointer = response.interact_pointer_pos()?;
    (response.dragged() || response.clicked())
        .then(|| ((pointer.x - x0) / (x1 - x0)).clamp(0.0, 1.0))
}

/// The 2-px dark line that separates the console from the sections below.
pub fn divider(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 2.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, DIVIDER);
}

/// Read-only dropdown look-alike (used for not-yet-implemented settings).
pub fn combo_box(ui: &mut Ui, text: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(6),
        FIELD,
        Stroke::new(1.0, FIELD_STROKE),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(14.0),
        TEXT,
    );
    let caret_box = Rect::from_center_size(
        pos2(rect.right() - 16.0, rect.center().y),
        Vec2::splat(20.0),
    );
    painter.rect_stroke(
        caret_box,
        CornerRadius::same(3),
        Stroke::new(1.0, FIELD_STROKE),
        StrokeKind::Inside,
    );
    painter.text(
        caret_box.center(),
        Align2::CENTER_CENTER,
        icon::CARET_DOWN,
        font(13.0),
        TEXT_DIM,
    );
    response
}

/// A [`combo_box`] that opens a list of `options` below it. Returns the
/// index chosen.
pub fn dropdown(ui: &mut Ui, current: &str, options: &[&str], size: Vec2) -> Option<usize> {
    let response = combo_box(ui, current, size).on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut chosen = None;
    egui::Popup::from_toggle_button_response(&response)
        .width(size.x)
        .gap(2.0)
        .frame(
            egui::Frame::new()
                .fill(FIELD)
                .stroke(Stroke::new(1.0, FIELD_STROKE))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(0, 4)),
        )
        .show(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for (i, option) in options.iter().enumerate() {
                let (item, r) = ui.allocate_exact_size(vec2(size.x, 32.0), Sense::click());
                if r.hovered() {
                    ui.painter()
                        .rect_filled(item, CornerRadius::ZERO, BUTTON_HOVER);
                }
                let color = if *option == current {
                    ACCENT_HOVER
                } else {
                    TEXT
                };
                ui.painter().text(
                    pos2(item.left() + 12.0, item.center().y),
                    Align2::LEFT_CENTER,
                    *option,
                    font(14.0),
                    color,
                );
                if r.clicked() {
                    chosen = Some(i);
                }
            }
        });
    chosen
}

/// Lays out `add` inside `rect`, which the caller has already allocated.
/// Unlike `Ui::scope_builder` this does not move the parent's cursor, so
/// several overlays can share one allocated area.
pub fn place<R>(
    ui: &mut Ui,
    rect: Rect,
    layout: egui::Layout,
    salt: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(layout),
    );
    add(&mut child)
}

/// Short message near the bottom of the window, above `bottom_offset`.
pub fn toast(ctx: &egui::Context, message: &str, bottom_offset: f32, max_width: f32) {
    egui::Area::new(egui::Id::new("toast"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -bottom_offset))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(10, 10, 10, 235))
                .stroke(Stroke::new(1.0, SEPARATOR_LIGHT))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width(max_width);
                    ui.label(egui::RichText::new(message).size(13.0).color(TEXT));
                });
        });
}

/// Invisible borders that let the user resize an undecorated window from
/// any edge or corner.
pub fn resize_edges(ui: &mut Ui, rect: Rect, thickness: f32) {
    use egui::{CursorIcon, ResizeDirection as D, ViewportCommand};
    let t = thickness;
    let c = t * 2.0;
    let edges = [
        (
            Rect::from_min_max(rect.min, pos2(rect.left() + c, rect.top() + c)),
            D::NorthWest,
            CursorIcon::ResizeNorthWest,
        ),
        (
            Rect::from_min_max(
                pos2(rect.right() - c, rect.top()),
                pos2(rect.right(), rect.top() + c),
            ),
            D::NorthEast,
            CursorIcon::ResizeNorthEast,
        ),
        (
            Rect::from_min_max(
                pos2(rect.left(), rect.bottom() - c),
                pos2(rect.left() + c, rect.bottom()),
            ),
            D::SouthWest,
            CursorIcon::ResizeSouthWest,
        ),
        (
            Rect::from_min_max(pos2(rect.right() - c, rect.bottom() - c), rect.max),
            D::SouthEast,
            CursorIcon::ResizeSouthEast,
        ),
        (
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + t)),
            D::North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - t), rect.max),
            D::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(rect.min, pos2(rect.left() + t, rect.bottom())),
            D::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(pos2(rect.right() - t, rect.top()), rect.max),
            D::East,
            CursorIcon::ResizeHorizontal,
        ),
    ];
    // Later widgets take the pointer, so the corners (listed first) are
    // added last and win where they overlap the edges.
    for (i, (area, direction, cursor)) in edges.into_iter().enumerate().rev() {
        let response = ui.interact(area, ui.id().with(("resize-edge", i)), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direction));
            break;
        }
    }
}

/// Fills the rest of the current row/area with `color` behind `add`.
pub fn band(ui: &mut Ui, color: Color32, margin: egui::Margin, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(color)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// Read-only number field with up/down arrows. Returns +1 or -1 when an
/// arrow is clicked or the mouse wheel turns over it.
pub fn spin_box(ui: &mut Ui, rect: Rect, text: &str) -> Option<f32> {
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

/// What the user did in a [`number_box`].
pub enum NumberInput {
    /// An arrow or the mouse wheel: +1 or -1.
    Step(f32),
    /// Text typed and confirmed with Enter or by leaving the field.
    Typed(String),
}

/// [`spin_box`] whose text can also be typed.
pub fn number_box(ui: &mut Ui, rect: Rect, text: &str) -> Option<NumberInput> {
    let step = spin_box(ui, rect, "");
    let field = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + 2.0),
        pos2(rect.right() - 18.0, rect.bottom() - 2.0),
    );
    let id = ui.id().with("number-box");
    let mut buffer = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| text.to_owned());
    let color = if ui.is_enabled() { TEXT } else { TEXT_FAINT };
    let response = ui.put(
        field,
        egui::TextEdit::singleline(&mut buffer)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(font(13.0))
            .text_color(color)
            .vertical_align(egui::Align::Center),
    );
    let typed = if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buffer));
        None
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
        response.lost_focus().then_some(NumberInput::Typed(buffer))
    };
    step.map(NumberInput::Step).or(typed)
}

/// Slider with two handles over `steps` evenly spaced stops; the upper
/// handle always stays above the lower one. Returns the new stop indices
/// while a handle is dragged or the track is clicked.
pub fn range_slider(
    ui: &mut Ui,
    rect: Rect,
    (lower, upper): (usize, usize),
    steps: usize,
    salt: &str,
) -> Option<(usize, usize)> {
    let enabled = ui.is_enabled();
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
    let (x0, x1) = (rect.left() + radius, rect.right() - radius);
    let last = steps.saturating_sub(1).max(1) as f32;
    let x = |i: usize| egui::lerp(x0..=x1, (i as f32 / last).clamp(0.0, 1.0));
    let cy = rect.center().y;
    let painter = ui.painter();
    let track = Rect::from_min_max(pos2(rect.left(), cy - 2.0), pos2(rect.right(), cy + 2.0));
    painter.rect_filled(track, CornerRadius::same(2), SLIDER_TRACK);
    let fill = Rect::from_min_max(pos2(x(lower), track.top()), pos2(x(upper), track.bottom()));
    painter.rect_filled(
        fill,
        CornerRadius::same(2),
        if enabled { ACCENT } else { TEXT_FAINT },
    );
    for i in [lower, upper] {
        painter.circle_filled(
            pos2(x(i), cy),
            radius,
            if enabled { Color32::WHITE } else { TEXT_FAINT },
        );
    }
    let pointer = response.interact_pointer_pos()?;
    let handle_id = response.id.with("handle");
    if ui.input(|i| i.pointer.primary_pressed()) {
        // The nearer handle; past either end, that end's handle.
        let upper_handle =
            (pointer.x - x(upper)).abs() < (pointer.x - x(lower)).abs() || pointer.x > x(upper);
        ui.data_mut(|d| d.insert_temp(handle_id, upper_handle && pointer.x >= x(lower)));
    }
    if !(response.dragged() || response.clicked()) || steps < 2 {
        return None;
    }
    let upper_handle = ui.data(|d| d.get_temp::<bool>(handle_id)).unwrap_or(false);
    let stop = (((pointer.x - x0) / (x1 - x0)).clamp(0.0, 1.0) * last).round() as usize;
    let next = if upper_handle {
        (lower, stop.max(lower + 1).min(steps - 1))
    } else {
        (stop.min(upper.saturating_sub(1)), upper)
    };
    (next != (lower, upper)).then_some(next)
}
