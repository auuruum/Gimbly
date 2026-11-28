//! The White Balance section of the Image tab.

use super::view_exposure::{slider_row, step_u8};
use super::{GimblyApp, switch_row};
use crate::camera::{
    Feature, GimbalCommand, WB_SHIFT_MAX, WB_SHIFT_ZERO, WB_TEMPERATURE, WB_TEMPERATURE_STEP,
    WhiteBalance,
};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::time::{Duration, Instant};

/// How long a value the user picked is shown before the camera's own
/// reading takes over again (when a write failed).
const TARGET_HOLD: Duration = Duration::from_secs(2);
const PAD: f32 = 130.0;

impl GimblyApp {
    /// The White Balance section: Auto WB with its shift pad, or the manual
    /// temperature or red and blue gains.
    pub(super) fn white_balance(&mut self, ui: &mut Ui) {
        let ready = self.ready(Feature::WhiteBalance) && self.status.white_balance.is_some();
        let reset;
        (self.white_balance_open, reset) = section_header(
            ui,
            "White Balance",
            self.white_balance_open,
            ready.then_some("Reset to Auto WB with no shift"),
        );
        let shown = self.shown_white_balance();
        // Like the official app: reset turns Auto WB on and clears the
        // shift; the manual values and method stay.
        if reset && let Some(wb) = shown {
            self.set_white_balance(WhiteBalance {
                auto: true,
                shift: [WB_SHIFT_ZERO; 2],
                ..wb
            });
        }
        if !self.white_balance_open {
            return;
        }
        let tip = self.gimbal_tip(Feature::WhiteBalance);
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 4,
            bottom: 18,
        };
        let mut next = shown;
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_enabled_ui(ready, |ui| {
                let auto = next.is_none_or(|wb| wb.auto);
                let (switch, _) = switch_row(
                    ui,
                    "Auto WB",
                    "Sets the colors by itself. Turn it off to set the color temperature or the red and blue gains by hand",
                    None,
                    auto,
                );
                let switched = switch.on_disabled_hover_text(&tip).clicked();
                let Some(wb) = &mut next else {
                    return;
                };
                if switched {
                    wb.auto = !auto;
                } else if auto {
                    shift_rows(ui, wb);
                } else {
                    manual_rows(ui, wb);
                }
            });
        });
        if next != shown
            && let Some(next) = next
        {
            self.set_white_balance(next);
        }
    }

    fn set_white_balance(&mut self, wb: WhiteBalance) {
        self.white_balance_target = Some((wb, Instant::now()));
        self.gimbal(GimbalCommand::SetWhiteBalance(wb));
    }

    /// The white balance to show: the user's latest pick until the camera reports it.
    fn shown_white_balance(&mut self) -> Option<WhiteBalance> {
        let camera = self.status.white_balance;
        match self.white_balance_target {
            Some((target, at)) if camera != Some(target) && at.elapsed() < TARGET_HOLD => {
                Some(target)
            }
            _ => {
                self.white_balance_target = None;
                camera
            }
        }
    }
}

/// The White Balance Shift pad with its B-A and G-M fields.
fn shift_rows(ui: &mut Ui, wb: &mut WhiteBalance) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(PAD, PAD), Sense::hover());
        if let Some(shift) = shift_pad(ui, rect, wb.shift) {
            wb.shift = shift;
        }
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("White Balance Shift").size(14.0));
                help(
                    ui,
                    "Tunes Auto WB: B-A from blue to amber, G-M from green to magenta. Click or drag in the square, or use the fields",
                );
            });
            for (i, name) in [(0, "B-A:"), (1, "G-M:")] {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(name).size(14.0));
                let (field, _) = ui.allocate_exact_size(vec2(74.0, 25.0), Sense::hover());
                let value = wb.shift[i];
                let input = place(ui, field, Layout::default(), ("wb-shift", i), |ui| {
                    number_box(ui, field, &shift_text(value))
                });
                let picked = match input {
                    Some(NumberInput::Step(step)) => Some(step_u8(value, step, WB_SHIFT_MAX)),
                    Some(NumberInput::Typed(text)) => parse_shift(&text),
                    None => None,
                };
                if let Some(picked) = picked {
                    wb.shift[i] = picked;
                }
            }
        });
    });
}

/// The square that picks the shift: B-A left to right, G-M bottom to top.
/// Returns the new shift while it is clicked or dragged.
fn shift_pad(ui: &mut Ui, rect: Rect, shift: [u8; 2]) -> Option<[u8; 2]> {
    let enabled = ui.is_enabled();
    let response = ui.interact(
        rect,
        ui.id().with("wb-pad"),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let mut mesh = egui::Mesh::default();
    const CELLS: u32 = 8;
    for row in 0..=CELLS {
        for column in 0..=CELLS {
            let (u, v) = (column as f32 / CELLS as f32, row as f32 / CELLS as f32);
            let mut color = pad_color(u * 2.0 - 1.0, 1.0 - v * 2.0);
            if !enabled {
                color = color.gamma_multiply(0.4);
            }
            mesh.colored_vertex(rect.min + vec2(u * rect.width(), v * rect.height()), color);
            if row < CELLS && column < CELLS {
                let i = row * (CELLS + 1) + column;
                mesh.add_triangle(i, i + 1, i + CELLS + 1);
                mesh.add_triangle(i + 1, i + CELLS + 2, i + CELLS + 1);
            }
        }
    }
    let painter = ui.painter_at(rect.expand(6.0));
    painter.add(mesh);
    let at = |value: u8| f32::from(value) / f32::from(WB_SHIFT_MAX);
    let marker = pos2(
        rect.left() + at(shift[0]) * rect.width(),
        rect.bottom() - at(shift[1]) * rect.height(),
    );
    painter.circle_stroke(marker, 5.0, Stroke::new(1.5, Color32::from_gray(30)));
    // A quick click can press and release within one frame, so check `clicked` too.
    let pointer = response
        .interact_pointer_pos()
        .filter(|_| response.is_pointer_button_down_on() || response.clicked())?;
    let pick = |t: f32| (t.clamp(0.0, 1.0) * f32::from(WB_SHIFT_MAX)).round() as u8;
    let next = [
        pick((pointer.x - rect.left()) / rect.width()),
        pick((rect.bottom() - pointer.y) / rect.height()),
    ];
    (next != shift).then_some(next)
}

/// The pad color at B-A `a` and G-M `m`, each -1 to 1: grey in the middle,
/// blue to amber across and green to magenta up.
fn pad_color(a: f32, m: f32) -> Color32 {
    let channel = |v: f32| ((0.62 + 0.5 * v).clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgb(
        channel(0.75 * a + 0.55 * m),
        channel(0.3 * a - 0.8 * m),
        channel(-0.85 * a + 0.45 * m),
    )
}

/// The manual rows: the temperature or the red and blue gains, and the method.
fn manual_rows(ui: &mut Ui, wb: &mut WhiteBalance) {
    if !wb.gains {
        temperature_row(ui, wb);
    }
    ui.horizontal(|ui| {
        ui.set_height(30.0);
        ui.label(egui::RichText::new("Manual Adjustment").size(15.0));
        help(
            ui,
            "Temp sets the color temperature in kelvin, R/B sets the red and blue gains",
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(picked) = segmented(ui, &["Temp", "R/B"], Some(wb.gains.into())) {
                wb.gains = picked == 1;
            }
        });
    });
    if wb.gains {
        for (name, gain) in [("R Gain", &mut wb.red), ("B Gain", &mut wb.blue)] {
            let value = *gain;
            let t = f32::from(value) / 255.0;
            let (dragged, input) = slider_row(ui, name, t, &value.to_string());
            let picked = dragged
                .map(|t| (t * 255.0).round() as u8)
                .or_else(|| match input? {
                    NumberInput::Step(step) => Some(step_u8(value, step, u8::MAX)),
                    NumberInput::Typed(text) => text.trim().parse().ok(),
                });
            if let Some(picked) = picked {
                *gain = picked;
            }
        }
    }
}

fn temperature_row(ui: &mut Ui, wb: &mut WhiteBalance) {
    let (min, max) = (*WB_TEMPERATURE.start(), *WB_TEMPERATURE.end());
    let kelvin = wb.temperature;
    let t = f32::from(kelvin.saturating_sub(min)) / f32::from(max - min);
    let (dragged, input) = slider_row(ui, "Temperature", t, &format!("{kelvin} K"));
    let picked = dragged
        .map(|t| round_temperature(f32::from(min) + t * f32::from(max - min)))
        .or_else(|| match input? {
            NumberInput::Step(step) if step > 0.0 => {
                Some(kelvin.saturating_add(WB_TEMPERATURE_STEP).min(max))
            }
            NumberInput::Step(_) => Some(kelvin.saturating_sub(WB_TEMPERATURE_STEP).max(min)),
            NumberInput::Typed(text) => parse_temperature(&text),
        });
    if let Some(picked) = picked {
        wb.temperature = picked;
    }
}

fn round_temperature(kelvin: f32) -> u16 {
    let step = f32::from(WB_TEMPERATURE_STEP);
    ((kelvin / step).round() * step) as u16
}

/// Typed kelvin, `4321` or `4321 K`, to the nearest step; out of range is
/// refused, as in the official app.
fn parse_temperature(text: &str) -> Option<u16> {
    let text = text.trim();
    let text = text.strip_suffix(['K', 'k']).unwrap_or(text).trim();
    let kelvin: f32 = text.parse().ok()?;
    let kelvin = round_temperature(kelvin.clamp(0.0, 20_000.0));
    WB_TEMPERATURE.contains(&kelvin).then_some(kelvin)
}

/// The shift as shown: `0.00`, `-1.75`.
fn shift_text(value: u8) -> String {
    format!("{:.2}", (f32::from(value) - f32::from(WB_SHIFT_ZERO)) / 4.0)
}

/// Typed shift, -7 to 7, to the nearest quarter (the official app only took the arrows).
fn parse_shift(text: &str) -> Option<u8> {
    let value: f32 = text.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let quarters = (value * 4.0).round() + f32::from(WB_SHIFT_ZERO);
    Some(quarters.clamp(0.0, f32::from(WB_SHIFT_MAX)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_text_and_typed_values() {
        assert_eq!(shift_text(WB_SHIFT_ZERO), "0.00");
        assert_eq!(shift_text(0), "-7.00");
        assert_eq!(shift_text(WB_SHIFT_MAX), "7.00");
        assert_eq!(shift_text(35), "1.75");
        assert_eq!(parse_shift("1.75"), Some(35));
        assert_eq!(parse_shift(" -0.3 "), Some(27));
        assert_eq!(parse_shift("9"), Some(WB_SHIFT_MAX));
        assert_eq!(parse_shift("-9"), Some(0));
        assert_eq!(parse_shift("abc"), None);
        assert_eq!(parse_shift("NaN"), None);
    }

    #[test]
    fn typed_temperature_rounds_and_refuses_out_of_range() {
        assert_eq!(parse_temperature("4321"), Some(4300));
        assert_eq!(parse_temperature("4389 K"), Some(4400));
        assert_eq!(parse_temperature("5700k"), Some(5700));
        assert_eq!(parse_temperature("2000"), Some(2000));
        assert_eq!(parse_temperature("10000"), Some(10000));
        assert_eq!(parse_temperature("1500"), None);
        assert_eq!(parse_temperature("12000"), None);
        assert_eq!(parse_temperature("1e30"), None);
        assert_eq!(parse_temperature("warm"), None);
    }

    #[test]
    fn pad_corners_match_the_axes() {
        // Top left blue-violet, top right red, bottom left cyan-green, bottom right yellow.
        let tl = pad_color(-1.0, 1.0);
        assert!(tl.b() > tl.r() && tl.b() > tl.g());
        let tr = pad_color(1.0, 1.0);
        assert!(tr.r() > tr.g() && tr.r() > tr.b());
        let bl = pad_color(-1.0, -1.0);
        assert!(bl.g() > bl.r());
        let br = pad_color(1.0, -1.0);
        assert!(br.r() > br.b() && br.g() > br.b());
    }
}
