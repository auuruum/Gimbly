//! The OBSBOT models Gimbly drives, and the features checked on each one.
//!
//! To add a model, follow `docs/adding-a-camera.md`. The messages are
//! described in `docs/protocol.md`.

use super::GimbalCommand;

/// One OBSBOT camera model.
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    /// USB product ID; the vendor ID is always [`super::OBSBOT_USB_VENDOR_ID`].
    pub product_id: u16,
    pub name: &'static str,
    /// Features checked on this model with real hardware and recorded in
    /// `docs/devices.md`. The app offers only these.
    pub features: &'static [Feature],
    /// The camera keeps sending the standby picture that
    /// `camera/standby.rs` recognizes for a moment after it wakes.
    pub standby_picture: bool,
}

pub const MODELS: &[Model] = &[Model {
    product_id: 0xff04,
    name: "Tiny 3 Lite",
    features: &[
        Feature::Gimbal,
        Feature::Zoom,
        Feature::FieldOfView,
        Feature::Presets,
        Feature::Sleep,
        Feature::HumanTracking,
        Feature::AutoZoom,
        Feature::TrackingSpeed,
        Feature::SoundSource,
        Feature::ObjectTracking,
        Feature::Composition,
        Feature::VoiceTracking,
        Feature::HandTracking,
        Feature::ZoneTracking,
        Feature::Hdr,
        Feature::Focus,
        Feature::Exposure,
        Feature::WhiteBalance,
        Feature::Image,
    ],
    standby_picture: true,
}];

impl Model {
    pub fn find(product_id: u16) -> Option<&'static Model> {
        MODELS.iter().find(|model| model.product_id == product_id)
    }

    pub fn supports(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }
}

/// A group of vendor commands that is checked and enabled per model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// Joystick, preview drag, Reset and turning to an angle.
    Gimbal,
    /// Vendor zoom ratio (the zoom slider and box).
    Zoom,
    /// Wide, Medium and Narrow.
    FieldOfView,
    /// Preset list, add, update, rename, delete and recall.
    Presets,
    /// Sleep and wake.
    Sleep,
    /// Human Tracking, Single and Group Mode.
    HumanTracking,
    /// Auto Zoom framing for Human Tracking.
    AutoZoom,
    /// Tracking Speed: Slow, Standard, Fast.
    TrackingSpeed,
    /// Sound Source Assisted Tracking.
    SoundSource,
    /// Object tracking from the preview, double-click target and framing.
    ObjectTracking,
    /// AI Composition lines and Face Framing.
    Composition,
    /// Voice Tracking and its range.
    VoiceTracking,
    /// Hand Tracking, its hand and its range panel.
    HandTracking,
    /// Zone Tracking: its switch, range, auto-tracking and Non-tracking Areas.
    ZoneTracking,
    /// The HDR switch on the Image tab.
    Hdr,
    /// The Focus section on the Image tab: Auto Focus, its mode, manual focus.
    Focus,
    /// The Exposure section and Anti-Flicker on the Image tab.
    Exposure,
    /// The White Balance section on the Image tab.
    WhiteBalance,
    /// The Image section (contrast, saturation, sharpness, hue) on the Image tab.
    Image,
}

impl Feature {
    pub const ALL: [Feature; 19] = [
        Self::Gimbal,
        Self::Zoom,
        Self::FieldOfView,
        Self::Presets,
        Self::Sleep,
        Self::HumanTracking,
        Self::AutoZoom,
        Self::TrackingSpeed,
        Self::SoundSource,
        Self::ObjectTracking,
        Self::Composition,
        Self::VoiceTracking,
        Self::HandTracking,
        Self::ZoneTracking,
        Self::Hdr,
        Self::Focus,
        Self::Exposure,
        Self::WhiteBalance,
        Self::Image,
    ];

    /// The name the UI and `docs/devices.md` use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gimbal => "Gimbal control",
            Self::Zoom => "Zoom",
            Self::FieldOfView => "Field of view",
            Self::Presets => "Presets",
            Self::Sleep => "Sleep/Resume",
            Self::HumanTracking => "Human Tracking",
            Self::AutoZoom => "Auto Zoom",
            Self::TrackingSpeed => "Tracking Speed",
            Self::SoundSource => "Sound Source Assisted Tracking",
            Self::ObjectTracking => "Object Tracking",
            Self::Composition => "AI Composition",
            Self::VoiceTracking => "Voice Tracking",
            Self::HandTracking => "Hand Tracking",
            Self::ZoneTracking => "Zone Tracking",
            Self::Hdr => "HDR",
            Self::Focus => "Focus",
            Self::Exposure => "Exposure",
            Self::WhiteBalance => "White Balance",
            Self::Image => "Image",
        }
    }
}

impl Model {
    /// Whether the app may send `command` to this model.
    pub fn allows(&self, command: &GimbalCommand) -> bool {
        command.features().iter().any(|&f| self.supports(f))
    }
}

impl GimbalCommand {
    /// The features this command belongs to; it is sent when the model has
    /// any of them. A new command must be placed here, which keeps it off
    /// models where its feature is not checked.
    pub fn features(&self) -> &'static [Feature] {
        use super::AiMode;
        use GimbalCommand::*;
        match self {
            Drag(_) | Speed { .. } | Reset | MoveTo { .. } => &[Feature::Gimbal],
            ZoomRatio(_) => &[Feature::Zoom],
            SetFov(_) => &[Feature::FieldOfView],
            TriggerPreset(_)
            | AddPreset
            | UpdatePreset { .. }
            | DeletePreset(_)
            | RenamePreset { .. } => &[Feature::Presets],
            SetHumanTracking(_) => &[Feature::HumanTracking],
            SetAutoZoom(_) => &[Feature::AutoZoom],
            SetTrackingSpeed(_) => &[Feature::TrackingSpeed],
            SetSoundSource(_) => &[Feature::SoundSource],
            SetHdr(_) => &[Feature::Hdr],
            SetAutoFocus(_) | SetFocusMode(_) | SetFocusPosition(_) => &[Feature::Focus],
            SetAutoExposure(_) | SetExposureMode(_) | SetShutter(_) | SetIso(_)
            | SetCompensation(_) | SetIsoLimits(..) | SetExposureCurve(_) | SetAntiFlicker(_) => {
                &[Feature::Exposure]
            }
            SetWhiteBalance(_) => &[Feature::WhiteBalance],
            SetImage(_) => &[Feature::Image],
            SelectPoint { .. } | SelectObject(_) | CancelTarget | SetObjectFraming(_) => {
                &[Feature::ObjectTracking]
            }
            ReadComposition | SetComposition { .. } | SetFaceFraming(_) => &[Feature::Composition],
            SetVoiceRange(_) | SetAiMode(AiMode::Voice) => &[Feature::VoiceTracking],
            // Off stops whichever AI mode runs.
            SetAiMode(AiMode::Off) => &[Feature::VoiceTracking, Feature::HandTracking],
            SetAiMode(AiMode::Human) => &[Feature::HumanTracking],
            SetAiMode(AiMode::Hand)
            | ResetWithZoom
            | SetHand(_)
            | OpenHandRange
            | UpdateHandRange(_)
            | ResetHandRange { .. }
            | ConfirmHandRange(_)
            | CloseHandRange { .. } => &[Feature::HandTracking],
            SetZoneTracking(_)
            | OpenZone
            | UpdateZone(_)
            | ResetZone { .. }
            | ConfirmZone(_)
            | SetZoneAutoTarget(_)
            | AddArea
            | UpdateArea(_)
            | RenameArea { .. }
            | DeleteArea(_)
            | CloseZone { .. } => &[Feature::ZoneTracking],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_unique_and_list_each_feature_once() {
        for (i, model) in MODELS.iter().enumerate() {
            assert!(
                MODELS[..i].iter().all(|m| m.product_id != model.product_id),
                "{} repeats a product ID",
                model.name
            );
            for (j, feature) in model.features.iter().enumerate() {
                assert!(
                    !model.features[..j].contains(feature),
                    "{} lists {feature:?} twice",
                    model.name
                );
            }
        }
    }

    #[test]
    fn every_model_has_test_results_in_devices_md() {
        // Only cameras with recorded hardware results may be listed.
        let devices = include_str!("../../../../docs/devices.md");
        for model in MODELS {
            assert!(
                devices.contains(&format!(
                    "{:04x}:{:04x}",
                    super::super::OBSBOT_USB_VENDOR_ID,
                    model.product_id
                )),
                "{} has no row in docs/devices.md",
                model.name
            );
        }
    }

    #[test]
    fn commands_reach_only_models_with_their_feature() {
        use super::super::AiMode;
        let voice_only = Model {
            product_id: 0,
            name: "test",
            features: &[Feature::VoiceTracking],
            standby_picture: false,
        };
        assert!(voice_only.allows(&GimbalCommand::SetAiMode(AiMode::Voice)));
        assert!(voice_only.allows(&GimbalCommand::SetAiMode(AiMode::Off)));
        assert!(!voice_only.allows(&GimbalCommand::SetAiMode(AiMode::Hand)));
        assert!(!voice_only.allows(&GimbalCommand::Reset));
        assert!(!voice_only.allows(&GimbalCommand::AddPreset));
    }

    #[test]
    fn feature_names_are_distinct() {
        for (i, feature) in Feature::ALL.iter().enumerate() {
            assert!(Feature::ALL[..i].iter().all(|f| f.name() != feature.name()));
        }
    }
}
