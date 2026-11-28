//! The Image section of the Image tab: contrast, saturation, sharpness and hue.

use super::GimblyApp;
use super::view_exposure::{slider_row, step_u8};
use crate::camera::{Feature, GimbalCommand, IMAGE_DEFAULT, IMAGE_MAX, ImageAdjust};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{self, Ui, vec2};
use std::time::{Duration, Instant};

/// How long a value the user picked is shown before the camera's own
/// reading takes over again (when a write failed).
const TARGET_HOLD: Duration = Duration::from_secs(2);
const NAMES: [&str; 4] = ["Contrast", "Saturation", "Sharpness", "Hue"];

impl GimblyApp {
    pub(super) fn image_adjust(&mut self, ui: &mut Ui) {
        let ready = self.ready(Feature::Image) && self.status.image.is_some();
        let reset;
        (self.image_open, reset) = section_header(
            ui,
            "Image",
            self.image_open,
            ready.then_some("Reset all four to 50"),
        );
        let shown = self.shown_image();
        if reset {
            self.set_image(IMAGE_DEFAULT);
        }
        if !self.image_open {
            return;
        }
        let tip = self.gimbal_tip(Feature::Image);
        let margin = egui::Margin {
            left: PADDING as i8,
            right: PADDING as i8,
            top: 4,
            bottom: 18,
        };
        let mut values = shown.unwrap_or(IMAGE_DEFAULT).values();
        band(ui, CARD, margin, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 16.0);
            ui.add_enabled_ui(ready, |ui| {
                for (value, name) in values.iter_mut().zip(NAMES) {
                    let (dragged, input) = slider_row(
                        ui,
                        name,
                        f32::from(*value) / f32::from(IMAGE_MAX),
                        &value.to_string(),
                    );
                    let picked = dragged
                        .map(|t| (t * f32::from(IMAGE_MAX)).round() as u8)
                        .or_else(|| match input? {
                            NumberInput::Step(step) => Some(step_u8(*value, step, IMAGE_MAX)),
                            NumberInput::Typed(text) => parse_value(&text),
                        });
                    if let Some(picked) = picked {
                        *value = picked;
                    }
                }
            })
            .response
            .on_disabled_hover_text(&tip);
        });
        let next = ImageAdjust::from_values(values);
        if shown.is_some_and(|shown| shown != next) {
            self.set_image(next);
        }
    }

    fn set_image(&mut self, image: ImageAdjust) {
        self.image_target = Some((image, Instant::now()));
        self.gimbal(GimbalCommand::SetImage(image));
    }

    /// The values to show: the user's latest pick until the camera reports it.
    fn shown_image(&mut self) -> Option<ImageAdjust> {
        let camera = self.status.image;
        match self.image_target {
            Some((target, at)) if camera != Some(target) && at.elapsed() < TARGET_HOLD => {
                Some(target)
            }
            _ => {
                self.image_target = None;
                camera
            }
        }
    }
}

/// A typed value; past 100 is refused, as in the official app.
fn parse_value(text: &str) -> Option<u8> {
    text.trim().parse().ok().filter(|&v| v <= IMAGE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_stay_in_range() {
        assert_eq!(parse_value("37"), Some(37));
        assert_eq!(parse_value(" 0 "), Some(0));
        assert_eq!(parse_value("100"), Some(100));
        assert_eq!(parse_value("150"), None);
        assert_eq!(parse_value("-1"), None);
        assert_eq!(parse_value("abc"), None);
    }

    #[test]
    fn values_keep_the_section_order() {
        let image = ImageAdjust::from_values([1, 2, 3, 4]);
        assert_eq!(
            (image.contrast, image.saturation, image.sharpness, image.hue),
            (1, 2, 3, 4)
        );
        assert_eq!(image.values(), [1, 2, 3, 4]);
    }
}
