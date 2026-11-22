//! AI tracking tools on the preview picture: drawing a box to track an
//! object, the tracked target's box, the AI Composition lines and the
//! first-run hint. The look follows OBSBOT Center's preview (see
//! `docs/clean-room/2026-11-22-preview-tracking.md`); the drawing code is
//! Gimbly's own.

use crate::camera::{Composition, GimbalCommand, TargetBox, Transform, clamp_composition};
use crate::preview_view::{self, from_camera, to_camera};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Layout, Painter, Pos2, Rect, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;

const GREEN: Color32 = Color32::from_rgb(0x2E, 0xE0, 0x6E);
const YELLOW: Color32 = Color32::from_rgb(0xF5, 0xB8, 0x1E);
/// A drawn box smaller than this (in points) is treated as a click.
const MIN_BOX: f32 = 10.0;
/// How close to a composition line a press grabs it.
const GRAB: f32 = 12.0;

/// Maps between the shown picture and the camera's picture.
#[derive(Clone, Copy)]
pub struct Mapping {
    pub picture: Rect,
    pub transform: Transform,
    /// See [`preview_view::crop_fraction`].
    pub crop: f32,
}

impl Mapping {
    /// The camera point under a screen position.
    pub fn camera(&self, at: Pos2) -> Pos2 {
        let size = self.picture.size().max(Vec2::splat(1.0));
        let shown = ((at - self.picture.min) / size).to_pos2();
        to_camera(shown, self.transform, self.crop)
    }

    /// The screen position of a camera point.
    pub fn screen(&self, camera: Pos2) -> Pos2 {
        let shown = from_camera(camera, self.transform, self.crop);
        self.picture.min + shown.to_vec2() * self.picture.size()
    }

    fn screen_box(&self, target: TargetBox) -> Rect {
        Rect::from_two_pos(
            self.screen(pos2(target.x0, target.y0)),
            self.screen(pos2(target.x1, target.y1)),
        )
    }

    fn camera_box(&self, a: Pos2, b: Pos2) -> TargetBox {
        let (a, b) = (self.camera(a), self.camera(b));
        TargetBox {
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        }
    }
}

/// What a double-click at `point` (camera fractions) asks for. The official
/// app cancels when it misses the tracked target's box; without the box
/// (the stream is unavailable) the camera decides from the point.
pub fn double_click(point: Pos2, tracking: bool, target: Option<TargetBox>) -> GimbalCommand {
    match target {
        Some(target) if tracking && !target.contains(point.x, point.y) => {
            GimbalCommand::CancelTarget
        }
        _ => GimbalCommand::SelectPoint {
            x: point.x,
            y: point.y,
        },
    }
}

/// The thin white box with green corner marks the official app draws
/// around the tracked target.
pub fn target_box(painter: &Painter, map: &Mapping, target: TargetBox) {
    let rect = map.screen_box(target).intersect(map.picture);
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let painter = painter.with_clip_rect(map.picture);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, Color32::from_white_alpha(200)),
        StrokeKind::Inside,
    );
    corners(&painter, rect, GREEN);
}

fn corners(painter: &Painter, rect: Rect, color: Color32) {
    let len = (rect.width().min(rect.height()) * 0.12).clamp(6.0, 22.0);
    let stroke = Stroke::new(3.0, color);
    for (corner, dx, dy) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        painter.add(Shape::line(
            vec![
                corner + vec2(0.0, dy * len),
                corner,
                corner + vec2(dx * len, 0.0),
            ],
            stroke,
        ));
    }
}

/// A dark label at the top of the picture. Returns true when its close
/// button was clicked.
pub fn banner(ui: &mut Ui, picture: Rect, text: &str, closable: bool) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font(13.0), TEXT);
    let close = if closable { 22.0 } else { 0.0 };
    let size = vec2(galley.size().x + 28.0 + close, 28.0);
    let rect = Rect::from_min_size(
        pos2(picture.center().x - size.x / 2.0, picture.top() + 10.0),
        size,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), Color32::from_black_alpha(170));
    ui.painter().galley(
        pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    if !closable {
        return false;
    }
    let button = Rect::from_center_size(
        pos2(rect.right() - 18.0, rect.center().y),
        Vec2::splat(22.0),
    );
    place(ui, button, Layout::default(), "banner-close", |ui| {
        icon_button(ui, icon::X, 22.0, 13.0, false)
            .on_hover_text("Stop selecting")
            .clicked()
    })
}

/// Selecting an object: a box drawn on the picture, waiting for ✓ or ✕
/// once the mouse is released. Positions are fractions of the picture so a
/// resized window keeps the box.
#[derive(Default)]
pub struct Selection {
    from: Option<Pos2>,
    to: Option<Pos2>,
    drawn: bool,
}

pub enum SelectOutcome {
    None,
    /// ✓: track the object in this box.
    Track(TargetBox),
    /// The banner's close button.
    Close,
}

pub fn select(ui: &mut Ui, map: &Mapping, state: &mut Selection) -> SelectOutcome {
    let picture = map.picture;
    let painter = ui.painter().with_clip_rect(picture);
    painter.rect_filled(picture, CornerRadius::ZERO, Color32::from_black_alpha(120));
    let size = picture.size().max(Vec2::splat(1.0));
    let to_fraction = |p: Pos2| ((picture.clamp(p) - picture.min) / size).to_pos2();
    let to_screen = |p: Pos2| picture.min + p.to_vec2() * size;

    let response = ui.interact(picture, ui.id().with("select-object"), Sense::drag());
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        state.from = Some(to_fraction(origin));
        state.to = state.from;
        state.drawn = false;
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        state.to = Some(to_fraction(pointer));
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    let rect = state
        .from
        .zip(state.to)
        .map(|(a, b)| Rect::from_two_pos(to_screen(a), to_screen(b)));
    if response.drag_stopped() {
        state.drawn = rect.is_some_and(|r| r.width() >= MIN_BOX && r.height() >= MIN_BOX);
        if !state.drawn {
            *state = Selection::default();
        }
    }

    let mut outcome = SelectOutcome::None;
    if let Some(rect) = rect {
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(2.0, GREEN),
            StrokeKind::Inside,
        );
        if state.drawn {
            // Under the box, or above it when it reaches the bottom.
            let y = if rect.bottom() + 34.0 <= picture.bottom() {
                rect.bottom() + 17.0
            } else {
                (rect.top() - 17.0).max(picture.top() + 12.0)
            };
            let at = |x: f32| Rect::from_center_size(pos2(x, y), Vec2::splat(28.0));
            let cancel = place(
                ui,
                at(rect.left() + 14.0),
                Layout::default(),
                "select-cancel",
                |ui| {
                    icon_button(ui, icon::X, 28.0, 18.0, false)
                        .on_hover_text("Draw again")
                        .clicked()
                },
            );
            let confirm = place(
                ui,
                at(rect.right() - 14.0),
                Layout::default(),
                "select-confirm",
                |ui| {
                    icon_button(ui, icon::CHECK, 28.0, 18.0, false)
                        .on_hover_text("Track this object")
                        .clicked()
                },
            );
            if cancel {
                *state = Selection::default();
            } else if confirm {
                outcome = SelectOutcome::Track(map.camera_box(rect.min, rect.max));
                *state = Selection::default();
            }
        }
    }
    if banner(ui, picture, "Select object for tracking", true) {
        outcome = SelectOutcome::Close;
    }
    outcome
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    /// The line that sets the horizontal place (vertical on an unturned
    /// picture).
    Horizontal,
    Vertical,
}

/// The AI Composition lines while they are being edited.
#[derive(Default)]
pub struct Composer {
    /// The values on screen once the user changed something; until then the
    /// camera's values are shown.
    edit: Option<Composition>,
    dragging: Option<Line>,
}

pub enum ComposeOutcome {
    None,
    /// A line was released here.
    Moved(Composition),
}

impl Composer {
    pub fn current(&self, camera: Option<Composition>) -> Composition {
        self.edit.or(camera).unwrap_or_default()
    }

    pub fn set(&mut self, composition: Composition) {
        self.edit = Some(composition);
    }
}

/// The two ends of `line` on screen for `composition`.
fn line_ends(map: &Mapping, composition: Composition, line: Line) -> [Pos2; 2] {
    let ends = match line {
        Line::Horizontal => {
            let x = 0.5 + composition.x;
            [pos2(x, 0.0), pos2(x, 1.0)]
        }
        Line::Vertical => {
            let y = 0.5 + composition.y;
            [pos2(0.0, y), pos2(1.0, y)]
        }
    };
    ends.map(|end| map.picture.clamp(map.screen(end)))
}

fn distance_to_segment(p: Pos2, [a, b]: [Pos2; 2]) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

pub fn compose(
    ui: &mut Ui,
    map: &Mapping,
    state: &mut Composer,
    camera: Option<Composition>,
) -> ComposeOutcome {
    let picture = map.picture;
    let painter = ui.painter().with_clip_rect(picture);
    let grid = Stroke::new(1.0, Color32::from_white_alpha(110));
    for segment in preview_view::composition_lines(picture) {
        painter.line_segment(segment, grid);
    }
    let mut composition = state.current(camera);
    let lines: Vec<Line> = if composition.face_framing {
        vec![Line::Vertical]
    } else {
        vec![Line::Horizontal, Line::Vertical]
    };

    let response = ui.interact(picture, ui.id().with("composition"), Sense::drag());
    let nearest = |p: Pos2, composition: Composition| {
        lines
            .iter()
            .map(|&line| {
                (
                    line,
                    distance_to_segment(p, line_ends(map, composition, line)),
                )
            })
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(line, _)| line)
    };
    if response.drag_started() {
        state.dragging = ui
            .input(|i| i.pointer.press_origin())
            .and_then(|p| nearest(p, composition));
    }
    let mut outcome = ComposeOutcome::None;
    if let Some(line) = state.dragging {
        if let Some(pointer) = response.interact_pointer_pos()
            && response.dragged()
        {
            let at = map.camera(pointer);
            match line {
                Line::Horizontal => composition.x = clamp_composition(at.x - 0.5),
                Line::Vertical => composition.y = clamp_composition(at.y - 0.5),
            }
            state.edit = Some(composition);
        }
        if response.drag_stopped() {
            state.dragging = None;
            outcome = ComposeOutcome::Moved(composition);
        }
    }
    let hovered = state
        .dragging
        .or_else(|| response.hover_pos().and_then(|p| nearest(p, composition)));
    if let Some(line) = hovered {
        let [a, b] = line_ends(map, composition, line);
        ui.ctx()
            .set_cursor_icon(if (b.x - a.x).abs() < (b.y - a.y).abs() {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::ResizeVertical
            });
    }

    for line in lines {
        let [a, b] = line_ends(map, composition, line);
        painter.line_segment([a, b], Stroke::new(1.5, YELLOW));
        // Short thick handles at both ends, as in the official app.
        let along = (b - a).normalized() * 22.0;
        painter.line_segment([a, a + along], Stroke::new(4.0, YELLOW));
        painter.line_segment([b - along, b], Stroke::new(4.0, YELLOW));
    }
    let text = if composition.face_framing {
        "Auto horizontal framing based on face orientation."
    } else {
        "Move lines to adjust composition position"
    };
    banner(ui, picture, text, false);
    outcome
}

/// The bottom bar while composing: Face Framing on the left, Reset and Done
/// on the right.
pub enum ComposeBar {
    None,
    FaceFraming(bool),
    Reset,
    Done,
}

pub fn compose_bar(ui: &mut Ui, bar: Rect, face_framing: bool) -> ComposeBar {
    let inner = bar.shrink2(vec2(10.0, 0.0));
    let size = vec2(92.0, 26.0);
    let mut picked = ComposeBar::None;
    place(
        ui,
        inner,
        Layout::left_to_right(egui::Align::Center),
        "compose-left",
        |ui| {
            if pill(ui, "Face Framing", face_framing, size)
                .on_hover_text("Let the camera choose the horizontal place from the face direction")
                .clicked()
            {
                picked = ComposeBar::FaceFraming(!face_framing);
            }
        },
    );
    place(
        ui,
        inner,
        Layout::right_to_left(egui::Align::Center),
        "compose-right",
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if pill(ui, "Done", true, size).clicked() {
                picked = ComposeBar::Done;
            }
            if pill(ui, "Reset", false, size)
                .on_hover_text("Back to the centre")
                .clicked()
            {
                picked = ComposeBar::Reset;
            }
        },
    );
    picked
}

/// The "AI" mark in corner brackets for the Object Tracking button.
pub fn object_button(ui: &mut Ui, size: f32, active: bool) -> egui::Response {
    let response = icon_button(ui, "", size, 16.0, active);
    let color = if !ui.is_enabled() {
        TEXT_FAINT
    } else if active {
        ON_ACCENT
    } else {
        TEXT
    };
    let mark = Rect::from_center_size(response.rect.center(), Vec2::splat(15.0));
    let painter = ui.painter();
    let len = 4.0;
    let stroke = Stroke::new(1.4, color);
    for (corner, dx, dy) in [
        (mark.left_top(), 1.0, 1.0),
        (mark.right_top(), -1.0, 1.0),
        (mark.left_bottom(), 1.0, -1.0),
        (mark.right_bottom(), -1.0, -1.0),
    ] {
        painter.add(Shape::line(
            vec![
                corner + vec2(0.0, dy * len),
                corner,
                corner + vec2(dx * len, 0.0),
            ],
            stroke,
        ));
    }
    painter.text(mark.center(), Align2::CENTER_CENTER, "AI", font(8.5), color);
    response
}

/// The first-run hint over the picture: drag to turn the gimbal,
/// double-click to pick a target. Returns true once it was clicked away.
pub fn hint(ui: &mut Ui, area: Rect) -> bool {
    let response = ui.interact(area, ui.id().with("preview-hint"), Sense::click_and_drag());
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(area, CornerRadius::ZERO, Color32::from_black_alpha(150));
    let scale = (area.width() / 960.0).clamp(0.55, 1.0);
    let gap = area.width() * 0.2;
    let left = area.center() - vec2(gap, 30.0 * scale);
    let right = area.center() + vec2(gap, -30.0 * scale);

    // A gimbal ring with four arrows around a mouse.
    let ring = Color32::from_gray(150);
    painter.circle_stroke(left, 72.0 * scale, Stroke::new(26.0 * scale, ring));
    for direction in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        let tip = left + direction * 82.0 * scale;
        let base = left + direction * 70.0 * scale;
        let side = vec2(-direction.y, direction.x) * 7.0 * scale;
        painter.add(Shape::convex_polygon(
            vec![tip, base + side, base - side],
            Color32::from_gray(225),
            Stroke::NONE,
        ));
    }
    mouse(&painter, left, scale, true);

    // A box with green corners and a clicking mouse.
    let frame = Rect::from_center_size(right - vec2(20.0, 22.0) * scale, Vec2::splat(96.0 * scale));
    painter.rect_stroke(
        frame,
        CornerRadius::ZERO,
        Stroke::new(1.0, Color32::from_white_alpha(170)),
        StrokeKind::Inside,
    );
    corners(&painter, frame, GREEN);
    let clicked = right + vec2(28.0, 26.0) * scale;
    mouse(&painter, clicked, scale, false);
    for angle in [-2.4f32, -1.9, -1.4] {
        let direction = Vec2::angled(angle);
        let start = clicked + vec2(-12.0, -26.0) * scale + direction * 8.0 * scale;
        painter.line_segment(
            [start, start + direction * 7.0 * scale],
            Stroke::new(2.0 * scale, GREEN),
        );
    }

    for (centre, text) in [
        (
            left,
            "Drag the mouse to control the direction of the gimbal",
        ),
        (right, "Double-click to select the target"),
    ] {
        painter.text(
            centre + vec2(0.0, 118.0 * scale),
            Align2::CENTER_TOP,
            text,
            font(14.0),
            TEXT,
        );
    }
    response.clicked() || response.drag_started()
}

/// A mouse with the left button pressed; `dot` marks it with a green dot.
fn mouse(painter: &Painter, centre: Pos2, scale: f32, dot: bool) {
    let body = Rect::from_center_size(centre, vec2(44.0, 66.0) * scale);
    let radius = CornerRadius::same((20.0 * scale) as u8);
    painter.rect_filled(body, radius, Color32::from_gray(160));
    painter.rect_stroke(
        body,
        radius,
        Stroke::new(2.0 * scale, Color32::from_gray(235)),
        StrokeKind::Inside,
    );
    let split = body.top() + body.height() * 0.45;
    let left = Rect::from_min_max(body.left_top(), pos2(body.center().x, split));
    painter.rect_filled(
        left.shrink(2.0 * scale),
        CornerRadius {
            nw: (18.0 * scale) as u8,
            ..CornerRadius::ZERO
        },
        Color32::WHITE,
    );
    let edge = Stroke::new(2.0 * scale, Color32::from_gray(235));
    painter.line_segment([pos2(body.left(), split), pos2(body.right(), split)], edge);
    painter.line_segment(
        [
            pos2(body.center().x, body.top()),
            pos2(body.center().x, split),
        ],
        edge,
    );
    if dot {
        painter.circle_filled(left.center() + vec2(1.0, 2.0) * scale, 5.0 * scale, GREEN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(transform: Transform) -> Mapping {
        Mapping {
            picture: Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 450.0)),
            transform,
            crop: 1.0,
        }
    }

    #[test]
    fn double_click_cancels_only_outside_the_target() {
        let target = TargetBox {
            x0: 0.2,
            y0: 0.1,
            x1: 0.6,
            y1: 1.0,
        };
        let inside = pos2(0.4, 0.5);
        let outside = pos2(0.8, 0.5);
        let select = |p: Pos2| GimbalCommand::SelectPoint { x: p.x, y: p.y };
        assert_eq!(
            double_click(outside, true, Some(target)),
            GimbalCommand::CancelTarget
        );
        assert_eq!(double_click(inside, true, Some(target)), select(inside));
        assert_eq!(double_click(outside, false, None), select(outside));
        assert_eq!(double_click(outside, true, None), select(outside));
    }

    #[test]
    fn screen_and_camera_points_agree() {
        let m = map(Transform::new(0, true));
        // Mirrored: the shown left edge is the camera's right edge.
        assert!((m.camera(pos2(100.0, 275.0)) - pos2(1.0, 0.5)).length() < 1e-5);
        assert!((m.screen(pos2(1.0, 0.5)) - pos2(100.0, 275.0)).length() < 1e-3);
        let target = m.camera_box(pos2(200.0, 100.0), pos2(300.0, 200.0));
        assert!(target.x0 < target.x1 && target.y0 < target.y1);
        let back = m.screen_box(target);
        assert!((back.min - pos2(200.0, 100.0)).length() < 1e-3);
    }

    #[test]
    fn composition_lines_follow_the_camera_axes() {
        let composition = Composition {
            x: -0.16,
            y: 0.1,
            face_framing: false,
        };
        let m = map(Transform::default());
        let [a, b] = line_ends(&m, composition, Line::Horizontal);
        assert!((a.x - (100.0 + 800.0 * 0.34)).abs() < 1e-3 && a.y == 50.0 && b.y == 500.0);
        // A quarter turn shows the camera's horizontal place as a row.
        let turned = map(Transform::new(90, false));
        let [a, b] = line_ends(&turned, composition, Line::Horizontal);
        assert!((a.y - b.y).abs() < 1e-3);
        assert!(
            distance_to_segment(pos2(500.0, 60.0), [pos2(0.0, 50.0), pos2(900.0, 50.0)]) - 10.0
                < 1e-3
        );
    }
}
