//! NV12 to RGB conversion for the on-screen preview.

use eframe::egui::Color32;
use gimbly_vcam_shm::Nv12View;

/// Frames wider than this are box-filtered to half size for the preview; the
/// preview window rarely shows more pixels and it quarters the conversion
/// cost.
pub const MAX_PREVIEW_WIDTH: usize = 1280;

pub struct PreviewFrame {
    pub size: [usize; 2],
    pub pixels: Vec<Color32>,
}

/// Quantisation range of the luma/chroma samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvRange {
    /// Y in 16..=235, UV in 16..=240 (video range).
    Limited,
    /// Y and UV in 0..=255 (JPEG range).
    #[cfg(any(not(target_os = "linux"), test))]
    Full,
    #[cfg(target_os = "linux")]
    Limited601,
    /// JPEG frames: full range with BT.601 colours.
    Full601,
}

/// BT.709 coefficients in 8.8 fixed point: (luma gain, V->R, U->G, V->G, U->B).
fn coefficients(range: YuvRange) -> (i32, i32, i32, i32, i32, i32) {
    match range {
        YuvRange::Limited => (16, 298, 459, 55, 136, 541),
        #[cfg(any(not(target_os = "linux"), test))]
        YuvRange::Full => (0, 256, 403, 48, 120, 475),
        #[cfg(target_os = "linux")]
        YuvRange::Limited601 => (16, 298, 409, 100, 208, 516),
        YuvRange::Full601 => (0, 256, 359, 88, 183, 454),
    }
}

#[inline]
fn rgb(y: i32, u: i32, v: i32, k: (i32, i32, i32, i32, i32, i32)) -> Color32 {
    let (y_off, gain, vr, ug, vg, ub) = k;
    let c = gain * (y - y_off) + 128;
    let d = u - 128;
    let e = v - 128;
    let r = (c + vr * e) >> 8;
    let g = (c - ug * d - vg * e) >> 8;
    let b = (c + ub * d) >> 8;
    Color32::from_rgb(
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    )
}

/// Converts a frame for the on-screen preview, halving wide frames.
pub fn convert(view: &Nv12View<'_>, range: YuvRange) -> PreviewFrame {
    convert_with(view, range, view.width > MAX_PREVIEW_WIDTH)
}

/// Converts a frame at its full size (used for snapshots).
pub fn convert_full(view: &Nv12View<'_>, range: YuvRange) -> PreviewFrame {
    convert_with(view, range, false)
}

fn convert_with(view: &Nv12View<'_>, range: YuvRange, halve: bool) -> PreviewFrame {
    let k = coefficients(range);
    if halve {
        let (w, h) = (view.width / 2, view.height / 2);
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            let r0 = &view.y[2 * y * view.y_stride..];
            let r1 = &view.y[(2 * y + 1) * view.y_stride..];
            let uv = &view.uv[y * view.uv_stride..];
            for x in 0..w {
                let luma = (r0[2 * x] as i32
                    + r0[2 * x + 1] as i32
                    + r1[2 * x] as i32
                    + r1[2 * x + 1] as i32
                    + 2)
                    >> 2;
                pixels.push(rgb(luma, uv[2 * x] as i32, uv[2 * x + 1] as i32, k));
            }
        }
        return PreviewFrame {
            size: [w, h],
            pixels,
        };
    }
    let (w, h) = (view.width, view.height);
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &view.y[y * view.y_stride..y * view.y_stride + w];
        let uv = &view.uv[(y / 2) * view.uv_stride..];
        for (x, &luma) in row.iter().enumerate() {
            let c = x & !1;
            pixels.push(rgb(luma as i32, uv[c] as i32, uv[c + 1] as i32, k));
        }
    }
    PreviewFrame {
        size: [w, h],
        pixels,
    }
}

/// A packed NV12 frame of `width`x`height` (both even) with `image`, an RGBA
/// picture `image_w`x`image_h`, fitted into `share` of the frame and centred
/// on black. It stands in for the picture a sleeping camera streams.
pub fn sleep_frame(
    width: usize,
    height: usize,
    image: &[u8],
    (image_w, image_h): (usize, usize),
    share: f32,
    range: YuvRange,
) -> Vec<u8> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let valid = image_w > 0 && image_h > 0 && image.len() >= image_w * image_h * 4;
    let fit = (width as f32 * share / image_w.max(1) as f32)
        .min(height as f32 * share / image_h.max(1) as f32);
    let (size_w, size_h) = (
        ((image_w as f32 * fit) as usize).clamp(1, width.max(1)),
        ((image_h as f32 * fit) as usize).clamp(1, height.max(1)),
    );
    let (left, top) = ((width - size_w) / 2, (height - size_h) / 2);
    let scale = 1.0 / fit;
    let (last_x, last_y) = (
        image_w.saturating_sub(1) as f32,
        image_h.saturating_sub(1) as f32,
    );
    // Bilinear sample of the image, premultiplied, which is its colour over black.
    let pixel = |x: usize, y: usize| -> [f32; 3] {
        if !valid || !(left..left + size_w).contains(&x) || !(top..top + size_h).contains(&y) {
            return [0.0; 3];
        }
        let sx = (((x - left) as f32 + 0.5) * scale - 0.5).clamp(0.0, last_x);
        let sy = (((y - top) as f32 + 0.5) * scale - 0.5).clamp(0.0, last_y);
        let (x0, y0) = (sx as usize, sy as usize);
        let (x1, y1) = ((x0 + 1).min(image_w - 1), (y0 + 1).min(image_h - 1));
        let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
        let mut out = [0.0; 3];
        for (ix, iy, weight) in [
            (x0, y0, (1.0 - fx) * (1.0 - fy)),
            (x1, y0, fx * (1.0 - fy)),
            (x0, y1, (1.0 - fx) * fy),
            (x1, y1, fx * fy),
        ] {
            let p = &image[(iy * image_w + ix) * 4..][..4];
            let alpha = p[3] as f32 / 255.0;
            for (o, &c) in out.iter_mut().zip(&p[..3]) {
                *o += c as f32 * alpha * weight;
            }
        }
        out
    };
    let (y_off, y_gain, c_gain) = match range {
        YuvRange::Limited => (16.0, 219.0 / 255.0, 224.0 / 255.0),
        #[cfg(any(not(target_os = "linux"), test))]
        YuvRange::Full => (0.0, 1.0, 1.0),
        #[cfg(target_os = "linux")]
        YuvRange::Limited601 => (16.0, 219.0 / 255.0, 224.0 / 255.0),
        YuvRange::Full601 => (0.0, 1.0, 1.0),
    };
    let (kr, kb) = match range {
        #[cfg(target_os = "linux")]
        YuvRange::Limited601 => (0.299, 0.114),
        YuvRange::Full601 => (0.299, 0.114),
        _ => (0.2126, 0.0722),
    };
    let luma = |[r, g, b]: [f32; 3]| kr * r + (1.0 - kr - kb) * g + kb * b;

    let mut frame = vec![0u8; width * height * 3 / 2];
    let (luma_plane, chroma_plane) = frame.split_at_mut(width * height);
    for by in (0..height).step_by(2) {
        for bx in (0..width).step_by(2) {
            let mut sum = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let rgb = pixel(bx + dx, by + dy);
                luma_plane[(by + dy) * width + bx + dx] =
                    (y_off + luma(rgb) * y_gain).round() as u8;
                for (s, c) in sum.iter_mut().zip(rgb) {
                    *s += c / 4.0;
                }
            }
            let y = luma(sum);
            let u = (sum[2] - y) / (2.0 * (1.0 - kb));
            let v = (sum[0] - y) / (2.0 * (1.0 - kr));
            let at = by / 2 * width + bx;
            chroma_plane[at] = (128.0 + u * c_gain).round().clamp(0.0, 255.0) as u8;
            chroma_plane[at + 1] = (128.0 + v * c_gain).round().clamp(0.0, 255.0) as u8;
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_frame_centres_the_icon_on_black() {
        // A 4x4 opaque white icon on a 16x16 frame lands on pixels 6..10.
        let icon = vec![255u8; 4 * 4 * 4];
        let buf = sleep_frame(16, 16, &icon, (4, 4), 0.25, YuvRange::Limited);
        assert_eq!(buf.len(), 16 * 16 * 3 / 2);
        let out = convert_full(&Nv12View::packed(&buf, 16, 16).unwrap(), YuvRange::Limited);
        assert_eq!(out.pixels[0], Color32::from_rgb(0, 0, 0));
        assert_eq!(out.pixels[8 * 16 + 8], Color32::from_rgb(255, 255, 255));
        assert_eq!(out.pixels[8 * 16 + 4], Color32::from_rgb(0, 0, 0));
    }

    #[test]
    fn sleep_frame_fits_a_wide_picture_and_survives_bad_input() {
        // An 8x2 picture fills the width of a 16x16 frame, centred.
        let picture = vec![255u8; 8 * 2 * 4];
        let buf = sleep_frame(16, 16, &picture, (8, 2), 1.0, YuvRange::Limited);
        let out = convert_full(&Nv12View::packed(&buf, 16, 16).unwrap(), YuvRange::Limited);
        assert_eq!(out.pixels[8 * 16], Color32::from_rgb(255, 255, 255));
        assert_eq!(out.pixels[0], Color32::from_rgb(0, 0, 0));
        // Too few bytes or a zero size draws black instead of panicking.
        for size in [(8, 2), (0, 0)] {
            let buf = sleep_frame(16, 16, &picture[..10], size, 1.0, YuvRange::Limited);
            assert!(buf[..256].iter().all(|&y| y == 16));
        }
    }

    fn frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut buf = vec![y; width * height * 3 / 2];
        for pair in buf[width * height..].as_chunks_mut::<2>().0 {
            pair[0] = u;
            pair[1] = v;
        }
        buf
    }

    #[test]
    fn neutral_grey_stays_grey() {
        let buf = frame(4, 2, 126, 128, 128);
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let out = convert(&view, YuvRange::Limited);
        assert_eq!(out.size, [4, 2]);
        assert!(
            out.pixels
                .iter()
                .all(|p| *p == Color32::from_rgb(128, 128, 128))
        );

        let buf = frame(4, 2, 128, 128, 128);
        let view = Nv12View::packed(&buf, 4, 2).unwrap();
        let out = convert(&view, YuvRange::Full);
        assert!(
            out.pixels
                .iter()
                .all(|p| *p == Color32::from_rgb(128, 128, 128))
        );
    }

    #[test]
    fn limited_range_black_and_white_clip_correctly() {
        let black = frame(2, 2, 16, 128, 128);
        let white = frame(2, 2, 235, 128, 128);
        let b = convert(&Nv12View::packed(&black, 2, 2).unwrap(), YuvRange::Limited);
        let w = convert(&Nv12View::packed(&white, 2, 2).unwrap(), YuvRange::Limited);
        assert_eq!(b.pixels[0], Color32::from_rgb(0, 0, 0));
        assert_eq!(w.pixels[0], Color32::from_rgb(255, 255, 255));
    }

    #[test]
    fn strong_v_is_red() {
        let buf = frame(2, 2, 81, 90, 240);
        let out = convert(&Nv12View::packed(&buf, 2, 2).unwrap(), YuvRange::Limited);
        let p = out.pixels[0];
        assert!(p.r() > 200 && p.g() < 60 && p.b() < 60, "{p:?}");
    }

    #[test]
    fn wide_frames_are_halved() {
        let width = MAX_PREVIEW_WIDTH + 2;
        let buf = frame(width, 4, 126, 128, 128);
        let out = convert(
            &Nv12View::packed(&buf, width, 4).unwrap(),
            YuvRange::Limited,
        );
        assert_eq!(out.size, [width / 2, 2]);
        assert_eq!(out.pixels.len(), width / 2 * 2);
    }

    #[test]
    fn full_conversion_keeps_wide_frames() {
        let width = MAX_PREVIEW_WIDTH + 2;
        let buf = frame(width, 4, 126, 128, 128);
        let out = convert_full(
            &Nv12View::packed(&buf, width, 4).unwrap(),
            YuvRange::Limited,
        );
        assert_eq!(out.size, [width, 4]);
        assert_eq!(out.pixels.len(), width * 4);
    }
}
