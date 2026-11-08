//! Recognizes the Tiny 3 Lite's standby picture in the raw video stream.

use gimbly_vcam_shm::Nv12View;

/// The camera can keep sending its standby picture after reporting that it
/// is awake. Decide from the frame itself so the handoff to live video does
/// not depend on how long waking takes.
pub(super) fn use_placeholder(sleeping: bool, tiny_3_lite: bool, view: &Nv12View<'_>) -> bool {
    sleeping || (tiny_3_lite && is_standby_picture(view))
}

fn is_standby_picture(view: &Nv12View<'_>) -> bool {
    let sample = |x: usize, y: usize| -> Option<[u8; 3]> {
        let luma = *view.y.get(y * view.y_stride + x)?;
        let uv_at = (y / 2) * view.uv_stride + (x & !1);
        Some([luma, *view.uv.get(uv_at)?, *view.uv.get(uv_at + 1)?])
    };
    let at_percent = |x, y| sample(view.width * x / 100, view.height * y / 100);
    let Some([bg, u, v]) = at_percent(10, 10) else {
        return false;
    };
    if bg > 48 || u.abs_diff(128) > 6 || v.abs_diff(128) > 6 {
        return false;
    }

    // The border is a flat, nearly neutral dark field. A dark scene alone
    // does not qualify; look for the red badge and its white ring below.
    if ![
        (50, 10),
        (90, 10),
        (10, 41),
        (25, 41),
        (75, 41),
        (90, 41),
        (10, 90),
        (50, 90),
        (90, 90),
    ]
    .into_iter()
    .all(|(x, y)| {
        at_percent(x, y).is_some_and(|[luma, u, v]| {
            luma.abs_diff(bg) <= 6 && u.abs_diff(128) <= 6 && v.abs_diff(128) <= 6
        })
    }) {
        return false;
    }

    // The badge scales with the shorter side of the image. Check before
    // any app-side crop, rotation or mirror.
    let center = view.width / 2;
    let unit = view.width.min(view.height);
    let offset = unit * 9 / 100;
    if offset == 0 {
        return false;
    }
    (39..=43).any(|row| {
        let y = view.height * row / 100;
        let red = |x| sample(x, y).is_some_and(|[_, _, v]| v >= 175);
        let white = |x| sample(x, y).is_some_and(|[luma, _, _]| luma >= 150);
        red(center - offset)
            && red(center)
            && red(center + offset)
            && (5..=8).any(|d| white(center - unit * d / 100))
            && (5..=8).any(|d| white(center + unit * d / 100))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic NV12; no pixels from the vendor picture are included.
    fn frame(width: usize, height: usize, stride: usize, badge: bool) -> Vec<u8> {
        let (y_size, uv_size) = (stride * height, stride * height / 2);
        let mut pixels = vec![32; y_size];
        pixels.resize(y_size + uv_size, 128);
        if badge {
            for y in height * 31 / 100..height * 52 / 100 {
                for x in width / 2 - height / 9..width / 2 + height / 9 {
                    let white =
                        (height * 5 / 100..=height * 8 / 100).contains(&x.abs_diff(width / 2));
                    pixels[y * stride + x] = if white { 220 } else { 60 };
                    if x.is_multiple_of(2) {
                        pixels[y_size + y / 2 * stride + x + 1] = if white { 128 } else { 220 };
                    }
                }
            }
        }
        pixels
    }

    #[test]
    fn resume_masks_standby_until_the_first_live_frame() {
        for (width, height, stride) in [(160, 90, 160), (320, 180, 336)] {
            let standby = frame(width, height, stride, true);
            let live = frame(width, height, stride, false);
            let view =
                |pixels| Nv12View::with_stride(pixels, width, height, stride, height).unwrap();

            assert!(use_placeholder(true, true, &view(&standby)));
            // Any number of frames can still contain the badge after Resume.
            assert!(use_placeholder(false, true, &view(&standby)));
            // Even a dark live image can take over immediately.
            assert!(!use_placeholder(false, true, &view(&live)));
            // A late standby frame still gets replaced.
            assert!(use_placeholder(false, true, &view(&standby)));
            assert!(!use_placeholder(false, false, &view(&standby)));
        }
    }

    #[test]
    fn red_and_white_in_a_scene_with_detail_do_not_trigger_the_placeholder() {
        let mut pixels = frame(160, 90, 160, true);
        pixels[81 * 160 + 80] = 160;
        let view = Nv12View::packed(&pixels, 160, 90).unwrap();
        assert!(!use_placeholder(false, true, &view));
    }
}
