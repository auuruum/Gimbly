//! Recognizes the camera's own sleep picture so Gimbly never passes it on.
//!
//! While asleep the camera streams the Sleep Background picked on the More
//! tab, and it keeps sending it for a moment after it reports that it is
//! awake. Gimbly shows its own pictures instead, so it remembers what the
//! sleeping camera sent and replaces matching frames for a while after wake.

use gimbly_vcam_shm::Nv12View;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long after wake a frame that looks like the sleep picture is replaced.
const AFTER_WAKE: Duration = Duration::from_secs(10);
/// Distinct sleep frames kept; a looping sleep video has a few hundred.
const KEEP: usize = 600;
const GRID: (usize, usize) = (12, 8);
type Print = [u8; GRID.0 * GRID.1 * 3];

#[derive(Default)]
pub(super) struct SleepFrames {
    prints: VecDeque<Print>,
    asleep: bool,
    woke: Option<Instant>,
}

impl SleepFrames {
    /// Whether this camera frame must be replaced by Gimbly's sleep picture.
    pub(super) fn replace(&mut self, sleeping: bool, view: &Nv12View<'_>, now: Instant) -> bool {
        if sleeping {
            self.asleep = true;
            self.woke = None;
            if let Some(print) = fingerprint(view)
                && !self.prints.iter().any(|known| similar(known, &print))
            {
                if self.prints.len() == KEEP {
                    self.prints.pop_front();
                }
                self.prints.push_back(print);
            }
            return true;
        }
        if std::mem::take(&mut self.asleep) {
            self.woke = Some(now);
        }
        match self.woke {
            Some(woke) if now.saturating_duration_since(woke) < AFTER_WAKE => fingerprint(view)
                .is_some_and(|print| self.prints.iter().any(|known| similar(known, &print))),
            _ => {
                self.woke = None;
                self.prints.clear();
                false
            }
        }
    }
}

/// Luma and chroma on a coarse grid, independent of the frame size.
fn fingerprint(view: &Nv12View<'_>) -> Option<Print> {
    let mut print = [0; GRID.0 * GRID.1 * 3];
    for (i, chunk) in print.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let x = (i % GRID.0 * 2 + 1) * view.width / (GRID.0 * 2);
        let y = (i / GRID.0 * 2 + 1) * view.height / (GRID.1 * 2);
        let uv = (y / 2) * view.uv_stride + (x & !1);
        chunk[0] = *view.y.get(y * view.y_stride + x)?;
        chunk[1] = *view.uv.get(uv)?;
        chunk[2] = *view.uv.get(uv + 1)?;
    }
    Some(print)
}

/// Compression noise only; a live scene differs far more.
fn similar(a: &Print, b: &Print) -> bool {
    let total: u32 = a
        .iter()
        .zip(b)
        .map(|(a, b)| u32::from(a.abs_diff(*b)))
        .sum();
    total <= 3 * a.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: usize, height: usize, seed: u8) -> Vec<u8> {
        (0..width * height * 3 / 2)
            .map(|i| (i % 97) as u8 ^ seed)
            .collect()
    }

    #[test]
    fn sleep_picture_is_replaced_after_wake_but_live_video_is_not() {
        let (w, h) = (64, 36);
        let sleep = frame(w, h, 0);
        let live = frame(w, h, 0x55);
        let view = |pixels| Nv12View::packed(pixels, w, h).unwrap();
        let start = Instant::now();
        let mut frames = SleepFrames::default();

        assert!(frames.replace(true, &view(&sleep), start));
        // Awake, but the camera still sends its sleep picture.
        let woke = start + Duration::from_secs(1);
        assert!(frames.replace(false, &view(&sleep), woke));
        assert!(!frames.replace(false, &view(&live), woke));
        // A late sleep frame is still caught.
        assert!(frames.replace(false, &view(&sleep), woke + Duration::from_secs(2)));
        // Long after wake nothing is replaced, even a frame like the picture.
        assert!(!frames.replace(false, &view(&sleep), woke + AFTER_WAKE));
        assert!(!frames.replace(false, &view(&sleep), woke + AFTER_WAKE * 2));
    }

    #[test]
    fn keeps_a_bounded_set_of_distinct_frames() {
        let (w, h) = (64, 36);
        let mut frames = SleepFrames::default();
        let now = Instant::now();
        for seed in 0..=255u8 {
            for _ in 0..4 {
                frames.replace(
                    true,
                    &Nv12View::packed(&frame(w, h, seed), w, h).unwrap(),
                    now,
                );
            }
        }
        assert!(frames.prints.len() <= KEEP);
        assert!(frames.prints.len() > 1);
    }

    #[test]
    fn tiny_frames_do_not_panic() {
        let pixels = [16u8; 6];
        let view = Nv12View::packed(&pixels, 2, 2).unwrap();
        let mut frames = SleepFrames::default();
        assert!(frames.replace(true, &view, Instant::now()));
        frames.replace(false, &view, Instant::now());
    }
}
