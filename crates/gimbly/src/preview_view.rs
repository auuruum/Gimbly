//! How the preview window lays out the camera picture: fitting it into the
//! video area, drawing composition lines and mapping points between the
//! shown picture and the camera's own picture. Rotation and mirroring happen
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

/// Where a point of the shown picture (fractions of its width and height)
/// lies in the camera's picture, which the camera reports and expects
/// targets in. `crop` is the shown part of the camera's width: 1.0, or less
/// for the portrait crop. Mirrors the sampling in `transform_into`.
pub fn to_camera(point: Pos2, t: Transform, crop: f32) -> Pos2 {
    let (dx, dy) = (point.x, point.y);
    let m = t.mirror;
    let flip = |flip: bool, v: f32| if flip { 1.0 - v } else { v };
    let (sx, sy) = match t.degrees() / 90 {
        0 => (flip(m, dx), dy),
        1 => (dy, flip(!m, dx)),
        2 => (flip(!m, dx), 1.0 - dy),
        _ => (1.0 - dy, flip(m, dx)),
    };
    pos2(0.5 + (sx - 0.5) * crop, sy)
}

/// The inverse of [`to_camera`].
pub fn from_camera(point: Pos2, t: Transform, crop: f32) -> Pos2 {
    let sx = if crop > 0.0 {
        0.5 + (point.x - 0.5) / crop
    } else {
        0.5
    };
    let sy = point.y;
    let m = t.mirror;
    let flip = |flip: bool, v: f32| if flip { 1.0 - v } else { v };
    let (dx, dy) = match t.degrees() / 90 {
        0 => (flip(m, sx), sy),
        1 => (flip(!m, sy), sx),
        2 => (flip(!m, sx), 1.0 - sy),
        _ => (flip(m, sy), 1.0 - sx),
    };
    pos2(dx, dy)
}

/// The shown part of the camera's width for an output of `width`x`height`;
/// below 1.0 for the portrait crop.
pub fn crop_fraction(width: u32, height: u32) -> f32 {
    // shortcut: assumes the camera's picture is 16:9, as every mode Gimbly
    // offers is; take the capture size from the session if that changes.
    if width == 0 || height == 0 || width >= height {
        return 1.0;
    }
    (width as f32 / height as f32 / (16.0 / 9.0)).min(1.0)
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
    fn camera_points_round_trip_through_every_transform() {
        let points = [
            pos2(0.1, 0.2),
            pos2(0.9, 0.3),
            pos2(0.5, 0.5),
            pos2(0.0, 1.0),
        ];
        for degrees in [0, 90, 180, 270] {
            for mirror in [false, true] {
                let t = Transform::new(degrees, mirror);
                for crop in [1.0, crop_fraction(608, 1080)] {
                    for p in points {
                        let back = from_camera(to_camera(p, t, crop), t, crop);
                        assert!(
                            (back - p).length() < 1e-5,
                            "{degrees} {mirror} {crop} {p:?}"
                        );
                    }
                }
            }
        }
        let near = |a: Pos2, b: Pos2| (a - b).length() < 1e-5;
        let identity = Transform::default();
        assert!(near(
            to_camera(pos2(0.2, 0.7), identity, 1.0),
            pos2(0.2, 0.7)
        ));
        let mirrored = Transform::new(0, true);
        assert!(near(
            to_camera(pos2(0.2, 0.7), mirrored, 1.0),
            pos2(0.8, 0.7)
        ));
        // Turned upside down: the shown top left is the camera's bottom right.
        let turned = Transform::new(180, false);
        assert!(near(to_camera(pos2(0.0, 0.0), turned, 1.0), pos2(1.0, 1.0)));
        // The portrait crop shows the middle of the camera's width.
        let crop = crop_fraction(608, 1080);
        assert!((crop - 0.3167).abs() < 1e-3);
        assert!(near(
            to_camera(pos2(0.5, 0.5), identity, crop),
            pos2(0.5, 0.5)
        ));
        assert!((to_camera(pos2(0.0, 0.5), identity, crop).x - 0.3417).abs() < 1e-3);
        assert_eq!(crop_fraction(1920, 1080), 1.0);
        assert_eq!(crop_fraction(0, 0), 1.0);
    }

    #[test]
    fn camera_points_match_the_rotated_pixels() {
        use gimbly_vcam_shm::pixel::{Nv12View, transform_into};
        // A 4x4 picture whose luma is the pixel index; read back which source
        // pixel lands at each shown corner (no bars for a square picture).
        let (w, h) = (4usize, 4usize);
        let y: Vec<u8> = (0..(w * h) as u8).collect();
        let uv = vec![128u8; w * h / 2];
        let src = Nv12View {
            width: w,
            height: h,
            y: &y,
            y_stride: w,
            uv: &uv,
            uv_stride: w,
        };
        for degrees in [0, 90, 180, 270] {
            for mirror in [false, true] {
                let t = Transform::new(degrees, mirror);
                let mut out = Vec::new();
                let view = transform_into(&src, t, &mut out);
                for (col, row) in [(0usize, 0usize), (3, 0), (0, 3), (2, 1)] {
                    let source = view.y[row * view.y_stride + col] as usize;
                    let shown = pos2((col as f32 + 0.5) / 4.0, (row as f32 + 0.5) / 4.0);
                    let camera = to_camera(shown, t, 1.0);
                    let expected = pos2(
                        ((source % w) as f32 + 0.5) / 4.0,
                        ((source / w) as f32 + 0.5) / 4.0,
                    );
                    assert!(
                        (camera - expected).length() < 1e-5,
                        "{degrees} {mirror} {col},{row}"
                    );
                }
            }
        }
    }

    #[test]
    fn aspect_badge_follows_orientation() {
        assert_eq!(aspect_label(1920, 1080), "16:9");
        assert_eq!(aspect_label(608, 1080), "9:16");
    }
}
