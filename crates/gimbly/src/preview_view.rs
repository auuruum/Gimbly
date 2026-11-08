//! How the preview window lays out the camera picture: fitting it into the
//! video area and drawing composition lines. Rotation and mirroring happen
//! earlier, in the capture thread, so the virtual camera gets them too.

use crate::camera::Transform;
use eframe::egui::{Pos2, Rect, Vec2, pos2};
use gimbly_vcam_shm::pixel::transformed_area;

/// The largest rectangle with the frame's shape centred in `area`.
pub fn fit(frame: Vec2, area: Rect) -> Rect {
    if frame.x <= 0.0 || frame.y <= 0.0 {
        return Rect::from_center_size(area.center(), Vec2::ZERO);
    }
    let scale = (area.width() / frame.x).min(area.height() / frame.y);
    Rect::from_center_size(area.center(), frame * scale)
}

/// The part of an on-screen frame `shown` (of `width`x`height` pixels) that
/// holds the picture, leaving out the bars a quarter turn adds.
pub fn picture_rect(shown: Rect, width: usize, height: usize, transform: Transform) -> Rect {
    if width == 0 || height == 0 {
        return shown;
    }
    let (x, y, w, h) = transformed_area(width, height, transform);
    let sx = shown.width() / width as f32;
    let sy = shown.height() / height as f32;
    Rect::from_min_size(
        shown.min + Vec2::new(x as f32 * sx, y as f32 * sy),
        Vec2::new(w as f32 * sx, h as f32 * sy),
    )
}

/// Rule-of-thirds lines across `rect`: two vertical, then two horizontal.
pub fn composition_lines(rect: Rect) -> [[Pos2; 2]; 4] {
    let x = |t: f32| rect.left() + rect.width() * t;
    let y = |t: f32| rect.top() + rect.height() * t;
    [
        [
            pos2(x(1.0 / 3.0), rect.top()),
            pos2(x(1.0 / 3.0), rect.bottom()),
        ],
        [
            pos2(x(2.0 / 3.0), rect.top()),
            pos2(x(2.0 / 3.0), rect.bottom()),
        ],
        [
            pos2(rect.left(), y(1.0 / 3.0)),
            pos2(rect.right(), y(1.0 / 3.0)),
        ],
        [
            pos2(rect.left(), y(2.0 / 3.0)),
            pos2(rect.right(), y(2.0 / 3.0)),
        ],
    ]
}

/// Aspect badge for a picture of `width`x`height`, e.g. "16:9".
pub fn aspect_label(width: u32, height: u32) -> &'static str {
    if height > width { "9:16" } else { "16:9" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    #[test]
    fn fit_letterboxes_into_the_area() {
        let area = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0));
        let shown = fit(vec2(1920.0, 1080.0), area);
        assert!((shown.width() - 1600.0).abs() < 0.01);
        assert!((shown.height() - 900.0).abs() < 0.01);
        assert_eq!(shown.center(), area.center());
    }

    #[test]
    fn picture_rect_skips_the_bars_of_a_quarter_turn() {
        let shown = Rect::from_min_size(Pos2::ZERO, vec2(960.0, 540.0));
        assert_eq!(picture_rect(shown, 1920, 1080, Transform::default()), shown);
        let turned = picture_rect(shown, 1920, 1080, Transform::new(90, false));
        assert!((turned.height() - 540.0).abs() < 0.01);
        assert!((turned.width() - 303.0).abs() < 1.0);
        assert!((turned.center().x - 480.0).abs() < 1.0);
    }

    #[test]
    fn composition_lines_split_into_thirds() {
        let rect = Rect::from_min_size(pos2(10.0, 0.0), vec2(300.0, 90.0));
        let lines = composition_lines(rect);
        assert_eq!(lines[0][0].x, 110.0);
        assert_eq!(lines[1][0].x, 210.0);
        assert_eq!(lines[2][0].y, 30.0);
        assert_eq!(lines[3][1], pos2(310.0, 60.0));
    }

    #[test]
    fn aspect_badge_follows_orientation() {
        assert_eq!(aspect_label(1920, 1080), "16:9");
        assert_eq!(aspect_label(608, 1080), "9:16");
    }
}
