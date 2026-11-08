//! Colours, fonts and egui style for the control panel.
//!
//! The palette follows the dark layout of OBSBOT Center's control panel
//! (sampled from a screenshot); all drawing code is Gimbly's own.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

pub const WINDOW: Color32 = Color32::from_rgb(0x37, 0x3A, 0x3C);
pub const WINDOW_EDGE: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const CARD: Color32 = Color32::from_rgb(0x22, 0x24, 0x26);
pub const SECTION: Color32 = Color32::from_rgb(0x19, 0x1A, 0x1B);
pub const DIVIDER: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
pub const SEPARATOR_DARK: Color32 = Color32::from_rgb(0x1C, 0x1D, 0x1E);
pub const SEPARATOR_LIGHT: Color32 = Color32::from_rgb(0x44, 0x48, 0x4A);
pub const BUTTON: Color32 = Color32::from_rgb(0x37, 0x3A, 0x3C);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x43, 0x46, 0x49);
pub const SLOT: Color32 = Color32::from_rgb(0x36, 0x3A, 0x3B);
pub const PIP_BUTTON: Color32 = Color32::from_rgb(0x55, 0x55, 0x5A);
pub const FIELD: Color32 = Color32::from_rgb(0x19, 0x1A, 0x1B);
pub const FIELD_STROKE: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x40);
pub const ACCENT: Color32 = Color32::from_rgb(0xFB, 0x00, 0x36);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xFF, 0x26, 0x56);
pub const TOGGLE_OFF: Color32 = Color32::from_rgb(0x55, 0x58, 0x5B);
pub const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0x9C, 0x9E);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6A, 0x6D, 0x70);
/// Dark veil over the controls while the camera sleeps.
pub const SLEEP_SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 0xD0);
pub const SLEEP_BUTTON: Color32 = Color32::from_rgb(0x4A, 0x4C, 0x4F);
pub const SLEEP_BUTTON_HOVER: Color32 = Color32::from_rgb(0x58, 0x5B, 0x5E);

// Software Settings window.
pub const NAV: Color32 = Color32::from_rgb(0x1F, 0x1E, 0x1F);
pub const NAV_SELECTED: Color32 = Color32::from_rgb(0x12, 0x13, 0x13);
pub const RULE: Color32 = Color32::from_rgb(0x35, 0x35, 0x36);
pub const KEY_FIELD: Color32 = Color32::from_rgb(0x0E, 0x0F, 0x10);
pub const SMALL_BUTTON: Color32 = Color32::from_rgb(0x2E, 0x30, 0x32);
pub const SMALL_BUTTON_STROKE: Color32 = Color32::from_rgb(0x4A, 0x4D, 0x50);
pub const CARET_BOX: Color32 = Color32::from_rgb(0x37, 0x3A, 0x3D);
pub const LINK: Color32 = Color32::from_rgb(0x3A, 0xA6, 0xE8);
pub const TABLE: Color32 = Color32::from_rgb(0x18, 0x19, 0x1A);
pub const TABLE_ROW: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x2F);

pub const PANEL_WIDTH: f32 = 360.0;
pub const PADDING: f32 = 20.0;

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = system_ui_font() {
        fonts
            .font_data
            .insert("system-ui".into(), FontData::from_owned(bytes).into());
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "system-ui".into());
        }
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles.insert(TextStyle::Body, font(14.0));
        style.text_styles.insert(TextStyle::Button, font(14.0));
        style.text_styles.insert(TextStyle::Small, font(12.0));
        style.text_styles.insert(TextStyle::Heading, font(16.0));

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = WINDOW;
        v.window_fill = CARD;
        v.extreme_bg_color = FIELD;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke.color = TEXT;
        v.widgets.inactive.bg_fill = BUTTON;
        v.widgets.inactive.weak_bg_fill = BUTTON;
        v.widgets.hovered.bg_fill = BUTTON_HOVER;
        v.widgets.hovered.weak_bg_fill = BUTTON_HOVER;
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.inactive.fg_stroke.color = TEXT;
        v.widgets.hovered.fg_stroke.color = TEXT;
        v.widgets.active.fg_stroke.color = TEXT;
        v.widgets.noninteractive.fg_stroke.color = TEXT;

        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.bar_inner_margin = 0.0;
        style.spacing.scroll.bar_outer_margin = 0.0;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    });
}

/// The platform's UI font, read at runtime from the OS (nothing is bundled).
#[cfg(windows)]
fn system_ui_font() -> Option<Vec<u8>> {
    let windir = std::env::var_os("WINDIR")?;
    std::fs::read(
        std::path::Path::new(&windir)
            .join("Fonts")
            .join("segoeui.ttf"),
    )
    .ok()
}

#[cfg(not(windows))]
fn system_ui_font() -> Option<Vec<u8>> {
    None
}
