//! Gimbal joysticks: the round one in the View and Gimbal section and the
//! drag overlay of the video preview. Both report how far the pointer is
//! from their centre; `gimbal` turns that into speeds.
//!
//! The look follows OBSBOT Center's controls; the drawing is Gimbly's own.

use crate::gimbal;
use crate::theme::*;
use eframe::egui::{
    self, Align2, Color32, Painter, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Ui,
    Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

/// How often a held joystick repeats its speed, like OBSBOT Center.
pub const REPEAT_EVERY: Duration = Duration::from_millis(100);

/// What a joystick did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drag {
    /// The drag began this frame.
    pub started: bool,
    /// Pointer offset from the centre in points while dragging (x right,
    /// y down).
    pub offset: Option<Vec2>,
    /// The drag ended this frame.
    pub ended: bool,
    /// `offset` differs from the last frame's (the pointer moved).
    pub moved: bool,
    /// A speed for `offset` should go out now (it changed, or the last one
    /// is [`REPEAT_EVERY`] old).
    pub send: bool,
    /// Preview-only fine control while Shift is held.
    pub fine: bool,
}

/// Remembers a drag in progress across frames.
#[derive(Default)]
pub struct DragState {
    centre: Option<Pos2>,
    /// When the last speed went out, to repeat it every [`REPEAT_EVERY`].
    last_sent: Option<Instant>,
    last_offset: Option<Vec2>,
    fine: bool,
}

impl DragState {
    pub fn dragging(&self) -> bool {
        self.centre.is_some()
    }

    /// Records `offset` for this frame and fills in `moved` and `send`.
    fn track(&mut self, offset: Vec2, fine: bool, drag: &mut Drag) {
        drag.offset = Some(offset);
        drag.fine = fine;
        drag.moved = self.last_offset != Some(offset);
        drag.send = drag.moved
            || fine != self.fine
            || self.last_sent.is_none_or(|t| t.elapsed() >= REPEAT_EVERY);
        if drag.send {
            self.last_sent = Some(Instant::now());
        }
        self.last_offset = Some(offset);
        self.fine = fine;
    }

    fn begin(&mut self, centre: Pos2) {
        *self = DragState {
            centre: Some(centre),
            ..Default::default()
        };
    }

    fn end(&mut self) {
        *self = DragState::default();
    }
}

// Measured on OBSBOT Center's panel joystick, in points.
const OUTER_RADIUS: f32 = 72.0;
const DISC_RADIUS: f32 = 54.5;
const KNOB_RADIUS: f32 = 12.5;
/// Arrow tips sit just inside the outer edge.
const ARROW_TIP: f32 = 68.0;
const ARROW_BASE: f32 = 62.0;
const ARROW_HALF_WIDTH: f32 = 4.5;

const RING: Color32 = Color32::from_rgb(0x18, 0x1A, 0x1C);
const RING_EDGE: Color32 = Color32::from_rgb(0x2E, 0x33, 0x36);
const DISC: Color32 = Color32::from_rgb(0x2A, 0x2E, 0x31);
const ARROW: Color32 = Color32::from_rgb(0x7F, 0x7F, 0x80);
const KNOB: Color32 = Color32::from_rgb(0xD8, 0xDD, 0xE1);
const KNOB_EDGE: Color32 = Color32::from_rgb(0xCD, 0xD1, 0xD3);

/// The panel joystick, centred in `rect`. Pressing anywhere on it puts the
/// knob under the pointer, like OBSBOT Center; the knob follows the pointer
/// up to [`gimbal::PANEL_TRAVEL`] from the centre and springs back on
/// release.
pub fn panel(ui: &mut Ui, rect: Rect, state: &mut DragState, enabled: bool) -> (Response, Drag) {
    let centre = rect.center();
    let response = ui.interact(
        rect,
        ui.id().with("gimbal-joystick"),
        if enabled {
            Sense::drag()
        } else {
            Sense::hover()
        },
    );
    let mut drag = Drag::default();
    if response.is_pointer_button_down_on() && !state.dragging() {
        state.begin(centre);
        drag.started = true;
    }
    if state.dragging() {
        if response.is_pointer_button_down_on()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            state.track(pointer - centre, false, &mut drag);
        } else if !response.is_pointer_button_down_on() {
            state.end();
            drag.ended = true;
        }
    }

    let painter = ui.painter();
    painter.circle(centre, OUTER_RADIUS, RING, Stroke::new(1.0, RING_EDGE));
    painter.circle_filled(centre, DISC_RADIUS, DISC);
    let arrow = if enabled { ARROW } else { TEXT_FAINT };
    for direction in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        triangle(painter, centre, direction, arrow);
    }
    let knob_offset = drag
        .offset
        .map_or(Vec2::ZERO, |o| clamp_length(o, gimbal::PANEL_TRAVEL));
    knob(painter, centre + knob_offset, KNOB_RADIUS, enabled);
    (response, drag)
}

// Measured on OBSBOT Center's preview overlay, in points.
const OVERLAY_OUTER: f32 = 77.0;
const OVERLAY_INNER: f32 = 55.0;
const OVERLAY_KNOB: f32 = 11.5;
const OVERLAY_ARROW_TIP: f32 = 73.0;
const OVERLAY_ARROW_BASE: f32 = 66.0;
/// Pointer travel before a press on the picture counts as a drag.
const OVERLAY_THRESHOLD: f32 = 4.0;
const RELEASE_SECONDS: f64 = 0.18;

/// Preview animation is separate from the active drag so it cannot move the camera.
#[derive(Default)]
pub struct OverlayState {
    drag: DragState,
    released: Option<ReleasedOverlay>,
    blocked_until_release: bool,
}

struct ReleasedOverlay {
    centre: Pos2,
    offset: Vec2,
    fine: bool,
    at: f64,
}

impl ReleasedOverlay {
    /// Cubic return of the knob and a smooth fade of the whole overlay.
    fn sample(&self, now: f64) -> Option<(Vec2, f32)> {
        let t = ((now - self.at) / RELEASE_SECONDS).clamp(0.0, 1.0) as f32;
        (t < 1.0).then(|| {
            (
                self.offset * (1.0 - t).powi(3),
                1.0 - t * t * (3.0 - 2.0 * t),
            )
        })
    }
}

impl OverlayState {
    /// Ends control immediately when the preview closes or becomes unavailable.
    pub fn cancel(&mut self) -> Drag {
        let ended = self.drag.dragging();
        // A held button must not restart control when focus or streaming returns.
        *self = Self {
            blocked_until_release: ended || self.blocked_until_release,
            ..Default::default()
        };
        Drag {
            ended,
            ..Default::default()
        }
    }
}

/// The preview's drag-to-turn overlay over `rect`. Pressing on the picture
/// and dragging shows a translucent ring centred where the drag began, with
/// the knob under the pointer. Shift slows control; release animates only the drawing.
pub fn overlay(ui: &mut Ui, rect: Rect, state: &mut OverlayState, enabled: bool) -> Drag {
    // Cancel on the focus-lost event, not on `!focused`: the click that
    // focuses the preview arrives before the focus event and must start a drag.
    let focus_lost = ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(false)))
    });
    let enabled = enabled && ui.is_enabled() && !focus_lost;
    let response = ui.interact(
        rect,
        ui.id().with("gimbal-overlay"),
        if enabled {
            Sense::drag()
        } else {
            Sense::hover()
        },
    );
    if !enabled {
        return state.cancel();
    }
    if state.blocked_until_release {
        if !ui.input(|i| i.pointer.primary_down()) {
            state.blocked_until_release = false;
        }
        return Drag::default();
    }
    let mut drag = Drag::default();
    let (press, fine, now) = ui.input(|i| (i.pointer.press_origin(), i.modifiers.shift, i.time));
    if !state.drag.dragging()
        && response.dragged_by(PointerButton::Primary)
        && let (Some(origin), Some(pointer)) = (press, response.interact_pointer_pos())
        && rect.contains(origin)
        && pointer.distance(origin) >= OVERLAY_THRESHOLD
    {
        state.drag.begin(origin);
        state.released = None;
        drag.started = true;
    }
    let mut painter = ui.painter().with_clip_rect(rect);
    if let Some(centre) = state.drag.centre {
        if response.dragged_by(PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let offset = pointer - centre;
            state.drag.track(offset, fine, &mut drag);
            draw_overlay(&painter, centre, offset, fine);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            ui.ctx().request_repaint_after(REPEAT_EVERY);
        } else {
            state.released = Some(ReleasedOverlay {
                centre,
                offset: clamp_length(
                    state.drag.last_offset.unwrap_or_default(),
                    (OVERLAY_OUTER + OVERLAY_INNER) / 2.0,
                ),
                fine: state.drag.fine,
                at: now,
            });
            state.drag.end();
            drag.ended = true;
        }
    }
    if let Some(released) = &state.released {
        if let Some((offset, opacity)) = released.sample(now) {
            painter.set_opacity(opacity);
            draw_overlay(&painter, released.centre, offset, released.fine);
            ui.ctx().request_repaint();
        } else {
            state.released = None;
        }
    }
    response.on_hover_text("Drag to turn the camera. Hold Shift for fine control (1/3 speed).");
    drag
}

fn draw_overlay(painter: &Painter, centre: Pos2, offset: Vec2, fine: bool) {
    let band = (OVERLAY_OUTER + OVERLAY_INNER) / 2.0;
    painter.circle_stroke(
        centre,
        band,
        Stroke::new(OVERLAY_OUTER - OVERLAY_INNER, Color32::from_white_alpha(70)),
    );
    painter.circle_filled(centre, OVERLAY_INNER, Color32::from_white_alpha(28));
    for direction in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        let tip = centre + direction * OVERLAY_ARROW_TIP;
        let base = centre + direction * OVERLAY_ARROW_BASE;
        let side = vec2(-direction.y, direction.x) * 5.0;
        painter.add(Shape::convex_polygon(
            vec![tip, base + side, base - side],
            Color32::from_white_alpha(200),
            Stroke::NONE,
        ));
    }
    let knob_at = centre + clamp_length(offset, band);
    painter.circle_filled(knob_at, OVERLAY_KNOB, Color32::from_white_alpha(235));
    if fine {
        let badge =
            Rect::from_center_size(centre + vec2(0.0, OVERLAY_OUTER + 17.0), vec2(82.0, 22.0));
        painter.rect_filled(badge, 5.0, Color32::from_black_alpha(170));
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            "Fine (Shift)",
            font(12.0),
            TEXT,
        );
    }
}

fn triangle(painter: &Painter, centre: Pos2, direction: Vec2, color: Color32) {
    let tip = centre + direction * ARROW_TIP;
    let base = centre + direction * ARROW_BASE;
    let side = vec2(-direction.y, direction.x) * ARROW_HALF_WIDTH;
    painter.add(Shape::convex_polygon(
        vec![tip, base + side, base - side],
        color,
        Stroke::NONE,
    ));
}

fn knob(painter: &Painter, at: Pos2, radius: f32, enabled: bool) {
    let (fill, edge) = if enabled {
        (KNOB, KNOB_EDGE)
    } else {
        (TEXT_FAINT, TEXT_FAINT)
    };
    // A soft shadow and a lighter top half give the knob some depth.
    painter.circle_filled(
        at + vec2(0.0, 1.5),
        radius + 1.0,
        Color32::from_black_alpha(70),
    );
    painter.circle(at, radius, edge, Stroke::NONE);
    painter.circle_filled(pos2(at.x, at.y - radius * 0.15), radius * 0.82, fill);
}

fn clamp_length(v: Vec2, max: f32) -> Vec2 {
    let length = v.length();
    if length > max { v * (max / length) } else { v }
}

/// Size the panel joystick needs (OBSBOT Center's widget is 148x156).
pub const PANEL_SIZE: Vec2 = vec2(148.0, 156.0);

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, FullOutput, Modifiers, RawInput};

    fn frame(
        ctx: &Context,
        state: &mut OverlayState,
        time: f64,
        events: Vec<Event>,
        enabled: bool,
    ) -> (Drag, FullOutput) {
        let mut drag = Drag::default();
        let mut output = ctx.run_ui(
            RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let rect = ui.max_rect();
                drag = overlay(ui, rect, state, enabled);
            },
        );
        output.textures_delta.clear();
        (drag, output)
    }

    fn button(pressed: bool) -> Event {
        Event::PointerButton {
            pos: pos2(200.0, 200.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn start_drag() -> (Context, OverlayState) {
        let ctx = Context::default();
        let mut state = OverlayState::default();
        frame(
            &ctx,
            &mut state,
            0.0,
            vec![Event::PointerMoved(pos2(200.0, 200.0))],
            true,
        );
        frame(&ctx, &mut state, 0.01, vec![button(true)], true);
        let (drag, output) = frame(
            &ctx,
            &mut state,
            0.02,
            vec![Event::PointerMoved(pos2(240.0, 180.0))],
            true,
        );
        assert!(drag.started && drag.send);
        assert_eq!(drag.offset, Some(vec2(40.0, -20.0)));
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::Default
        );
        (ctx, state)
    }

    #[test]
    fn clamps_long_offsets_only() {
        assert_eq!(clamp_length(vec2(3.0, 4.0), 10.0), vec2(3.0, 4.0));
        assert_eq!(clamp_length(vec2(30.0, 40.0), 10.0), vec2(6.0, 8.0));
    }

    #[test]
    fn repeats_unchanged_speeds_slowly() {
        let mut state = DragState::default();
        state.begin(Pos2::ZERO);
        let mut track = |x| {
            let mut drag = Drag::default();
            state.track(vec2(x, 0.0), false, &mut drag);
            (drag.moved, drag.send)
        };
        assert_eq!(track(1.0), (true, true));
        assert_eq!(track(1.0), (false, false));
        assert_eq!(track(2.0), (true, true));
    }

    #[test]
    fn shift_changes_speed_without_a_pointer_move() {
        let (ctx, mut state) = start_drag();
        for (time, modifiers, fine) in [
            (0.03, Modifiers::SHIFT, true),
            (0.04, Modifiers::NONE, false),
        ] {
            let (drag, _) = frame(
                &ctx,
                &mut state,
                time,
                vec![Event::ModifiersChanged(modifiers)],
                true,
            );
            assert!(drag.send && !drag.moved);
            assert_eq!(drag.fine, fine);
            assert_eq!(drag.offset, Some(vec2(40.0, -20.0)));
        }
    }

    #[test]
    fn release_stops_before_animation_and_animation_emits_no_motion() {
        let (ctx, mut state) = start_drag();
        let (drag, _) = frame(&ctx, &mut state, 0.03, vec![button(false)], true);
        assert_eq!(
            drag,
            Drag {
                ended: true,
                ..Default::default()
            }
        );
        assert!(!state.drag.dragging());
        let release = state.released.as_ref().unwrap();
        let (first, opacity) = release.sample(0.03).unwrap();
        assert_eq!(first, vec2(40.0, -20.0));
        assert_eq!(opacity, 1.0);
        let (later, opacity) = release.sample(0.12).unwrap();
        assert!(later.length() < first.length());
        assert!((opacity - 0.5).abs() < 0.01);
        for time in [0.04, 0.12, 0.22, 0.3] {
            let (drag, _) = frame(&ctx, &mut state, time, Vec::new(), true);
            assert_eq!(drag, Drag::default());
        }
        assert!(state.released.is_none());
    }

    #[test]
    fn disabling_or_cancelling_the_preview_stops_once_and_clears_animation() {
        let (ctx, mut state) = start_drag();
        let (drag, _) = frame(&ctx, &mut state, 0.03, Vec::new(), false);
        assert!(drag.ended && !drag.send);
        assert!(!state.drag.dragging() && state.released.is_none());
        assert_eq!(state.cancel(), Drag::default());

        let (_, mut state) = start_drag();
        assert!(state.cancel().ended);
        assert_eq!(state.cancel(), Drag::default());
    }

    #[test]
    fn losing_focus_stops_without_restarting_on_focus_return() {
        let (ctx, mut state) = start_drag();
        let mut drag = Drag::default();
        let mut output = ctx.run_ui(
            RawInput {
                time: Some(0.03),
                focused: false,
                events: vec![Event::WindowFocused(false)],
                ..Default::default()
            },
            |ui| {
                let rect = ui.max_rect();
                drag = overlay(ui, rect, &mut state, true);
            },
        );
        output.textures_delta.clear();
        assert!(drag.ended && !drag.send);
        let (drag, _) = frame(&ctx, &mut state, 0.04, Vec::new(), true);
        assert_eq!(drag, Drag::default());
        assert!(state.released.is_none());
    }

    #[test]
    fn the_click_that_focuses_the_preview_starts_a_drag() {
        let ctx = Context::default();
        let mut state = OverlayState::default();
        let mut unfocused = |time, events| {
            ctx.run_ui(
                RawInput {
                    time: Some(time),
                    focused: false,
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let rect = ui.max_rect();
                    assert_eq!(overlay(ui, rect, &mut state, true), Drag::default());
                },
            )
            .textures_delta
            .clear();
        };
        unfocused(0.0, vec![Event::PointerMoved(pos2(200.0, 200.0))]);
        unfocused(0.01, vec![button(true)]);
        let (drag, _) = frame(
            &ctx,
            &mut state,
            0.02,
            vec![
                Event::WindowFocused(true),
                Event::PointerMoved(pos2(240.0, 180.0)),
            ],
            true,
        );
        assert!(drag.started && drag.send);
        assert_eq!(drag.offset, Some(vec2(40.0, -20.0)));
    }

    #[test]
    fn a_new_drag_replaces_the_release_animation() {
        let (ctx, mut state) = start_drag();
        frame(&ctx, &mut state, 0.03, vec![button(false)], true);
        assert!(state.released.is_some());
        frame(&ctx, &mut state, 0.04, vec![button(true)], true);
        let (drag, _) = frame(
            &ctx,
            &mut state,
            0.05,
            vec![Event::PointerMoved(pos2(180.0, 220.0))],
            true,
        );
        assert!(drag.started && drag.send);
        assert_eq!(drag.offset, Some(vec2(-20.0, 20.0)));
        assert!(state.released.is_none());
    }
}
