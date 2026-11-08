//! Joystick speed curves: how far a joystick is pulled becomes the pitch and
//! yaw speeds sent to the camera.
//!
//! Positive yaw turns right; negative pitch looks up, so screen y maps to
//! pitch directly.

use eframe::egui::Vec2;

/// Fastest speed either joystick asks for.
pub const SPEED_MAX: f32 = 30.0;
/// Gimbal Speed slider range (1 = slow, 5 = fast).
pub const SPEED_LEVEL_MAX: u8 = 5;
/// How far the panel joystick's knob travels from the centre, in points.
pub const PANEL_TRAVEL: f32 = 59.0;

/// The Coarse/Fine button of the View and Gimbal section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedMode {
    /// Normal speed (the button reads "Coarse").
    Coarse,
    /// A third of the Coarse speed for fine framing.
    Fine,
}

impl SpeedMode {
    fn factor(self) -> f32 {
        match self {
            SpeedMode::Coarse => 1.0,
            SpeedMode::Fine => 1.0 / 3.0,
        }
    }
}

/// The panel joystick's pull, 0 (centre) to 1 (full speed). It is updated
/// only when the pointer moves, so holding still keeps the speed.
// shortcut: linear in the knob travel. The capture of 2026-11-08 confirms the
// full-pull speeds (30, 20, 10) but not the knob-to-speed curve or which level
// gave which; recapture with the knob held at known positions and levels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanelPull {
    ratio: f32,
}

impl PanelPull {
    pub fn ratio(self) -> f32 {
        self.ratio
    }

    /// Takes in a new pointer distance from the centre, in points.
    pub fn moved(&mut self, distance: f32) -> f32 {
        self.ratio = (distance / PANEL_TRAVEL).clamp(0.0, 1.0);
        self.ratio
    }
}

/// Top speed of the panel joystick for a Gimbal Speed level and mode:
/// 30 at level 5, 10 at level 1, a third of that in Fine mode.
pub fn panel_top_speed(mode: SpeedMode, level: u8) -> f32 {
    let level = level.clamp(1, SPEED_LEVEL_MAX) as f32;
    SPEED_MAX * (level + 1.0) / (SPEED_LEVEL_MAX as f32 + 1.0) * mode.factor()
}

/// Panel joystick: (pitch, yaw) for a pull of `ratio` towards `offset`.
pub fn panel_speed(offset: Vec2, ratio: f32, mode: SpeedMode, level: u8) -> (f32, f32) {
    let top = panel_top_speed(mode, level);
    along(offset, ratio * top)
}

/// Pull at which the preview overlay reaches full speed.
const PREVIEW_FULL_PULL: f32 = 40.0;

/// Preview overlay: full speed at 40 points; Shift uses a third of that speed.
pub fn preview_speed(offset: Vec2, fine: bool) -> (f32, f32) {
    let mode = if fine {
        SpeedMode::Fine
    } else {
        SpeedMode::Coarse
    };
    along(
        offset,
        SPEED_MAX * (offset.length() / PREVIEW_FULL_PULL).min(1.0) * mode.factor(),
    )
}

/// Splits `speed` into (pitch, yaw) along the direction of `offset`.
fn along(offset: Vec2, speed: f32) -> (f32, f32) {
    let distance = offset.length();
    if distance < f32::EPSILON {
        return (0.0, 0.0);
    }
    (offset.y / distance * speed, offset.x / distance * speed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn panel_pull_is_capped_at_full_travel() {
        let mut pull = PanelPull::default();
        assert!(close(pull.moved(PANEL_TRAVEL / 2.0), 0.5));
        assert!(close(pull.moved(PANEL_TRAVEL * 3.0), 1.0));
    }

    #[test]
    fn diagonal_pulls_split_between_axes() {
        let (pitch, yaw) = panel_speed(vec2(36.0, -36.0), 1.0, SpeedMode::Coarse, 5);
        assert!(close(pitch, -yaw) && close(yaw, 30.0 / 2f32.sqrt()));
    }

    #[test]
    fn level_and_fine_scale_the_top_speed() {
        assert!(close(panel_top_speed(SpeedMode::Coarse, 5), 30.0));
        assert!(close(panel_top_speed(SpeedMode::Coarse, 3), 20.0));
        assert!(close(panel_top_speed(SpeedMode::Coarse, 1), 10.0));
        assert!(close(panel_top_speed(SpeedMode::Fine, 5), 10.0));
    }

    #[test]
    fn preview_reaches_full_speed_at_full_pull() {
        let (pitch, yaw) = preview_speed(vec2(23.0, 0.0), false);
        assert!(close(pitch, 0.0) && close(yaw, 17.25));
        let (pitch, yaw) = preview_speed(vec2(0.0, -120.0), false);
        assert!(close(pitch, -30.0) && close(yaw, 0.0));
    }

    #[test]
    fn preview_fine_is_one_third_even_beyond_full_pull() {
        for offset in [Vec2::ZERO, vec2(12.0, -16.0), vec2(300.0, -400.0)] {
            let (pitch, yaw) = preview_speed(offset, false);
            let (fine_pitch, fine_yaw) = preview_speed(offset, true);
            assert!(close(fine_pitch * 3.0, pitch));
            assert!(close(fine_yaw * 3.0, yaw));
        }
    }

    #[test]
    fn centre_stops() {
        assert_eq!(
            panel_speed(Vec2::ZERO, 0.0, SpeedMode::Coarse, 5),
            (0.0, 0.0)
        );
    }
}
