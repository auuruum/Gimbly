//! The Exposure section and the Anti-Flicker row of the Image tab.

use super::{GimblyApp, switch_row};
use crate::camera::{
    AntiFlicker, COMPENSATION_MAX, COMPENSATION_ZERO, Exposure, Feature, FocusMode, GimbalCommand,
    ISO_LIMITS, ISO_MAX, SHUTTER_SPEEDS,
};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

/// How long a value the user picked is shown before the camera's own
/// reading takes over again (when a write failed).
const TARGET_HOLD: Duration = Duration::from_secs(2);
/// Curve points stay this far from the fixed ends, as in the official app.
const CURVE_X_MIN: u8 = 8;
const CURVE_X_MAX: u8 = 247;
/// A press this close to a point grabs it; this close to the curve adds one.
const GRAB: f32 = 8.0;
const NEAR_CURVE: f32 = 10.0;

/// The exposure curve editor's own state.
#[derive(Default)]
pub(super) struct CurveUi {
    selected: Option<usize>,
    dragging: bool,
    /// Points the user set, until the camera reports them.
    target: Option<(Vec<[u8; 2]>, Instant)>,
}

impl GimblyApp {
    /// The Exposure section: Auto Exposure with its mode, compensation and
    /// ISO limits, or the manual shutter, ISO and curve.
    pub(super) fn exposure(&mut self, ui: &mut Ui) {
        let ready = self.ready(Feature::Exposure) && self.status.exposure.is_some();
        let reset;
        (self.exposure_open, reset) = section_header(
            ui,
            "Exposure",
            self.exposure_open,
            ready.then_some("Reset to Auto Exposure on faces"),
        );
        // Like the official app: reset turns Auto Exposure on in Face mode and
        // restores compensation and the ISO limits; Anti-Flicker and the
        // manual values stay.
        if reset {
            self.exposure_target = None;
            self.gimbal(GimbalCommand::SetExposureMode(FocusMode::Face));
            self.gimbal(GimbalCommand::SetAutoExposure(true));
            self.gimbal(GimbalCommand::SetIsoLimits(ISO_LIMITS[0], ISO_LIMITS[7]));
            self.gimbal(GimbalCommand::SetCompensation(COMPENSATION_ZERO));
        }
        if !self.exposure_open {
            return;
        }
        let shown = self.shown_exposure();
        let tip = self.gimbal_tip(Feature::Exposure);
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 4,
            bottom: 18,
        };
        let auto = shown.is_none_or(|e| e.auto);
        let mut commands = Vec::new();
        let mut next = shown;
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_enabled_ui(ready, |ui| {
                let (switch, _) = switch_row(
                    ui,
                    "Auto Exposure",
                    "Sets the brightness by itself. Turn it off to set the shutter speed and ISO by hand",
                    None,
                    auto,
                );
                if switch.on_disabled_hover_text(&tip).clicked() {
                    commands.push(GimbalCommand::SetAutoExposure(!auto));
                }
                let Some(exposure) = &mut next else {
                    return;
                };
                if auto {
                    self.auto_rows(ui, exposure, &mut commands);
                } else {
                    manual_rows(ui, exposure, &mut commands);
                    self.curve(ui, &mut commands);
                }
            });
        });
        if next != shown
            && let Some(next) = next
        {
            self.exposure_target = Some((next, Instant::now()));
        }
        for command in commands {
            self.gimbal(command);
        }
    }

    /// The exposure to show: the user's latest pick until the camera reports it.
    fn shown_exposure(&mut self) -> Option<Exposure> {
        let camera = self.status.exposure;
        match self.exposure_target {
            Some((target, at)) if camera != Some(target) && at.elapsed() < TARGET_HOLD => {
                Some(target)
            }
            _ => {
                self.exposure_target = None;
                camera
            }
        }
    }

    fn auto_rows(&self, ui: &mut Ui, exposure: &mut Exposure, commands: &mut Vec<GimbalCommand>) {
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.label(egui::RichText::new("Auto Exposure Mode").size(15.0));
            help(
                ui,
                "Global meters the whole picture, Face meters faces first",
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let selected = self
                    .status
                    .exposure_mode
                    .map(|m| usize::from(m == FocusMode::Face));
                match segmented(ui, &["Global", "Face"], selected) {
                    Some(0) => commands.push(GimbalCommand::SetExposureMode(FocusMode::Global)),
                    Some(_) => commands.push(GimbalCommand::SetExposureMode(FocusMode::Face)),
                    None => {}
                }
            });
        });
        let value = exposure.compensation;
        let t = f32::from(value) / f32::from(COMPENSATION_MAX);
        let (dragged, input) = slider_row(ui, "Compensation", t, &compensation_text(value));
        let picked = dragged
            .map(|t| (t * f32::from(COMPENSATION_MAX)).round() as u8)
            .or_else(|| match input? {
                NumberInput::Step(step) => Some(step_u8(value, step, COMPENSATION_MAX)),
                NumberInput::Typed(text) => parse_compensation(&text),
            });
        if let Some(picked) = picked.filter(|&p| p != value) {
            exposure.compensation = picked;
            commands.push(GimbalCommand::SetCompensation(picked));
        }
        ui.label(egui::RichText::new("ISO Upper and Lower Limit").size(15.0));
        let (lower, upper) = exposure.iso_limits;
        let index = |iso: u16| ISO_LIMITS.iter().position(|&v| v == iso).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.set_height(50.0);
            let limit = |ui: &mut Ui, value: u16, name: &str| {
                ui.allocate_ui_with_layout(
                    vec2(70.0, 50.0),
                    Layout::top_down(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(egui::RichText::new(value.to_string()).size(13.0));
                        ui.label(egui::RichText::new(name).size(12.0).color(TEXT_DIM));
                    },
                );
            };
            limit(ui, lower, "ISO Lower\nLimit");
            let width = (ui.available_width() - 78.0).max(40.0);
            let (track, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
            let picked = range_slider(
                ui,
                track,
                (index(lower), index(upper)),
                ISO_LIMITS.len(),
                "iso-limits",
            );
            limit(ui, upper, "ISO Upper\nLimit");
            if let Some((lo, hi)) = picked
                && let (Some(&lo), Some(&hi)) = (ISO_LIMITS.get(lo), ISO_LIMITS.get(hi))
            {
                exposure.iso_limits = (lo, hi);
                commands.push(GimbalCommand::SetIsoLimits(lo, hi));
            }
        });
    }

    /// The Exposure Curve title with its reset and delete icons, and the graph.
    fn curve(&mut self, ui: &mut Ui, commands: &mut Vec<GimbalCommand>) {
        let camera = self.status.exposure_curve.clone().unwrap_or_default();
        let points = match &self.curve_ui.target {
            Some((target, at))
                if *target != camera && (self.curve_ui.dragging || at.elapsed() < TARGET_HOLD) =>
            {
                target.clone()
            }
            _ => {
                self.curve_ui.target = None;
                camera
            }
        };
        if self.curve_ui.selected.is_some_and(|i| i >= points.len()) {
            self.curve_ui.selected = None;
        }
        let mut next = None;
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.label(egui::RichText::new("Exposure Curve").size(15.0));
            help(
                ui,
                "Bends the brightness of dark and bright parts. Click the line to add a point, drag it, or select it and delete it",
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let selected = self.curve_ui.selected;
                let delete = ui
                    .add_enabled_ui(selected.is_some(), |ui| {
                        icon_button(ui, icon::TRASH, 22.0, 14.0, false)
                            .on_hover_text("Delete the selected point")
                    })
                    .inner;
                if delete.clicked()
                    && let Some(i) = selected
                {
                    let mut rest = points.clone();
                    rest.remove(i);
                    self.curve_ui.selected = None;
                    next = Some(rest);
                }
                if icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, 22.0, 14.0, false)
                    .on_hover_text("Straight line")
                    .clicked()
                {
                    self.curve_ui.selected = None;
                    next = Some(Vec::new());
                }
            });
        });
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 130.0), Sense::hover());
        if let Some(edited) = curve_editor(ui, rect, &points, &mut self.curve_ui) {
            next = Some(edited);
        }
        if let Some(next) = next {
            self.curve_ui.target = Some((next.clone(), Instant::now()));
            commands.push(GimbalCommand::SetExposureCurve(next));
        }
    }

    /// Anti-Flicker: Off, 50 Hz or 60 Hz. Like the official app, it only
    /// works while Auto Exposure is on and stays visible when Exposure is folded.
    pub(super) fn anti_flicker(&mut self, ui: &mut Ui) {
        let flicker = self.status.anti_flicker;
        let auto = self.shown_exposure().is_some_and(|e| e.auto);
        let ready = self.ready(Feature::Exposure) && flicker.is_some() && auto;
        let tip = if self.ready(Feature::Exposure) && !auto {
            "Anti-Flicker works while Auto Exposure is on".to_owned()
        } else {
            self.gimbal_tip(Feature::Exposure)
        };
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 14,
            bottom: 14,
        };
        let mut picked = None;
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            ui.add_enabled_ui(ready, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(30.0);
                    ui.label(egui::RichText::new("Anti-Flicker").size(15.0))
                        .on_disabled_hover_text(&tip);
                    help(
                        ui,
                        "Matches the shutter to the mains frequency so lamps do not flicker in the picture",
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let selected = flicker.map(|f| match f {
                            AntiFlicker::Off => 0,
                            AntiFlicker::Hz50 => 1,
                            AntiFlicker::Hz60 => 2,
                        });
                        picked = segmented(ui, &["Off", "50 Hz", "60 Hz"], selected);
                    });
                });
            });
        });
        let flicker = match picked {
            Some(0) => AntiFlicker::Off,
            Some(1) => AntiFlicker::Hz50,
            Some(_) => AntiFlicker::Hz60,
            None => return,
        };
        self.gimbal(GimbalCommand::SetAntiFlicker(flicker));
    }
}

/// Shutter Speed and ISO, shown while Auto Exposure is off.
fn manual_rows(ui: &mut Ui, exposure: &mut Exposure, commands: &mut Vec<GimbalCommand>) {
    // Slowest shutter on the right, as in the official app; up arrows go faster.
    let last = SHUTTER_SPEEDS.len() - 1;
    let index = shutter_index(exposure.shutter);
    let denominator = SHUTTER_SPEEDS.get(index).map_or(0, |(d, _)| *d);
    let t = 1.0 - index as f32 / last as f32;
    let (dragged, input) = slider_row(ui, "Shutter\nSpeed", t, &format!("1/{denominator}"));
    let picked = dragged
        .map(|t| ((1.0 - t) * last as f32).round() as usize)
        .or_else(|| match input? {
            NumberInput::Step(step) => Some(if step > 0.0 {
                (index + 1).min(last)
            } else {
                index.saturating_sub(1)
            }),
            NumberInput::Typed(text) => parse_shutter(&text),
        });
    if let Some(&(_, units)) = picked
        .filter(|&p| p != index)
        .and_then(|p| SHUTTER_SPEEDS.get(p))
    {
        exposure.shutter = units;
        commands.push(GimbalCommand::SetShutter(units));
    }

    let iso = exposure.iso;
    let t = f32::from(iso - 100) / f32::from(ISO_MAX - 100);
    let (dragged, input) = slider_row(ui, "ISO", t, &iso.to_string());
    let picked = dragged
        .map(|t| 100 + (t * f32::from(ISO_MAX - 100) / 100.0).round() as u16 * 100)
        .or_else(|| match input? {
            NumberInput::Step(step) => Some(if step > 0.0 {
                (iso + 100).min(ISO_MAX)
            } else {
                iso.saturating_sub(100).max(100)
            }),
            NumberInput::Typed(text) => parse_iso(&text),
        });
    if let Some(picked) = picked.filter(|&p| p != iso) {
        exposure.iso = picked;
        commands.push(GimbalCommand::SetIso(picked));
    }
}

/// A label with a slider and a number field on the right. Returns the
/// slider position (0–1) while dragged, and what was done in the field.
pub(super) fn slider_row(
    ui: &mut Ui,
    label: &str,
    t: f32,
    text: &str,
) -> (Option<f32>, Option<NumberInput>) {
    ui.horizontal(|ui| {
        ui.set_height(30.0);
        ui.label(egui::RichText::new(label).size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (field, _) = ui.allocate_exact_size(vec2(74.0, 25.0), Sense::hover());
            let input = place(ui, field, Layout::default(), (label, "field"), |ui| {
                number_box(ui, field, text)
            });
            let (track, _) = ui.allocate_exact_size(vec2(100.0, 20.0), Sense::hover());
            let enabled = ui.is_enabled();
            (slider(ui, track, t, enabled, label), input)
        })
        .inner
    })
    .inner
}

/// The [`SHUTTER_SPEEDS`] entry nearest to a value from the camera.
fn shutter_index(units: u32) -> usize {
    (0..SHUTTER_SPEEDS.len())
        .min_by_key(|&i| SHUTTER_SPEEDS[i].1.abs_diff(units))
        .unwrap_or(0)
}

/// Typed shutter text, `1/250` or `250`, to the nearest offered speed.
fn parse_shutter(text: &str) -> Option<usize> {
    let text = text.trim();
    let denominator: u32 = text
        .strip_prefix("1/")
        .unwrap_or(text)
        .trim()
        .parse()
        .ok()?;
    (0..SHUTTER_SPEEDS.len()).min_by_key(|&i| u32::from(SHUTTER_SPEEDS[i].0).abs_diff(denominator))
}

/// Typed ISO, rounded down to a hundred; values below 100 are ignored.
fn parse_iso(text: &str) -> Option<u16> {
    let iso: u32 = text.trim().parse().ok()?;
    (iso >= 100).then(|| (iso.min(u32::from(ISO_MAX)) / 100 * 100) as u16)
}

/// Compensation in stops, as shown: `0`, `1`, `-1.3`, `2.7`.
fn compensation_text(value: u8) -> String {
    let thirds = i32::from(value) - i32::from(COMPENSATION_ZERO);
    if thirds % 3 == 0 {
        (thirds / 3).to_string()
    } else {
        format!("{:.1}", thirds as f32 / 3.0)
    }
}

/// Typed stops to the compensation value, towards zero to the nearest third.
fn parse_compensation(text: &str) -> Option<u8> {
    let stops: f32 = text.trim().parse().ok()?;
    if !stops.is_finite() {
        return None;
    }
    let value = (stops * 3.0).trunc() + f32::from(COMPENSATION_ZERO);
    Some(value.clamp(0.0, f32::from(COMPENSATION_MAX)) as u8)
}

pub(super) fn step_u8(value: u8, step: f32, max: u8) -> u8 {
    if step > 0.0 {
        value.saturating_add(1).min(max)
    } else {
        value.saturating_sub(1)
    }
}

/// The curve through (0, 0), the points and (255, 255), at `x` (0–255):
/// a monotone cubic (Fritsch–Carlson), so it never overshoots a point.
fn curve_value(points: &[[u8; 2]], x: f32) -> f32 {
    let mut knots: Vec<[f32; 2]> = vec![[0.0, 0.0]];
    knots.extend(points.iter().map(|p| [f32::from(p[0]), f32::from(p[1])]));
    knots.push([255.0, 255.0]);
    let n = knots.len();
    let slopes: Vec<f32> = knots
        .windows(2)
        .map(|w| {
            let dx = w[1][0] - w[0][0];
            if dx > 0.0 {
                (w[1][1] - w[0][1]) / dx
            } else {
                0.0
            }
        })
        .collect();
    let mut tangents = vec![0.0f32; n];
    tangents[0] = slopes[0];
    tangents[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        let (a, b) = (slopes[i - 1], slopes[i]);
        tangents[i] = if a * b <= 0.0 {
            0.0
        } else {
            // Harmonic mean keeps the piece monotone.
            2.0 * a * b / (a + b)
        };
    }
    let x = x.clamp(0.0, 255.0);
    let i = knots.windows(2).position(|w| x <= w[1][0]).unwrap_or(n - 2);
    let ([x0, y0], [x1, y1]) = (knots[i], knots[i + 1]);
    let h = x1 - x0;
    if h <= 0.0 {
        return y0;
    }
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
        + (t3 - 2.0 * t2 + t) * h * tangents[i]
        + (-2.0 * t3 + 3.0 * t2) * y1
        + (t3 - t2) * h * tangents[i + 1];
    y.clamp(0.0, 255.0)
}

/// The curve graph. A press near a point selects and drags it; near the
/// line it adds a point there; elsewhere it clears the selection. Returns
/// the new points when they change.
fn curve_editor(
    ui: &mut Ui,
    rect: Rect,
    points: &[[u8; 2]],
    state: &mut CurveUi,
) -> Option<Vec<[u8; 2]>> {
    let enabled = ui.is_enabled();
    let response = ui.interact(
        rect,
        ui.id().with("exposure-curve"),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let to_screen = |x: f32, y: f32| {
        pos2(
            egui::lerp(rect.left()..=rect.right(), x / 255.0),
            egui::lerp(rect.bottom()..=rect.top(), y / 255.0),
        )
    };
    let to_value = |p: egui::Pos2| {
        (
            (p.x - rect.left()) / rect.width() * 255.0,
            (rect.bottom() - p.y) / rect.height() * 255.0,
        )
    };
    let mut next = None;
    if let Some(pointer) = response.interact_pointer_pos() {
        if ui.input(|i| i.pointer.primary_pressed()) {
            let hit = points.iter().position(|p| {
                to_screen(f32::from(p[0]), f32::from(p[1])).distance(pointer) <= GRAB
            });
            let (x, _) = to_value(pointer);
            let on_line = (to_screen(x, curve_value(points, x)).y - pointer.y).abs() <= NEAR_CURVE;
            state.dragging = false;
            if let Some(i) = hit {
                state.selected = Some(i);
                state.dragging = true;
            } else if on_line {
                let x = x
                    .round()
                    .clamp(f32::from(CURVE_X_MIN), f32::from(CURVE_X_MAX))
                    as u8;
                if points.iter().all(|p| p[0] != x) {
                    let y = curve_value(points, f32::from(x)).round() as u8;
                    let mut added = points.to_vec();
                    let at = added.partition_point(|p| p[0] < x);
                    added.insert(at, [x, y]);
                    state.selected = Some(at);
                    state.dragging = true;
                    next = Some(added);
                }
            } else {
                state.selected = None;
            }
        } else if state.dragging
            && response.dragged()
            && let Some(i) = state.selected
            && i < points.len()
        {
            let (x, y) = to_value(pointer);
            let low = if i == 0 {
                CURVE_X_MIN
            } else {
                points[i - 1][0].saturating_add(1)
            };
            let high = points
                .get(i + 1)
                .map_or(CURVE_X_MAX, |p| p[0].saturating_sub(1));
            let x = (x.round().clamp(0.0, 255.0) as u8).clamp(low, high.max(low));
            let y = y.round().clamp(0.0, 255.0) as u8;
            if points[i] != [x, y] {
                let mut moved = points.to_vec();
                moved[i] = [x, y];
                next = Some(moved);
            }
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        state.dragging = false;
    }

    let shown = next.as_deref().unwrap_or(points);
    let painter = ui.painter_at(rect.expand(6.0));
    painter.rect(
        rect,
        CornerRadius::same(2),
        SECTION,
        Stroke::new(1.0, FIELD_STROKE),
        StrokeKind::Inside,
    );
    let grid = Stroke::new(1.0, SEPARATOR_LIGHT);
    for i in 1..4 {
        let f = i as f32 * 255.0 / 4.0;
        painter.extend(egui::Shape::dashed_line(
            &[to_screen(f, 0.0), to_screen(f, 255.0)],
            grid,
            3.0,
            3.0,
        ));
        painter.extend(egui::Shape::dashed_line(
            &[to_screen(0.0, f), to_screen(255.0, f)],
            grid,
            3.0,
            3.0,
        ));
    }
    painter.line_segment(
        [to_screen(0.0, 0.0), to_screen(255.0, 255.0)],
        Stroke::new(1.0, TEXT_FAINT),
    );
    let selected = state.selected.filter(|&i| i < shown.len());
    if let Some(p) = selected.and_then(|i| shown.get(i)) {
        let at = to_screen(f32::from(p[0]), f32::from(p[1]));
        let red = Stroke::new(1.0, ACCENT);
        painter.line_segment([pos2(at.x, rect.top()), pos2(at.x, rect.bottom())], red);
        painter.line_segment([pos2(rect.left(), at.y), pos2(rect.right(), at.y)], red);
    }
    let line: Vec<_> = (0..=64)
        .map(|i| {
            let x = i as f32 * 255.0 / 64.0;
            to_screen(x, curve_value(shown, x))
        })
        .collect();
    let color = if enabled { TEXT } else { TEXT_FAINT };
    painter.add(egui::Shape::line(line, Stroke::new(1.5, color)));
    for end in [to_screen(0.0, 0.0), to_screen(255.0, 255.0)] {
        painter.circle_filled(end, 2.5, TEXT_DIM);
    }
    for (i, p) in shown.iter().enumerate() {
        let fill = if Some(i) == selected {
            ACCENT
        } else {
            Color32::WHITE
        };
        painter.circle_filled(to_screen(f32::from(p[0]), f32::from(p[1])), 3.5, fill);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutter_matches_the_official_list() {
        assert_eq!(shutter_index(167), 0);
        assert_eq!(shutter_index(42), 6);
        assert_eq!(shutter_index(1), SHUTTER_SPEEDS.len() - 1);
        assert_eq!(shutter_index(10_000), 0);
        // Typed 250 became 1/240 in the official app.
        assert_eq!(parse_shutter("250").map(|i| SHUTTER_SPEEDS[i].0), Some(240));
        assert_eq!(
            parse_shutter("1/1000").map(|i| SHUTTER_SPEEDS[i].0),
            Some(1000)
        );
        assert_eq!(parse_shutter("fast"), None);
    }

    #[test]
    fn iso_and_compensation_text() {
        assert_eq!(parse_iso("1234"), Some(1200));
        assert_eq!(parse_iso("50"), None);
        assert_eq!(parse_iso("9999"), Some(ISO_MAX));
        assert_eq!(parse_iso("-5"), None);
        for (value, text) in [
            (0, "-3"),
            (5, "-1.3"),
            (9, "0"),
            (12, "1"),
            (17, "2.7"),
            (18, "3"),
        ] {
            assert_eq!(compensation_text(value), text);
        }
        assert_eq!(parse_compensation("-1.5"), Some(5));
        assert_eq!(parse_compensation("9"), Some(COMPENSATION_MAX));
        assert_eq!(parse_compensation("-9"), Some(0));
        assert_eq!(parse_compensation("NaN"), None);
    }

    #[test]
    fn curve_passes_through_its_points_and_stays_in_range() {
        assert!((curve_value(&[], 128.0) - 128.0).abs() < 0.01);
        let points = [[0x33, 0x5e], [0x7e, 0xcc]];
        for p in points {
            assert!((curve_value(&points, f32::from(p[0])) - f32::from(p[1])).abs() < 0.01);
        }
        let wild = [[10, 255], [20, 0], [CURVE_X_MAX, 255]];
        for x in 0..=255 {
            let y = curve_value(&wild, f32::from(x as u8));
            assert!((0.0..=255.0).contains(&y), "{x}: {y}");
        }
    }
}
